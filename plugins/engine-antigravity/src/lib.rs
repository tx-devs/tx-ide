//! The Antigravity engine for tx-core: `agy [--conversation ID] [--model M] [--effort E]
//! [--agent tx] --dangerously-skip-permissions --settings '{"trustedWorkspaces":["<cwd>"]}'
//! [-i prompt]` (agy 1.5.16), with the per-launch files written into the worktree:
//!
//! - [`AGENT_FILE`]: a custom agent carrying the profile text as its system prompt, the full
//!   default tool set (an agent without `tools` loses the shell and the editing tools) and the
//!   definition's MCP servers. Its body replaces Antigravity's persona line; the default prompt
//!   sections stay. `inheritMcp: true` is the only setting under which agy exposes an agent's own
//!   servers, and it brings the operator's global ones along. Written only when there is a
//!   profile text or a server: otherwise the default agent runs.
//! - [`PLUGIN_HOOKS`]: tx's hooks in a workspace plugin, so a repository's own
//!   `.agents/hooks.json` is left alone. Each is `(<hook command> <event>) || true`: agy refuses
//!   a tool whose `PreToolUse` hook fails, and a tx hook must never block the agent.
//! - one link per granted skill under [`SKILLS_DIR`].
//!
//! Trust comes per launch through `--settings` and is never written to agy's settings.
//!
//! Engine data, under `ext.antigravity` of the record, is this adapter's alone to read:
//!
//! ```json
//! {"conversation_id": "c64c…", "transcript_path": "/Users/w/.gemini/antigravity-cli/brain/c64c…/.system_generated/logs/transcript_full.jsonl",
//!  "model": "gemini-4-neon", "effort": "high"}
//! ```
//!
//! agy picks the conversation id itself: every hook payload carries it (`conversationId`,
//! `transcriptPath`), so the record learns it from the first one. agy fires nothing until the
//! first prompt (`SessionStart` comes with it, a resume fires none), so a resume is wrapped in a
//! shell that fires a synthetic [`READY_EVENT`] a second after launch; it takes the agent out of
//! `Starting` and changes nothing later.
//!
//! A fork copies the source's conversation database under the new session's id before agy
//! starts: `sqlite3 <src>.db ".backup <new>.db"`, then the new ids into `trajectory_meta`, as
//! agy's own `/fork` writes them. The copy runs in the launch's shell, since `fork` is pure.
//!
//! `archive` keeps the transcript (`<archive>/<id>.jsonl`, by offset) and, once the process is
//! gone, the conversation database (`<archive>/<id>.db`, with its `-wal` when one is left);
//! `restore` puts back what is missing before a resume.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tx_core::engine::{AgentSummary, DefinitionsRequest, EngineCheck, Finding};
use tx_core::harness::{McpServer, bearer_variable, variable_name};
use tx_core::{
    Agent, ENGINE_ARGS_OPTION, EngineError, EngineKind, IdleReason, Input, Key, LaunchPlan,
    LaunchRequest, LlmEngine, LlmEngineChange, LlmEngineHook, LlmEngineState, WorkspaceAction,
    engine_data, hook_shell_command, mirror_file, mirror_file_exact,
};

pub const AGY_BIN: &str = "agy";
pub const SQLITE_BIN: &str = "sqlite3";
/// The kind this engine provides.
pub const KIND: EngineKind = EngineKind::ANTIGRAVITY;
/// The `ext` key this engine owns.
pub const EXT_KEY: &str = "antigravity";
pub const CONVERSATION_ID_KEY: &str = "conversation_id";
pub const TRANSCRIPT_PATH_KEY: &str = "transcript_path";
pub const MODEL_KEY: &str = "model";
pub const EFFORT_KEY: &str = "effort";
pub const COMMAND_KEY: &str = "command";

pub const CONVERSATION_FLAG: &str = "--conversation";
pub const MODEL_FLAG: &str = "--model";
pub const EFFORT_FLAG: &str = "--effort";
pub const AGENT_FLAG: &str = "--agent";
pub const SETTINGS_FLAG: &str = "--settings";
pub const PROMPT_FLAG: &str = "-i";
/// Every launch runs every tool without asking.
pub const SKIP_PERMISSIONS_FLAG: &str = "--dangerously-skip-permissions";

pub const AGENT_NAME: &str = "tx";
pub const AGENT_FILE: &str = ".agents/agents/tx.md";
pub const PLUGIN_MANIFEST: &str = ".agents/plugins/tx/plugin.json";
pub const PLUGIN_HOOKS: &str = ".agents/plugins/tx/hooks.json";
pub const PLUGIN_NAME: &str = "tx";
pub const SKILLS_DIR: &str = ".agents/skills";

/// The tool set of agy's default agent (1.5.16), less `swarm`, which a built-in plugin adds
/// to the default agent only. A custom agent gets only the tools it lists,
/// and without `tools` a small read-only set; naming a tool agy lacks fails the session. The
/// MCP and resource tools come on their own.
pub const DEFAULT_TOOLS: [&str; 14] = [
    "read_file",
    "write_file",
    "edit_file",
    "run_command",
    "manage_task",
    "schedule",
    "search_web",
    "update_steps",
    "ask_question",
    "define_subagent",
    "invoke_subagent",
    "manage_subagents",
    "send_message",
    "run_workflow",
];

/// The hooks wired per launch, each running `<hook command> <event>`. `SessionStart`,
/// `PreInvocation`, `PostInvocation` and `Stop` take their handlers directly; the tool events
/// take them under a matcher.
pub const HOOK_EVENTS: [&str; 6] = [
    "SessionStart",
    "PreInvocation",
    "PreToolUse",
    "PostToolUse",
    "PostInvocation",
    "Stop",
];
const TOOL_EVENTS: [&str; 2] = ["PreToolUse", "PostToolUse"];
/// Not agy's: what the resume wrapper fires once the TUI is starting.
pub const READY_EVENT: &str = "Ready";
/// The tool through which the model asks the operator a question.
pub const QUESTION_TOOL: &str = "ask_question";
/// How long the resume wrapper waits before [`READY_EVENT`]: core saves the record right after
/// tmux starts the process, and a hook for a record not yet saved is dropped.
pub const READY_DELAY_SECONDS: u32 = 1;

/// What `tx harness ls` prints for every agent of this engine; the manifest says the same.
pub const NOTES: [&str; 2] = [
    "antigravity: name a base model (gemini-4-neon) and let the effort pick its tier; a model id \
     with a tier (gemini-4-neon-low) and a different effort make agy fall back to high, and max \
     is high on gemini 4 neon",
    "antigravity fork copies the conversation with the sqlite3 CLI, which must be on PATH",
];

/// What every launch carries that no tx option removes (agy 1.5.16).
pub const FORCED: [&str; 8] = [
    "Antigravity's default prompt sections and tools: the profile replaces only its persona line, \
     and every agent gets agy's full default tool set",
    "the user's ~/.gemini/config MCP servers, hooks, skills and plugins, and agy's built-in \
     plugins: an agent's own MCP servers are exposed only alongside the global ones",
    "the user's ~/.gemini/GEMINI.md and the cross-tool skills of ~/.agents/skills",
    "the workspace's AGENTS.md, GEMINI.md, .agents/rules and .agents/hooks.json",
    "MCP tools reached lazily through call_mcp_tool after reading their schema files: the model \
     may say a server is missing until told to try it",
    "agy's own ${VAR} handling: none. A stdio server's env passes a variable only under its own \
     name, and a header naming one is refused",
    "a command the model moves to the background ends its turn; the agent works again on its own \
     when the command finishes",
    "after tx interrupt, an empty input: the interrupted prompt is not put back",
];

/// agy's directory, `$HOME/.gemini/antigravity-cli`: conversation databases under
/// `conversations/`, transcripts and artifacts under `brain/<id>/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AntigravityAdapter {
    pub antigravity_home: PathBuf,
}

/// The launch options `configure` accepts. Every key is optional; an unknown key is refused.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AntigravityOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<Effort>,
    /// The program started in place of [`AGY_BIN`], given tx's whole argument list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Appended after tx's own flags, before the prompt: `tx create … --engine-args ARGS`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
}

/// agy's `--effort` levels. A model offers a subset: Gemini 4 Neon has no `max`, and agy then
/// warns and uses High.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", try_from = "String")]
pub enum Effort {
    Low,
    Medium,
    High,
    XHigh,
    Max,
}

impl Effort {
    pub const ALL: [Effort; 5] = [
        Effort::Low,
        Effort::Medium,
        Effort::High,
        Effort::XHigh,
        Effort::Max,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Effort::Low => "low",
            Effort::Medium => "medium",
            Effort::High => "high",
            Effort::XHigh => "xhigh",
            Effort::Max => "max",
        }
    }
}

impl FromStr for Effort {
    type Err = String;

    /// A word, or a level `1`–`5` on the tx scale (`tx_core::effort_word`).
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let word = tx_core::effort_word(text).unwrap_or(text);
        Self::ALL
            .into_iter()
            .find(|effort| effort.as_str() == word)
            .ok_or_else(|| {
                format!(
                    "unknown effort {text:?} (antigravity expects 1-5 or low, medium, high, xhigh or max)"
                )
            })
    }
}

impl TryFrom<String> for Effort {
    type Error = String;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        text.parse()
    }
}

/// `{"tx":{"<event>":[…]}}`: every event of [`HOOK_EVENTS`] runs `(<hook command> <event>) ||
/// true`, the tool events under a `*` matcher.
pub fn hooks_file(hook_command: &[String]) -> String {
    let events: serde_json::Map<String, Value> = HOOK_EVENTS
        .iter()
        .map(|event| {
            let handler = serde_json::json!([{
                "type": "command",
                "command": format!("({}) || true", hook_shell_command(hook_command, event)),
            }]);
            let handlers = if TOOL_EVENTS.contains(event) {
                serde_json::json!([{ "matcher": "*", "hooks": handler }])
            } else {
                handler
            };
            ((*event).to_owned(), handlers)
        })
        .collect();
    serde_json::json!({ PLUGIN_NAME: events }).to_string()
}

/// One entry of the agent's `mcpServers` list. agy expands no `${VAR}`, and a stdio server
/// inherits agy's environment: an `env` entry that passes a variable under its own name is
/// dropped, a literal one kept, and anything else naming a variable is refused, since resolving
/// it would write the secret into the worktree.
pub fn mcp_entry(name: &str, server: &McpServer) -> Result<Value, EngineError> {
    let refuse = |what: String| {
        EngineError::Refused(format!(
            "antigravity: MCP server {name}: {what}: agy expands no variables, and tx does not \
             write secrets into the worktree"
        ))
    };
    match server {
        McpServer::Stdio { command, args, env } => {
            let mut literal = serde_json::Map::new();
            for (variable, value) in env {
                match variable_name(value) {
                    Some(source) if source == variable => {}
                    Some(source) => {
                        return Err(refuse(format!("env {variable}=${{{source}}}")));
                    }
                    None => {
                        literal.insert(variable.clone(), value.clone().into());
                    }
                }
            }
            let mut entry = serde_json::json!({ "name": name, "command": command });
            if !args.is_empty() {
                entry["args"] = serde_json::json!(args);
            }
            if !literal.is_empty() {
                entry["env"] = Value::Object(literal);
            }
            Ok(entry)
        }
        McpServer::Http { url, headers } => {
            for (header, value) in headers {
                if variable_name(value)
                    .or_else(|| bearer_variable(value))
                    .is_some()
                {
                    return Err(refuse(format!("header {header}")));
                }
            }
            let mut entry = serde_json::json!({ "name": name, "url": url });
            if !headers.is_empty() {
                entry["headers"] = serde_json::json!(headers);
            }
            Ok(entry)
        }
    }
}

/// The custom agent: YAML frontmatter (each value a JSON flow scalar, which YAML reads as is)
/// and the profile text as its system prompt.
pub fn agent_file(text: &str, mcp_servers: &[Value]) -> String {
    let mut frontmatter = vec![
        format!("name: {}", Value::from(AGENT_NAME)),
        format!("description: {}", Value::from("tx agent")),
        "mainAgent: true".to_owned(),
        "subagent: false".to_owned(),
        format!("tools: {}", serde_json::json!(DEFAULT_TOOLS)),
    ];
    if !mcp_servers.is_empty() {
        frontmatter.push("inheritMcp: true".to_owned());
        frontmatter.push(format!("mcpServers: {}", Value::from(mcp_servers.to_vec())));
    }
    format!("---\n{}\n---\n{text}\n", frontmatter.join("\n"))
}

/// One symlink per granted skill dir: `<skills dir>/<skill name>` → the skill dir.
pub fn skill_links(skill_dirs: &[PathBuf]) -> Vec<WorkspaceAction> {
    skill_dirs
        .iter()
        .filter_map(|dir| {
            let name = dir.file_name()?;
            Some(WorkspaceAction::Symlink {
                src: dir.clone(),
                dst: PathBuf::from(SKILLS_DIR).join(name),
            })
        })
        .collect()
}

/// `--settings` for one launch: trust the worktree, without the dialog and without writing it.
pub fn launch_settings(working_directory: &Path) -> String {
    serde_json::json!({ "trustedWorkspaces": [working_directory] }).to_string()
}

impl AntigravityAdapter {
    pub fn new(antigravity_home: impl Into<PathBuf>) -> Self {
        Self {
            antigravity_home: antigravity_home.into(),
        }
    }

    /// `<home>/conversations/<id>.db`: what a resume reads.
    pub fn database_path(&self, conversation: &str) -> PathBuf {
        self.antigravity_home
            .join("conversations")
            .join(format!("{conversation}.db"))
    }

    /// `<home>/brain/<id>/.system_generated/logs/transcript_full.jsonl`, what hooks report as
    /// `transcriptPath`.
    pub fn transcript_path(&self, conversation: &str) -> PathBuf {
        self.antigravity_home
            .join("brain")
            .join(conversation)
            .join(".system_generated/logs/transcript_full.jsonl")
    }

    /// `current` with `conversation` and its transcript path, `None` when both are already
    /// recorded.
    fn with_conversation(
        &self,
        current: &Value,
        conversation: &str,
        transcript: Option<&str>,
    ) -> Option<Value> {
        let transcript = transcript.map_or_else(
            || {
                self.transcript_path(conversation)
                    .to_string_lossy()
                    .into_owned()
            },
            str::to_owned,
        );
        if current.get(CONVERSATION_ID_KEY).and_then(Value::as_str) == Some(conversation)
            && current.get(TRANSCRIPT_PATH_KEY).and_then(Value::as_str) == Some(&transcript)
        {
            return None;
        }
        let mut data = current.as_object().cloned().unwrap_or_default();
        data.insert(CONVERSATION_ID_KEY.into(), conversation.into());
        data.insert(TRANSCRIPT_PATH_KEY.into(), transcript.into());
        Some(Value::Object(data))
    }

    /// The command line every launch shares after the `args` that name the conversation, and
    /// the program that runs it (`agy`, or the `command` option).
    fn command_line(
        &self,
        request: &LaunchRequest<'_>,
        mut args: Vec<String>,
        prompt: &str,
    ) -> Result<(String, Vec<String>, Vec<WorkspaceAction>), EngineError> {
        if let Some((name, provider)) = request.profile.provided_skills().first() {
            return Err(EngineError::Refused(format!(
                "antigravity: skill {name:?} is provided by {} and does not exist on antigravity",
                provider.as_str()
            )));
        }
        let mcp_servers = request
            .profile
            .mcp_servers()
            .iter()
            .map(|(name, server)| mcp_entry(name, server))
            .collect::<Result<Vec<_>, _>>()?;
        let data = engine_data(self, request.session);
        let option = |key: &str| data.get(key).and_then(Value::as_str);
        if let Some(model) = option(MODEL_KEY) {
            args.extend([MODEL_FLAG.to_owned(), model.to_owned()]);
        }
        if let Some(effort) = option(EFFORT_KEY) {
            args.extend([EFFORT_FLAG.to_owned(), effort.to_owned()]);
        }
        let mut workspace = Vec::new();
        let text = request.profile.text();
        if !text.is_empty() || !mcp_servers.is_empty() {
            args.extend([AGENT_FLAG.to_owned(), AGENT_NAME.to_owned()]);
            workspace.push(WorkspaceAction::Write {
                path: AGENT_FILE.into(),
                content: agent_file(text, &mcp_servers),
            });
        }
        args.extend([
            SKIP_PERMISSIONS_FLAG.to_owned(),
            SETTINGS_FLAG.to_owned(),
            launch_settings(&request.session.working_directory),
        ]);
        args.extend(
            data.get(ENGINE_ARGS_OPTION)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned),
        );
        if !prompt.is_empty() {
            args.extend([PROMPT_FLAG.to_owned(), prompt.to_owned()]);
        }
        if !request.hook_command.is_empty() {
            workspace.push(WorkspaceAction::Write {
                path: PLUGIN_MANIFEST.into(),
                content: serde_json::json!({ "name": PLUGIN_NAME }).to_string(),
            });
            workspace.push(WorkspaceAction::Write {
                path: PLUGIN_HOOKS.into(),
                content: hooks_file(request.hook_command),
            });
        }
        workspace.extend(skill_links(request.profile.skill_dirs()));
        let program = option(COMMAND_KEY).unwrap_or(AGY_BIN).to_owned();
        Ok((program, args, workspace))
    }

    /// The conversation recorded for `session` and its transcript path.
    fn recorded_conversation<'a>(&self, session: &'a Agent) -> Option<(&'a str, &'a Path)> {
        let data = engine_data(self, session);
        Some((
            data.get(CONVERSATION_ID_KEY)?.as_str()?,
            Path::new(data.get(TRANSCRIPT_PATH_KEY)?.as_str()?),
        ))
    }
}

/// `program args…` run by `sh -c script sh <arguments> program args…`: the script ends in
/// `exec "$@"` after shifting its own arguments away.
fn wrapped(
    script: String,
    arguments: Vec<String>,
    program: String,
    args: Vec<String>,
) -> LaunchPlan {
    let mut wrapper_args = vec!["-c".to_owned(), script, "sh".to_owned()];
    wrapper_args.extend(arguments);
    wrapper_args.push(program);
    wrapper_args.extend(args);
    LaunchPlan::new("sh").args(wrapper_args)
}

fn plan_with(workspace: Vec<WorkspaceAction>, mut plan: LaunchPlan) -> LaunchPlan {
    for action in workspace {
        plan = plan.workspace(action);
    }
    plan
}

impl LlmEngine for AntigravityAdapter {
    fn kind(&self) -> EngineKind {
        KIND
    }

    fn ext_key(&self) -> &'static str {
        EXT_KEY
    }

    fn configure(&self, session: &Agent, options: &Value) -> Result<Value, EngineError> {
        if !options.is_object() {
            return Err(EngineError::Refused(format!(
                "antigravity options: expected a JSON object, got {options}"
            )));
        }
        let options: AntigravityOptions = serde_json::from_value(options.clone())
            .map_err(|error| EngineError::Refused(format!("antigravity options: {error}")))?;
        for (key, value) in [(MODEL_KEY, &options.model), (COMMAND_KEY, &options.command)] {
            if value.as_deref() == Some("") {
                return Err(EngineError::Refused(format!(
                    "antigravity options: {key} must not be empty"
                )));
            }
        }
        let mut data = engine_data(self, session)
            .as_object()
            .cloned()
            .unwrap_or_default();
        if let Some(model) = options.model {
            data.insert(MODEL_KEY.into(), model.into());
        }
        if let Some(effort) = options.effort {
            data.insert(EFFORT_KEY.into(), effort.as_str().into());
        }
        if let Some(command) = options.command {
            data.insert(COMMAND_KEY.into(), command.into());
        }
        if let Some(args) = options.args {
            data.insert(ENGINE_ARGS_OPTION.into(), args.into());
        }
        Ok(Value::Object(data))
    }

    /// agy names the conversation; the first hook (`SessionStart`, with the prompt) records it.
    fn start(&self, request: &LaunchRequest<'_>, prompt: &str) -> Result<LaunchPlan, EngineError> {
        let (program, args, workspace) = self.command_line(request, Vec::new(), prompt)?;
        Ok(plan_with(workspace, LaunchPlan::new(program).args(args)))
    }

    /// `--conversation <id>`, wrapped to fire [`READY_EVENT`]: a resume fires no hook until the
    /// operator types.
    fn resume(&self, request: &LaunchRequest<'_>) -> Result<LaunchPlan, EngineError> {
        let Some((conversation, _)) = self.recorded_conversation(request.session) else {
            return Err(EngineError::Refused(format!(
                "antigravity cannot resume {}: no conversation recorded (ext.{EXT_KEY}.{CONVERSATION_ID_KEY})",
                request.session.name
            )));
        };
        let args = vec![CONVERSATION_FLAG.to_owned(), conversation.to_owned()];
        let (program, args, workspace) = self.command_line(request, args, "")?;
        if request.hook_command.is_empty() {
            return Ok(plan_with(workspace, LaunchPlan::new(program).args(args)));
        }
        let script = format!(
            "(sleep {READY_DELAY_SECONDS}; {}) </dev/null >/dev/null 2>&1 & exec \"$@\"",
            hook_shell_command(request.hook_command, READY_EVENT)
        );
        Ok(plan_with(
            workspace,
            wrapped(script, Vec::new(), program, args),
        ))
    }

    /// A copy of the source's conversation database under the new session's id, made by the
    /// launch's shell, then `--conversation <id> -i <prompt>` on it. The plan records the new
    /// conversation up front.
    fn fork(
        &self,
        request: &LaunchRequest<'_>,
        source: &Agent,
        prompt: &str,
    ) -> Result<LaunchPlan, EngineError> {
        let Some((source_conversation, _)) = self.recorded_conversation(source) else {
            return Err(EngineError::Refused(format!(
                "antigravity cannot fork {}: no conversation recorded (ext.{EXT_KEY}.{CONVERSATION_ID_KEY})",
                source.name
            )));
        };
        let conversation = request.session.id.to_string();
        let trajectory = uuid::Uuid::new_v4().to_string();
        let args = vec![CONVERSATION_FLAG.to_owned(), conversation.clone()];
        let (program, args, workspace) = self.command_line(request, args, prompt)?;
        let script = format!(
            "test -f \"$1\" && {SQLITE_BIN} \"$1\" \".backup '$2'\" && {SQLITE_BIN} \"$2\" \
             \"UPDATE trajectory_meta SET cascade_id='$3', trajectory_id='$4'\" && shift 4 && \
             exec \"$@\""
        );
        let arguments = vec![
            self.database_path(source_conversation)
                .to_string_lossy()
                .into_owned(),
            self.database_path(&conversation)
                .to_string_lossy()
                .into_owned(),
            conversation.clone(),
            trajectory,
        ];
        let data = self
            .with_conversation(engine_data(self, request.session), &conversation, None)
            .unwrap_or_else(|| engine_data(self, request.session).clone());
        let mut plan = plan_with(workspace, wrapped(script, arguments, program, args));
        plan.engine_data = Some(data);
        Ok(plan)
    }

    /// Escape, as the status line says ("esc to cancel"); agy fires no hook for it and lands at
    /// an empty prompt.
    fn interrupt(&self, _: &Agent) -> Option<(Input, Option<LlmEngineState>)> {
        Some((Input::Key(Key::Escape), Some(LlmEngineState::Idle)))
    }

    /// A turn is `PreInvocation → [PreToolUse → PostToolUse]… → PostInvocation → … → Stop`;
    /// `SessionStart` fires with the first prompt, right before its `PreInvocation`, so it says
    /// nothing of the state. The model's question (`PreToolUse` of [`QUESTION_TOOL`]) waits on
    /// the operator until its `PostToolUse`. [`READY_EVENT`] only ends `Starting`. Every payload
    /// names the conversation, which is recorded when it changes (a resume, `/fork` or
    /// `/resume` inside the TUI).
    fn on_hook(&self, session: &Agent, hook: &LlmEngineHook) -> LlmEngineChange {
        let change = match hook.name.as_str() {
            "PreInvocation" | "PostInvocation" | "PostToolUse" => {
                LlmEngineChange::to(LlmEngineState::Working)
            }
            "PreToolUse" => {
                let tool = hook
                    .payload
                    .pointer("/toolCall/name")
                    .and_then(Value::as_str);
                if tool == Some(QUESTION_TOOL) {
                    LlmEngineChange::idle(IdleReason::Waiting)
                } else {
                    LlmEngineChange::to(LlmEngineState::Working)
                }
            }
            "Stop" => LlmEngineChange::idle(IdleReason::Finished),
            READY_EVENT if session.state == LlmEngineState::Starting => {
                LlmEngineChange::idle(IdleReason::Finished)
            }
            _ => LlmEngineChange::NONE,
        };
        let engine_data = hook
            .payload
            .get("conversationId")
            .and_then(Value::as_str)
            .and_then(|conversation| {
                self.with_conversation(
                    engine_data(self, session),
                    conversation,
                    hook.payload.get("transcriptPath").and_then(Value::as_str),
                )
            });
        LlmEngineChange {
            engine_data,
            ..change
        }
    }

    /// The transcript by offset while the process lives, compared whole once it is gone; the
    /// conversation database (and a `-wal` left beside it) only then.
    fn archive(&self, session: &Agent, archive_directory: &Path) -> Result<(), EngineError> {
        let Some((conversation, transcript)) = self.recorded_conversation(session) else {
            return Ok(());
        };
        let ended = session.state == LlmEngineState::Ended;
        if transcript.is_file() {
            let archived = archive_directory.join(format!("{conversation}.jsonl"));
            if ended {
                mirror_file_exact(transcript, &archived)?;
            } else {
                mirror_file(transcript, &archived)?;
            }
        }
        if !ended {
            return Ok(());
        }
        for (live, archived) in database_files(
            &self.database_path(conversation),
            archive_directory,
            conversation,
        ) {
            if live.is_file() {
                mirror_file_exact(&live, &archived)?;
            }
        }
        Ok(())
    }

    /// The database when it is gone, and the transcript when it is gone, from the archive; a
    /// live conversation is never touched.
    fn restore(&self, session: &Agent, archive_directory: &Path) -> Result<bool, EngineError> {
        let Some((conversation, transcript)) = self.recorded_conversation(session) else {
            return Ok(false);
        };
        let database = self.database_path(conversation);
        let mut restored = false;
        if !database.exists()
            && archive_directory
                .join(format!("{conversation}.db"))
                .is_file()
        {
            for (live, archived) in database_files(&database, archive_directory, conversation) {
                if archived.is_file() {
                    mirror_file_exact(&archived, &live)?;
                }
            }
            restored = true;
        }
        let archived_transcript = archive_directory.join(format!("{conversation}.jsonl"));
        if archived_transcript.is_file() && !transcript.exists() {
            mirror_file_exact(&archived_transcript, transcript)?;
            restored = true;
        }
        Ok(restored)
    }

    fn notes(&self) -> Vec<String> {
        NOTES.iter().map(|note| (*note).to_owned()).collect()
    }

    fn check(&self, request: &DefinitionsRequest<'_>) -> Result<EngineCheck, EngineError> {
        Ok(check_report(request))
    }
}

/// The database and its `-wal`, live path beside archived path.
fn database_files(
    database: &Path,
    archive_directory: &Path,
    conversation: &str,
) -> [(PathBuf, PathBuf); 2] {
    let wal = PathBuf::from(format!("{}-wal", database.to_string_lossy()));
    [
        (
            database.to_path_buf(),
            archive_directory.join(format!("{conversation}.db")),
        ),
        (
            wal,
            archive_directory.join(format!("{conversation}.db-wal")),
        ),
    ]
}

/// What one agent definition gets on Antigravity that its file does not say.
pub fn definition_notes(agent: &AgentSummary) -> Vec<String> {
    let mut notes = Vec::new();
    if !agent.tools.is_empty() || !agent.disallowed_tools.is_empty() {
        notes.push(
            "antigravity grants agy's full default tool set: `tools` and `disallowedTools` change \
             nothing"
                .into(),
        );
    }
    notes
}

/// What `tx harness check --engine antigravity` reports: [`FORCED`]; per agent each skill an
/// engine provides, which Antigravity has not got, at the line of its `skills`; per agent
/// [`definition_notes`].
pub fn check_report(request: &DefinitionsRequest<'_>) -> EngineCheck {
    let mut problems = Vec::new();
    let mut notes = Vec::new();
    for agent in request.agents {
        for name in &agent.skills {
            if let Some(provider) = request.provided_skills.get(name) {
                problems.push(Finding {
                    path: agent.path.clone(),
                    line: Some(agent.lines.get("skills").copied().unwrap_or(1)),
                    message: format!(
                        "skill {name:?} is provided by {}: not on antigravity",
                        provider.as_str()
                    ),
                });
            }
        }
        notes.extend(definition_notes(agent).into_iter().map(|message| Finding {
            path: agent.path.clone(),
            line: None,
            message,
        }));
    }
    EngineCheck {
        always_present: FORCED.iter().map(|item| (*item).to_owned()).collect(),
        problems,
        notes,
    }
}

/// The plugin wrapper: registers [`AntigravityAdapter`] under the plugin id
/// `engine-antigravity`, with agy's directory resolved once from the environment.
#[derive(Clone, Copy, Debug, Default)]
pub struct AntigravityPlugin;

/// `$HOME/.gemini/antigravity-cli`: nothing documented relocates it.
pub fn antigravity_home_from_env() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(".gemini")
        .join("antigravity-cli")
}

impl tx_core::Plugin for AntigravityPlugin {
    fn id(&self) -> &'static str {
        "engine-antigravity"
    }

    fn register(&self, registry: &mut tx_core::Registry) -> Result<(), tx_core::PluginError> {
        registry.engine(Box::new(AntigravityAdapter::new(
            antigravity_home_from_env(),
        )));
        Ok(())
    }
}
