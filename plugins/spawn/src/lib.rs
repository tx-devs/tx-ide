//! tx-core plugin `spawn`: prefix+n. In the key's popup (bin/tmux-new-agent) an nvim form
//! (nvim/tx-new-agent.lua, embedded here and written to a temp dir at run time) asks for the
//! agent's name, tags, project, prompt, engine, model, effort and agent definition;
//! `_new-agent` then creates the agent session and reports on the firing client's status line.
//!
//! The project is the firing pane's directory (the poc has no projects config). The engines are
//! the active ones, each offering the models and efforts its plugin declares. The agents are
//! the agent definitions (`tx harness ls agents`), the configured default (`agent.definition`)
//! preselected; each brings its own prompt, tools, skills and roles.

use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Map, Value, json};
use tx_client::tmux::Ui;
use tx_core::harness::{Harness, HarnessError};
use tx_core::{
    CreateOptions, DefinitionChoice, EngineKind, LaunchOptions, LlmEngine, Plugin, PluginError,
    Registry, TxCore, Verb, VerbError, VerbIo,
};

pub const PLUGIN_ID: &str = "spawn";
pub const DEFAULT_TAG: &str = "agent";
/// Preselected effort, when the engine offers it; else its first.
pub const DEFAULT_EFFORT: &str = "high";
/// What an engine that declares no models (or no efforts) offers: the pick that passes no
/// `model` (or `effort`) option.
pub const UNSET: &str = "default";
const TAGS_HELP: &str = "Comma-separated (default: agent)";
const FORM_MODULE: &str = "tx-new-agent";
const FORM: &[u8] = include_bytes!("../nvim/tx-new-agent.lua");
const SPEC_ENV: &str = "TX_NEW_AGENT_SPEC";
const RTP_ENV: &str = "TX_NEW_AGENT_RTP";
/// Where the form writes its answers; also in the spec, exported for shims that stand in for
/// nvim.
const RESULT_ENV: &str = "TX_NEW_AGENT_RESULT";

#[derive(Clone, Copy, Debug, Default)]
pub struct SpawnPlugin;

impl Plugin for SpawnPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn register(&self, r: &mut Registry) -> Result<(), PluginError> {
        r.verb(Arc::new(NewAgent));
        Ok(())
    }
}

/// The form's answers, as tx-new-agent.lua writes them; `cwd` is the picked project's path.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Reply {
    pub name: String,
    pub prompt: String,
    pub engine: EngineKind,
    pub model: String,
    pub effort: String,
    pub agent: String,
    pub tags: String,
    pub cwd: String,
}

/// The agent definitions offered: every winning `agents/<name>.md` under the harness roots,
/// sorted.
pub fn agent_names(roots: impl IntoIterator<Item = PathBuf>) -> Result<Vec<String>, HarnessError> {
    Ok(Harness::new(roots).agent_files()?.into_keys().collect())
}

/// The pane's directory as the one project, named after its last component.
pub fn pane_project(path: &str) -> (String, String) {
    let name = Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(path);
    (name.to_owned(), path.to_owned())
}

/// What the form offers of one engine, as its plugin declares it: the models, the one the CLI
/// runs without `--model` first, and the efforts; [`UNSET`] alone for a list it leaves empty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineOffer {
    pub kind: EngineKind,
    pub models: Vec<String>,
    pub efforts: Vec<String>,
}

impl EngineOffer {
    pub fn of(engine: &dyn LlmEngine) -> Self {
        let or_unset = |list: Vec<String>| {
            if list.is_empty() {
                vec![UNSET.to_owned()]
            } else {
                list
            }
        };
        Self {
            kind: engine.kind(),
            models: or_unset(engine.models()),
            efforts: or_unset(engine.efforts()),
        }
    }

    /// [`DEFAULT_EFFORT`] when offered, else the first.
    fn preselected_effort(&self) -> &str {
        self.efforts
            .iter()
            .find(|effort| *effort == DEFAULT_EFFORT)
            .unwrap_or(&self.efforts[0])
    }
}

/// The form's spec: the one project, the engines, each one's models (its default first) and
/// efforts, the agents, and what is preselected (the first engine; its effort: high when
/// offered, else its first; the agent: `default_agent` when offered, else the first); `result`
/// is where the answers go.
pub fn form_spec(
    result: &Path,
    agents: &[String],
    default_agent: Option<&str>,
    project: (&str, &str),
    engines: &[EngineOffer],
) -> Value {
    let kinds: Vec<&str> = engines.iter().map(|engine| engine.kind.as_str()).collect();
    let models: Map<String, Value> = engines
        .iter()
        .map(|engine| (engine.kind.to_string(), json!(engine.models)))
        .collect();
    let efforts: Map<String, Value> = engines
        .iter()
        .map(|engine| (engine.kind.to_string(), json!(engine.efforts)))
        .collect();
    let agent = default_agent
        .filter(|agent| agents.iter().any(|offered| offered == agent))
        .or(agents.first().map(String::as_str))
        .unwrap_or_default();
    let (project_name, project_path) = project;
    json!({
        "result": result,
        "projects": [project_name],
        "project_paths": { project_name: project_path },
        "project": project_name,
        "engines": kinds,
        "models": models,
        "efforts": efforts,
        "engine": kinds[0],
        "effort": engines[0].preselected_effort(),
        "agents": agents,
        "agent": agent,
        "tags_hint": TAGS_HELP,
    })
}

/// `tutor, api` → `[tutor, api]`; empty → `[agent]`.
pub fn tags_of(tags: &str) -> Vec<String> {
    let tags: Vec<String> = tags
        .split(',')
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(str::to_owned)
        .collect();
    if tags.is_empty() {
        vec![DEFAULT_TAG.to_owned()]
    } else {
        tags
    }
}

/// The session the answers describe: the picked agent definition (which brings its roles), the
/// model only when it is not the engine's default (the first `offer` lists), the effort only
/// when it is not [`UNSET`]. The engine validates the options when the session is created.
pub fn create_options(reply: &Reply, offer: &EngineOffer) -> CreateOptions {
    let mut launch = LaunchOptions::agent(reply.engine)
        .definition(DefinitionChoice::new(&reply.agent))
        .initial_prompt(&reply.prompt);
    if reply.model != offer.models[0] {
        launch = launch.option("model", reply.model.as_str());
    }
    if reply.effort != UNSET {
        launch = launch.option("effort", reply.effort.as_str());
    }
    CreateOptions::new(&reply.name, &reply.cwd, launch).tags(tags_of(&reply.tags))
}

fn io_failed(path: &Path) -> impl FnOnce(std::io::Error) -> VerbError {
    let path = path.to_path_buf();
    move |error| VerbError::Failed(format!("{}: {error}", path.display()))
}

/// Runs the form in `directory` (the spec, the lua module and the answers all live there);
/// `None` when it was cancelled.
fn run_form(directory: &Path, spec: &Value) -> Result<Option<Reply>, VerbError> {
    let lua_dir = directory.join("lua");
    fs::create_dir_all(&lua_dir).map_err(io_failed(&lua_dir))?;
    let module = lua_dir.join(format!("{FORM_MODULE}.lua"));
    fs::write(&module, FORM).map_err(io_failed(&module))?;
    let spec_path = directory.join("spec.json");
    fs::write(&spec_path, spec.to_string()).map_err(io_failed(&spec_path))?;
    let result = directory.join("answers.json");
    // --clean: the form brings its own look, and no user config runs before it.
    process::Command::new("nvim")
        .args([
            "--clean",
            "-n",
            "-i",
            "NONE",
            "--cmd",
            &format!("set shortmess+=I | lua vim.opt.rtp:prepend(vim.env.{RTP_ENV})"),
            "-c",
            &format!("lua require('{FORM_MODULE}').open()"),
        ])
        .env(SPEC_ENV, &spec_path)
        .env(RTP_ENV, directory)
        .env(RESULT_ENV, &result)
        .status()
        .map_err(|error| VerbError::Failed(format!("could not run nvim: {error}")))?;
    if !result.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(&result).map_err(io_failed(&result))?;
    let reply: Reply = serde_json::from_str(&text)
        .map_err(|error| VerbError::Failed(format!("{}: {error}", result.display())))?;
    Ok(Some(reply))
}

pub struct NewAgent;

impl Verb for NewAgent {
    fn name(&self) -> &'static str {
        "_new-agent"
    }

    fn usage(&self) -> &'static str {
        "_new-agent <pane> <client> [--cwd DIR]   the new-agent form, then create the agent (prefix+n, internal)"
    }

    fn run(&self, core: &TxCore, io: &mut VerbIo<'_>, argv: &[String]) -> Result<i32, VerbError> {
        let (pane, client, directory) = match argv {
            [pane, client] => (pane, client, None),
            [pane, client, flag, directory] if flag == "--cwd" => {
                (pane, client, Some(directory.clone()))
            }
            _ => {
                let _ = writeln!(
                    io.stderr,
                    "usage: tx _new-agent <pane> <client> [--cwd DIR]"
                );
                return Ok(2);
            }
        };
        let ui = Ui::from_core(core);
        let path = directory
            .or_else(|| ui.display_message("#{pane_current_path}", Some(pane)))
            .or_else(|| {
                std::env::current_dir()
                    .ok()
                    .map(|cwd| cwd.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| "/".to_owned());
        let (project_name, project_path) = pane_project(&path);
        let engines: Vec<EngineOffer> = core
            .engine_kinds()
            .into_iter()
            .filter_map(|kind| core.engine(kind))
            .map(EngineOffer::of)
            .collect();
        let label = tx_core::keys::label(core, PLUGIN_ID, "new_agent");
        if engines.is_empty() {
            ui.display_status(client, &format!("{label}: no engine plugin is active"));
            return Ok(1);
        }
        let agents = match agent_names(core.config().directories.harness_roots()) {
            Ok(agents) if agents.is_empty() => {
                ui.display_status(
                    client,
                    &format!("{label}: no agent definitions (run `tx update`)"),
                );
                return Ok(1);
            }
            Ok(agents) => agents,
            Err(error) => {
                ui.display_status(client, &format!("{label}: {error}"));
                return Ok(1);
            }
        };
        let directory = std::env::temp_dir().join(format!("tx-new-agent-{}", process::id()));
        fs::create_dir_all(&directory).map_err(io_failed(&directory))?;
        let spec = form_spec(
            &directory.join("answers.json"),
            &agents,
            core.config().agent_defaults.definition.as_deref(),
            (&project_name, &project_path),
            &engines,
        );
        let reply = run_form(&directory, &spec);
        let _ = fs::remove_dir_all(&directory);
        let Some(reply) = reply? else {
            return Ok(0);
        };
        // The agent may get `<name>-2` when the name is taken: say what it is really called.
        let offer = engines
            .iter()
            .find(|offer| offer.kind == reply.engine)
            .expect("the form answers one of the engines it offered");
        let created = core
            .create_agent(create_options(&reply, offer))
            .and_then(|id| core.get_agent(&id));
        match created {
            Ok(session) => {
                ui.display_status(client, &format!("spawned {}", session.name));
                Ok(0)
            }
            Err(error) => {
                ui.display_status(client, &format!("spawn of {} failed: {error}", reply.name));
                Ok(1)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use tx_core::{Agent, EngineError, LaunchPlan, LaunchRequest, LlmEngineChange, LlmEngineHook};

    use super::*;

    /// An engine that declares `models` and `efforts`, and is never launched.
    struct Declaring {
        models: &'static [&'static str],
        efforts: &'static [&'static str],
    }

    impl LlmEngine for Declaring {
        fn kind(&self) -> EngineKind {
            "aider".parse().unwrap()
        }

        fn ext_key(&self) -> &str {
            "aider"
        }

        fn configure(&self, _: &Agent, _: &Value) -> Result<Value, EngineError> {
            unreachable!()
        }

        fn start(&self, _: &LaunchRequest<'_>, _: &str) -> Result<LaunchPlan, EngineError> {
            unreachable!()
        }

        fn fork(
            &self,
            _: &LaunchRequest<'_>,
            _: &Agent,
            _: &str,
        ) -> Result<LaunchPlan, EngineError> {
            unreachable!()
        }

        fn resume(&self, _: &LaunchRequest<'_>) -> Result<LaunchPlan, EngineError> {
            unreachable!()
        }

        fn on_hook(&self, _: &Agent, _: &LlmEngineHook) -> LlmEngineChange {
            unreachable!()
        }

        fn models(&self) -> Vec<String> {
            self.models
                .iter()
                .map(|model| (*model).to_owned())
                .collect()
        }

        fn efforts(&self) -> Vec<String> {
            self.efforts
                .iter()
                .map(|effort| (*effort).to_owned())
                .collect()
        }
    }

    fn offer(kind: EngineKind, models: &[&str], efforts: &[&str]) -> EngineOffer {
        let owned = |list: &[&str]| list.iter().map(|item| (*item).to_owned()).collect();
        EngineOffer {
            kind,
            models: owned(models),
            efforts: owned(efforts),
        }
    }

    fn claude() -> EngineOffer {
        offer(
            EngineKind::CLAUDE,
            &["fable", "opus"],
            &["low", "medium", "high", "xhigh", "max"],
        )
    }

    fn codex() -> EngineOffer {
        offer(
            EngineKind::CODEX,
            &["gpt-6-astra", "gpt-6-sol"],
            &["none", "minimal", "low", "medium", "high", "xhigh", "max"],
        )
    }

    #[test]
    fn an_engine_offers_what_it_declares_and_default_for_what_it_does_not() {
        let declaring = Declaring {
            models: &["big", "small"],
            efforts: &["low", "max"],
        };
        assert_eq!(
            EngineOffer::of(&declaring),
            offer("aider".parse().unwrap(), &["big", "small"], &["low", "max"])
        );
        let silent = Declaring {
            models: &[],
            efforts: &[],
        };
        assert_eq!(
            EngineOffer::of(&silent),
            offer("aider".parse().unwrap(), &[UNSET], &[UNSET])
        );
    }

    fn reply() -> Reply {
        Reply {
            name: "health".into(),
            prompt: "add GET /health".into(),
            engine: EngineKind::CODEX,
            model: "gpt-6-sol".into(),
            effort: "xhigh".into(),
            agent: "qa".into(),
            tags: "tutor, api".into(),
            cwd: "/p/my proj".into(),
        }
    }

    #[test]
    fn the_spec_preselects_the_first_engine_its_default_model_high_and_the_default_agent() {
        let agents = ["claude".to_owned(), "developer".to_owned(), "qa".to_owned()];
        let spec = form_spec(
            Path::new("/t/answers.json"),
            &agents,
            Some("developer"),
            ("web", "/srv/web"),
            &[
                claude(),
                codex(),
                offer("aider".parse().unwrap(), &[UNSET], &[UNSET]),
            ],
        );
        assert_eq!(spec["result"], "/t/answers.json");
        assert_eq!(spec["projects"], json!(["web"]));
        assert_eq!(spec["project_paths"], json!({"web": "/srv/web"}));
        assert_eq!(spec["project"], "web");
        assert_eq!(spec["engines"], json!(["claude", "codex", "aider"]));
        assert_eq!(spec["engine"], "claude");
        assert_eq!(spec["models"]["claude"][0], "fable");
        assert_eq!(spec["models"]["codex"], json!(["gpt-6-astra", "gpt-6-sol"]));
        assert_eq!(spec["models"]["aider"], json!(["default"]));
        assert_eq!(
            spec["efforts"]["codex"],
            json!(["none", "minimal", "low", "medium", "high", "xhigh", "max"])
        );
        assert_eq!(spec["efforts"]["aider"], json!(["default"]));
        assert_eq!(spec["effort"], "high");
        assert_eq!(spec["agents"], json!(["claude", "developer", "qa"]));
        assert_eq!(spec["agent"], "developer");
        // A default that is not offered (or none at all) preselects the first agent.
        for default in [Some("ghost"), None] {
            let spec = form_spec(
                Path::new("/t/answers.json"),
                &agents,
                default,
                ("web", "/srv/web"),
                &[claude()],
            );
            assert_eq!(spec["agent"], "claude", "{default:?}");
        }
        // A first engine without high preselects its first effort.
        let spec = form_spec(
            Path::new("/t/answers.json"),
            &agents,
            None,
            ("web", "/srv/web"),
            &[
                offer(EngineKind::CODEX, &["gpt-6-astra"], &["low", "max"]),
                claude(),
            ],
        );
        assert_eq!(spec["effort"], "low");
    }

    #[test]
    fn the_pane_directory_is_the_project() {
        assert_eq!(
            pane_project("/p/my proj"),
            ("my proj".to_owned(), "/p/my proj".to_owned())
        );
        assert_eq!(pane_project("/").0, "/");
    }

    #[test]
    fn the_agents_are_the_definitions_of_every_root() {
        let shipped = tempfile::tempdir().unwrap();
        let user = tempfile::tempdir().unwrap();
        for (root, name) in [
            (&shipped, "developer"),
            (&shipped, "qa"),
            (&user, "developer"),
            (&user, "mine"),
        ] {
            let agents = root.path().join("agents");
            fs::create_dir_all(&agents).unwrap();
            fs::write(agents.join(format!("{name}.md")), "x").unwrap();
        }
        fs::write(shipped.path().join("agents/notes.txt"), "x").unwrap();
        let roots = [
            shipped.path().to_path_buf(),
            user.path().to_path_buf(),
            PathBuf::from("/nonexistent"),
        ];
        assert_eq!(agent_names(roots).unwrap(), ["developer", "mine", "qa"]);
        assert!(agent_names([]).unwrap().is_empty());
    }

    #[test]
    fn tags_are_split_and_default_to_agent() {
        assert_eq!(tags_of("tutor, api,,"), ["tutor", "api"]);
        assert_eq!(tags_of(""), [DEFAULT_TAG]);
    }

    #[test]
    fn the_answers_become_an_agent_session_of_the_picked_definition() {
        let options = create_options(&reply(), &codex());
        assert_eq!(options.name, "health");
        assert_eq!(options.cwd, PathBuf::from("/p/my proj"));
        assert_eq!(options.tags, ["tutor", "api"]);
        let LaunchOptions::Agent {
            engine,
            profile,
            options,
            initial_prompt,
            definition,
        } = options.launch
        else {
            panic!("an agent launch");
        };
        assert_eq!(engine, EngineKind::CODEX);
        // The definition brings its roles: the launch names none.
        assert!(profile.is_empty());
        assert_eq!(definition, Some(DefinitionChoice::new("qa")));
        assert_eq!(options, json!({"effort": "xhigh", "model": "gpt-6-sol"}));
        assert_eq!(initial_prompt, "add GET /health");
    }

    #[test]
    fn the_default_model_passes_nothing_extra() {
        let defaults = Reply {
            model: "gpt-6-astra".into(),
            tags: String::new(),
            ..reply()
        };
        let options = create_options(&defaults, &codex());
        assert_eq!(options.tags, [DEFAULT_TAG]);
        let LaunchOptions::Agent { options, .. } = options.launch else {
            panic!("an agent launch");
        };
        assert_eq!(options, json!({"effort": "xhigh"}));
    }

    /// An engine that declares no models or efforts gets neither option.
    #[test]
    fn the_unset_picks_pass_nothing() {
        let unset = Reply {
            engine: "aider".parse().unwrap(),
            model: UNSET.into(),
            effort: UNSET.into(),
            ..reply()
        };
        let offer = offer("aider".parse().unwrap(), &[UNSET], &[UNSET]);
        let LaunchOptions::Agent { options, .. } = create_options(&unset, &offer).launch else {
            panic!("an agent launch");
        };
        assert_eq!(options, json!({}));
    }

    /// The engine, not the form, decides what an effort may be: a bad one is passed through
    /// and refused at create.
    #[test]
    fn a_bad_effort_is_passed_to_the_engine() {
        let bad = Reply {
            effort: "7".into(),
            ..reply()
        };
        let LaunchOptions::Agent { options, .. } = create_options(&bad, &codex()).launch else {
            panic!("an agent launch");
        };
        assert_eq!(options["effort"], "7");
    }
}
