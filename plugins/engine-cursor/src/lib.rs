//! The Cursor engine for tx-core: `cursor-agent [--new-session-id ID | --resume ID]
//! [--model M] [--plugin-dir <cwd>/.cursor/tx-plugin] --force --trust --approve-mcps
//! --disable-auto-update [prompt]` (cursor-agent 2026.11.22), with the per-launch files written
//! into the worktree:
//!
//! - [`HOOKS_FILE`]: tx's hooks, one per [`HOOK_EVENTS`]. They go in the project file, not the
//!   plugin, because the TUI fires `beforeSubmitPrompt` and `stop` only when a user or project
//!   `hooks.json` defines them; a file naming an event Cursor does not know is dropped whole,
//!   silently.
//! - [`PROFILE_RULE`]: the profile text as an always-applied rule. A plugin's rules never reach
//!   the model and `--system-prompt` is refused for ordinary accounts.
//! - [`PLUGIN_DIR`]: a plugin carrying the agent definition's MCP servers as `.mcp.json`, only
//!   when it declares any; `--approve-mcps` keeps them from asking.
//! - one link per granted skill under [`SKILLS_DIR`].
//!
//! Engine data, under `ext.cursor` of the record, is this adapter's alone to read:
//!
//! ```json
//! {"conversation_id": "5f0c…", "transcript_path": "/Users/w/.cursor/projects/Users-w-repo/agent-transcripts/5f0c…/5f0c….jsonl",
//!  "model": "gpt-5.3-codex-high"}
//! ```
//!
//! A start names its chat after the tx session id (`--new-session-id`, hidden but honoured), so
//! the plan carries the conversation up front: a resume fires no `sessionStart` (only
//! `workspaceOpen`, which names none), and the transcript path hooks report stays `null` until
//! the first tool. A `/new` inside the TUI is picked up from the next hook's `conversation_id`.
//!
//! `model`, `command` and `args` are launch options. Cursor has no effort: it is part of the
//! model id (`gpt-5.3-codex-high`), and the CLI refuses the bracket parameters its help
//! advertises (`gpt-5.3-codex[effort=high]`: "Cannot use this model"). `effort` is accepted, so
//! that `agent.effort` in `config.yaml` does not refuse every launch, and dropped ([`NOTES`]).
//!
//! A fork is refused: Cursor has no fork, and a copied chat keeps its source's id inside
//! `store.db`. `archive` keeps the transcript (`<archive>/<id>.jsonl`, by offset) and, once the
//! process is gone and the database checkpointed, the chat directory's files
//! (`<archive>/<id>/`); `restore` puts back what is missing, before a resume, which would
//! otherwise silently start an empty chat under the same id.

use std::path::{Path, PathBuf};

use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tx_core::engine::{AgentSummary, DefinitionsRequest, EngineCheck, Finding};
use tx_core::{
    Agent, ENGINE_ARGS_OPTION, EngineError, EngineKind, IdleReason, Input, Key, LaunchPlan,
    LaunchRequest, LlmEngine, LlmEngineChange, LlmEngineHook, LlmEngineState, WorkspaceAction,
    engine_data, hook_shell_command, mirror_file, mirror_file_exact,
};

pub const CURSOR_BIN: &str = "cursor-agent";
/// The `ext` key this engine owns.
pub const EXT_KEY: &str = "cursor";
pub const CONVERSATION_ID_KEY: &str = "conversation_id";
pub const TRANSCRIPT_PATH_KEY: &str = "transcript_path";
pub const MODEL_KEY: &str = "model";
pub const COMMAND_KEY: &str = "command";

pub const NEW_SESSION_ID_FLAG: &str = "--new-session-id";
pub const RESUME_FLAG: &str = "--resume";
pub const MODEL_FLAG: &str = "--model";
pub const PLUGIN_DIR_FLAG: &str = "--plugin-dir";
/// Every launch: run every tool without asking, trust the worktree, approve the MCP servers,
/// and never swap the CLI's version under a running agent.
pub const LAUNCH_FLAGS: [&str; 4] = [
    "--force",
    "--trust",
    "--approve-mcps",
    "--disable-auto-update",
];

pub const HOOKS_FILE: &str = ".cursor/hooks.json";
pub const PROFILE_RULE: &str = ".cursor/rules/tx-profile.mdc";
pub const PLUGIN_DIR: &str = ".cursor/tx-plugin";
pub const PLUGIN_MANIFEST: &str = ".cursor/tx-plugin/.cursor-plugin/plugin.json";
pub const PLUGIN_MCP_FILE: &str = ".cursor/tx-plugin/.mcp.json";
pub const PLUGIN_NAME: &str = "tx";
pub const SKILLS_DIR: &str = ".cursor/skills";

/// The hooks wired per launch, each running `<hook_command> <event>`. Only names Cursor knows.
pub const HOOK_EVENTS: [&str; 6] = [
    "workspaceOpen",
    "sessionStart",
    "beforeSubmitPrompt",
    "preToolUse",
    "postToolUse",
    "stop",
];

/// What `tx harness ls` prints for every agent of this engine; the manifest says the same.
pub const NOTES: [&str; 1] =
    ["cursor ignores the effort option: pick it with the model id (gpt-5.3-codex-high)"];

/// The models the new-agent form offers, the one `cursor-agent` runs without `--model` first; a
/// few of `cursor-agent --list-models` (2026.11.22), each naming its effort tier. Cursor has no
/// effort, so the form offers none. The manifest says the same.
pub const MODELS: [&str; 7] = [
    "auto",
    "claude-opus-5-5-high",
    "claude-sonnet-5-thinking-high",
    "gpt-5.6-sol-high",
    "gpt-5.3-codex-high",
    "composer-2.5",
    "grok-4.7-high",
];

/// What every launch carries that no tx option removes (cursor-agent 2026.11.22).
pub const FORCED: [&str; 8] = [
    "Cursor's own system prompt and built-in tools: the profile is an always-applied rule beside \
     them, and no built-in tool can be switched off",
    "the workspace's AGENTS.md, CLAUDE.md, .cursorrules and .cursor/rules",
    "the user's ~/.cursor/mcp.json servers and ~/.cursor/hooks.json",
    "the Claude Code hooks of ~/.claude/settings.json and the worktree's .claude/settings.json, \
     given Cursor's payloads: Cursor imports them and has no switch to stop it",
    "the user's Claude Code plugins, which Cursor loads as its own",
    "the AskQuestion tool, whose dialog fires no hook: the agent shows working while it waits",
    "the worktree's .cursor/hooks.json, replaced by tx's on every launch: a project's own hooks \
     there do not run",
    "after tx interrupt, the interrupted prompt put back in the input: the next message is \
     appended to it (clear it with C-u, once per line)",
];

/// Cursor's directory: `CURSOR_CONFIG_DIR`, else `$HOME/.cursor`. Chats live under
/// `<cursor home>/chats/`, transcripts under `<cursor home>/projects/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CursorAdapter {
    pub cursor_home: PathBuf,
}

/// The launch options `configure` accepts. Every key is optional; an unknown key is refused.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CursorOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Accepted and dropped: Cursor's effort is part of the model id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<Value>,
    /// The program started in place of [`CURSOR_BIN`], given tx's whole argument list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Appended after tx's own flags, before the prompt: `tx create … --engine-args ARGS`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
}

/// Cursor's name for a working directory under `projects/`: every run of characters other
/// than ASCII letters and digits becomes one `-`, trimmed at both ends.
pub fn project_slug(working_directory: &Path) -> String {
    let mut slug = String::new();
    for character in working_directory.to_string_lossy().chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_end_matches('-').to_owned()
}

/// `{"version":1,"hooks":{"<event>":[{"command":"<hook command> <event>"}]}}`.
pub fn hooks_file(hook_command: &[String]) -> String {
    let hooks: serde_json::Map<String, Value> = HOOK_EVENTS
        .iter()
        .map(|event| {
            (
                (*event).to_owned(),
                serde_json::json!([{ "command": hook_shell_command(hook_command, event) }]),
            )
        })
        .collect();
    serde_json::json!({ "version": 1, "hooks": hooks }).to_string()
}

/// The profile text as a rule Cursor applies to every request.
pub fn profile_rule(text: &str) -> String {
    format!("---\ndescription: tx profile\nalwaysApply: true\n---\n{text}")
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

impl CursorAdapter {
    pub fn new(cursor_home: impl Into<PathBuf>) -> Self {
        Self {
            cursor_home: cursor_home.into(),
        }
    }

    /// `<cursor home>/projects/<slug>/agent-transcripts/<id>/<id>.jsonl`.
    pub fn transcript_path(&self, working_directory: &Path, conversation: &str) -> PathBuf {
        self.cursor_home
            .join("projects")
            .join(project_slug(working_directory))
            .join("agent-transcripts")
            .join(conversation)
            .join(format!("{conversation}.jsonl"))
    }

    /// `<cursor home>/chats/<md5 of the working directory>/<id>`: `store.db` and its
    /// companions, what a resume reads.
    pub fn chat_directory(&self, working_directory: &Path) -> PathBuf {
        let digest = Md5::digest(working_directory.to_string_lossy().as_bytes());
        let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
        self.cursor_home.join("chats").join(hex)
    }

    /// `current` with `conversation` and its transcript path, `None` when both are already
    /// recorded.
    fn with_conversation(
        &self,
        session: &Agent,
        current: &Value,
        conversation: &str,
    ) -> Option<Value> {
        if current.get(CONVERSATION_ID_KEY).and_then(Value::as_str) == Some(conversation) {
            return None;
        }
        let transcript = self.transcript_path(&session.working_directory, conversation);
        let mut data = current.as_object().cloned().unwrap_or_default();
        data.insert(CONVERSATION_ID_KEY.into(), conversation.into());
        data.insert(
            TRANSCRIPT_PATH_KEY.into(),
            transcript.to_string_lossy().into_owned().into(),
        );
        Some(Value::Object(data))
    }

    /// The command line every launch shares after the `args` that name the conversation.
    fn plan(
        &self,
        request: &LaunchRequest<'_>,
        mut args: Vec<String>,
        stored_data: Option<Value>,
        prompt: &str,
    ) -> Result<LaunchPlan, EngineError> {
        if let Some((name, provider)) = request.profile.provided_skills().first() {
            return Err(EngineError::Refused(format!(
                "cursor: skill {name:?} is provided by {} and does not exist on cursor",
                provider.as_str()
            )));
        }
        let data = engine_data(self, request.session);
        let option = |key: &str| data.get(key).and_then(Value::as_str);
        if let Some(model) = option(MODEL_KEY) {
            args.extend([MODEL_FLAG.to_owned(), model.to_owned()]);
        }
        let working_directory = &request.session.working_directory;
        let mut workspace = Vec::new();
        if !request.profile.mcp_servers().is_empty() {
            args.extend([
                PLUGIN_DIR_FLAG.to_owned(),
                working_directory
                    .join(PLUGIN_DIR)
                    .to_string_lossy()
                    .into_owned(),
            ]);
            workspace.push(WorkspaceAction::Write {
                path: PLUGIN_MANIFEST.into(),
                content: serde_json::json!({ "name": PLUGIN_NAME }).to_string(),
            });
            workspace.push(WorkspaceAction::Write {
                path: PLUGIN_MCP_FILE.into(),
                content: serde_json::json!({ "mcpServers": request.profile.mcp_servers() })
                    .to_string(),
            });
        }
        args.extend(LAUNCH_FLAGS.iter().map(|flag| (*flag).to_owned()));
        args.extend(
            data.get(ENGINE_ARGS_OPTION)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned),
        );
        if !prompt.is_empty() {
            args.push(prompt.to_owned());
        }
        if !request.hook_command.is_empty() {
            workspace.push(WorkspaceAction::Write {
                path: HOOKS_FILE.into(),
                content: hooks_file(request.hook_command),
            });
        }
        if !request.profile.text().is_empty() {
            workspace.push(WorkspaceAction::Write {
                path: PROFILE_RULE.into(),
                content: profile_rule(request.profile.text()),
            });
        }
        workspace.extend(skill_links(request.profile.skill_dirs()));
        let mut plan = LaunchPlan::new(option(COMMAND_KEY).unwrap_or(CURSOR_BIN)).args(args);
        plan.engine_data = stored_data;
        for action in workspace {
            plan = plan.workspace(action);
        }
        Ok(plan)
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

impl LlmEngine for CursorAdapter {
    fn kind(&self) -> EngineKind {
        EngineKind::CURSOR
    }

    fn ext_key(&self) -> &'static str {
        EXT_KEY
    }

    fn configure(&self, session: &Agent, options: &Value) -> Result<Value, EngineError> {
        if !options.is_object() {
            return Err(EngineError::Refused(format!(
                "cursor options: expected a JSON object, got {options}"
            )));
        }
        let options: CursorOptions = serde_json::from_value(options.clone())
            .map_err(|error| EngineError::Refused(format!("cursor options: {error}")))?;
        for (key, value) in [(MODEL_KEY, &options.model), (COMMAND_KEY, &options.command)] {
            if value.as_deref() == Some("") {
                return Err(EngineError::Refused(format!(
                    "cursor options: {key} must not be empty"
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
        if let Some(command) = options.command {
            data.insert(COMMAND_KEY.into(), command.into());
        }
        if let Some(args) = options.args {
            data.insert(ENGINE_ARGS_OPTION.into(), args.into());
        }
        Ok(Value::Object(data))
    }

    /// The chat is named after the session id, so the plan stores the conversation up front.
    fn start(&self, request: &LaunchRequest<'_>, prompt: &str) -> Result<LaunchPlan, EngineError> {
        let id = request.session.id.to_string();
        let current = engine_data(self, request.session);
        let data = self
            .with_conversation(request.session, current, &id)
            .unwrap_or_else(|| current.clone());
        let args = vec![NEW_SESSION_ID_FLAG.to_owned(), id];
        self.plan(request, args, Some(data), prompt)
    }

    fn resume(&self, request: &LaunchRequest<'_>) -> Result<LaunchPlan, EngineError> {
        let Some((conversation, _)) = self.recorded_conversation(request.session) else {
            return Err(EngineError::Refused(format!(
                "cursor cannot resume {}: no conversation recorded (ext.{EXT_KEY}.{CONVERSATION_ID_KEY})",
                request.session.name
            )));
        };
        let args = vec![RESUME_FLAG.to_owned(), conversation.to_owned()];
        self.plan(request, args, None, "")
    }

    fn fork(
        &self,
        _: &LaunchRequest<'_>,
        source: &Agent,
        _: &str,
    ) -> Result<LaunchPlan, EngineError> {
        Err(EngineError::Refused(format!(
            "cursor cannot fork {}: cursor-agent has no fork",
            source.name
        )))
    }

    /// Ctrl-C once, as the status line says ("ctrl+c to stop"); a second one would exit. The
    /// interrupted prompt is put back into the input.
    fn interrupt(&self, _: &Agent) -> Option<(Input, Option<LlmEngineState>)> {
        Some((Input::Key(Key::ControlC), Some(LlmEngineState::Idle)))
    }

    /// `stop` ends every turn (completed, aborted, and the error that follows an abort);
    /// `workspaceOpen` fires once the TUI is up, on a resume too, which fires no
    /// `sessionStart`; its payload names no conversation. With every tool allowed nothing stops
    /// on an approval, and the AskQuestion dialog fires no hook, so nothing says `Waiting`.
    fn on_hook(&self, session: &Agent, hook: &LlmEngineHook) -> LlmEngineChange {
        let change = match hook.name.as_str() {
            "workspaceOpen" | "sessionStart" | "stop" => {
                LlmEngineChange::idle(IdleReason::Finished)
            }
            "beforeSubmitPrompt" | "preToolUse" | "postToolUse" => {
                LlmEngineChange::to(LlmEngineState::Working)
            }
            _ => LlmEngineChange::NONE,
        };
        let engine_data = hook
            .payload
            .get(CONVERSATION_ID_KEY)
            .and_then(Value::as_str)
            .and_then(|conversation| {
                self.with_conversation(session, engine_data(self, session), conversation)
            });
        LlmEngineChange {
            engine_data,
            ..change
        }
    }

    /// The transcript by offset while the process lives, compared whole once it is gone; the
    /// chat directory's files only then, when SQLite has checkpointed its log into `store.db`.
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
        let chat = self
            .chat_directory(&session.working_directory)
            .join(conversation);
        let entries = match std::fs::read_dir(&chat) {
            Ok(entries) => entries,
            // A process that never opened its chat leaves none.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(source) => return Err(EngineError::Io { path: chat, source }),
        };
        let archived_chat = archive_directory.join(conversation);
        for entry in entries {
            let entry = entry.map_err(|source| EngineError::Io {
                path: chat.clone(),
                source,
            })?;
            if entry.path().is_file() {
                mirror_file_exact(&entry.path(), &archived_chat.join(entry.file_name()))?;
            }
        }
        Ok(())
    }

    /// The chat directory's files when its `store.db` is gone, and the transcript when it is
    /// gone, from the archive; a live chat is never touched.
    fn restore(&self, session: &Agent, archive_directory: &Path) -> Result<bool, EngineError> {
        let Some((conversation, transcript)) = self.recorded_conversation(session) else {
            return Ok(false);
        };
        let archived_chat = archive_directory.join(conversation);
        let chat = self
            .chat_directory(&session.working_directory)
            .join(conversation);
        let mut restored = false;
        if archived_chat.join("store.db").is_file() && !chat.join("store.db").exists() {
            let entries = std::fs::read_dir(&archived_chat).map_err(|source| EngineError::Io {
                path: archived_chat.clone(),
                source,
            })?;
            for entry in entries {
                let entry = entry.map_err(|source| EngineError::Io {
                    path: archived_chat.clone(),
                    source,
                })?;
                mirror_file_exact(&entry.path(), &chat.join(entry.file_name()))?;
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

    fn models(&self) -> Vec<String> {
        MODELS.map(str::to_owned).to_vec()
    }

    fn check(&self, request: &DefinitionsRequest<'_>) -> Result<EngineCheck, EngineError> {
        Ok(check_report(request))
    }
}

/// What one agent definition gets on Cursor that its file does not say.
pub fn definition_notes(agent: &AgentSummary) -> Vec<String> {
    let mut notes = Vec::new();
    if !agent.tools.is_empty() || !agent.disallowed_tools.is_empty() {
        notes.push(
            "cursor has no tool switches: `tools` and `disallowedTools` change nothing".into(),
        );
    }
    notes
}

/// What `tx harness check --engine cursor` reports: [`FORCED`]; per agent each skill an engine
/// provides, which Cursor has not got, at the line of its `skills`; per agent
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
                        "skill {name:?} is provided by {}: not on cursor",
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

/// The plugin wrapper: registers [`CursorAdapter`] under the plugin id `engine-cursor`, with the
/// Cursor home resolved once from the environment.
#[derive(Clone, Copy, Debug, Default)]
pub struct CursorPlugin;

/// `CURSOR_CONFIG_DIR`, else `$HOME/.cursor`.
pub fn cursor_home_from_env() -> PathBuf {
    match std::env::var_os("CURSOR_CONFIG_DIR") {
        Some(directory) if !directory.is_empty() => PathBuf::from(directory),
        _ => std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default()
            .join(".cursor"),
    }
}

impl tx_core::Plugin for CursorPlugin {
    fn id(&self) -> &'static str {
        "engine-cursor"
    }

    fn register(&self, registry: &mut tx_core::Registry) -> Result<(), tx_core::PluginError> {
        registry.engine(Box::new(CursorAdapter::new(cursor_home_from_env())));
        Ok(())
    }
}
