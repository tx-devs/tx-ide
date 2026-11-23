//! tx-core plugin `attention`: the agents that need the operator. An agent that stops on the
//! operator (an approval, a question, a plan), or finishes a turn the operator is owed, is
//! marked on its tmux session ([`marks`]); the mark shows on the status line, the operator is
//! told ([`notify`]), and prefix+Space (`attention next`) goes to the first marked agent. An
//! agent the operator is looking at is not announced, and its finished turn not marked. A mark
//! goes when the agent works again, and a finished one when its pane takes the focus
//! (`keys/attention.tmux`).

pub mod marks;
pub mod notify;
pub mod settings;

use std::sync::Arc;

use serde_json::json;
use tx_client::render::reltime;
use tx_client::show::Show;
use tx_client::tmux::Ui;
use tx_core::{
    Agent, CoreConfig, CoreError, Event, EventSink, LlmEngineState, Plugin, PluginError, Registry,
    TxCore, Verb, VerbError, VerbIo, events,
};

use crate::marks::{Mark, Need, need_of};
use crate::settings::{Desktop, Finished, Settings};

pub const PLUGIN_ID: &str = "attention";

/// What marks the assistant's session: `tx_assistant::TAG` and `ROLE`, repeated here as the
/// home does, so that this plugin does not link the assistant.
const ASSISTANT_TAG: &str = "tx-system";
const ASSISTANT_ROLE: &str = "TX-ASSISTANT";

const USAGE: &str = "attention [ls [--json] | next <client> | clear]   the agents that need the operator; next shows the first (prefix+Space)";
const NOBODY: &str = "no agent needs you";

#[derive(Clone, Copy, Debug, Default)]
pub struct AttentionPlugin;

impl Plugin for AttentionPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn register(&self, r: &mut Registry) -> Result<(), PluginError> {
        let settings = Settings::parse(r.settings()).map_err(|reason| PluginError::Register {
            plugin: PLUGIN_ID.to_owned(),
            reason,
        })?;
        r.sink(Box::new(Sink {
            settings,
            config: r.config().clone(),
        }));
        r.verb(Arc::new(Attention));
        Ok(())
    }
}

fn failed(error: impl std::fmt::Display) -> VerbError {
    VerbError::Failed(error.to_string())
}

fn is_assistant(agent: &Agent) -> bool {
    agent.tags.iter().any(|tag| tag == ASSISTANT_TAG)
        && agent.roles.iter().any(|role| role == ASSISTANT_ROLE)
}

/// Whether the operator started `agent`: nobody did it for them, or the assistant did.
fn is_own(core: &TxCore, agent: &Agent) -> Result<bool, String> {
    match &agent.parent_id {
        None => Ok(true),
        Some(parent) => core
            .get_agent(parent)
            .map(|parent| is_assistant(&parent))
            .map_err(|error| error.to_string()),
    }
}

/// Whether `event` is a state change away from `working`: its detail is `<from> → <to>`
/// (`tx_core::EventType::StateChanged`).
fn worked(event: &Event) -> bool {
    event.detail.starts_with(LlmEngineState::Working.as_str())
}

/// Marks and notifies on every state change of an agent. What the agent needs is read from
/// its record, and an agent already marked for it is left alone, so the same event twice
/// changes nothing.
struct Sink {
    settings: Settings,
    config: CoreConfig,
}

impl Sink {
    /// The mark `agent` should carry after `event`, the operator looking elsewhere. A
    /// finished turn is one the agent worked on: an agent idle straight from `starting` has
    /// nothing to show yet, and its first prompt follows at once.
    fn wanted(&self, core: &TxCore, agent: &Agent, event: &Event) -> Result<Option<Need>, String> {
        Ok(match need_of(agent) {
            Some(Need::Finished) => {
                let marked = worked(event)
                    && match self.settings.finished {
                        Finished::All => true,
                        Finished::None => false,
                        Finished::Own => is_own(core, agent)?,
                    };
                marked.then_some(Need::Finished)
            }
            need => need,
        })
    }

    fn notify(&self, ui: &Ui, agent: &Agent, need: Need) -> std::io::Result<()> {
        let text = notify::text(&agent.name, need);
        let terminals = notify::terminals(ui);
        if self.settings.message {
            notify::message(ui, &terminals, &text);
        }
        let desktop = match self.settings.desktop {
            Desktop::All => true,
            Desktop::Waiting => need == Need::Waiting,
            Desktop::None => false,
        };
        if desktop {
            match &self.settings.command {
                Some(command) => notify::command(
                    command,
                    &json!({
                        "id": agent.id.as_str(),
                        "name": agent.name,
                        "need": need.name(),
                        "text": text,
                    }),
                )?,
                None => notify::desktop(&terminals, &text)?,
            }
        }
        if self.settings.sound {
            notify::sound(&terminals)?;
        }
        Ok(())
    }

    fn handle(&self, event: &Event) -> Result<(), String> {
        let core = TxCore::open(self.config.clone()).map_err(|error| error.to_string())?;
        let agent = core
            .get_agent(&event.id)
            .map_err(|error| error.to_string())?;
        let session = match core.get_execution_location(&agent.id) {
            Ok(location) => location.session_target,
            // Ended, or its session is gone: the mark went with it.
            Err(CoreError::NotRunning(_)) => return Ok(()),
            Err(error) => return Err(error.to_string()),
        };
        let ui = Ui::from_core(&core);
        let marked = marks::read(&ui, &session);
        // Looked at, a finished turn has been seen; a question stays open until it is answered.
        let looked_at = marks::looked_at(&ui).contains(&session);
        let wanted = self
            .wanted(&core, &agent, event)?
            .filter(|need| *need == Need::Waiting || !looked_at);
        match wanted {
            Some(need) if marked != Some(need) => {
                marks::set(&ui, &session, need, event.ts);
                if looked_at {
                    return Ok(());
                }
                self.notify(&ui, &agent, need)
                    .map_err(|error| error.to_string())
            }
            Some(_) => Ok(()),
            None => {
                if marked.is_some() {
                    marks::clear(&ui, &session);
                }
                Ok(())
            }
        }
    }
}

impl EventSink for Sink {
    fn on_event(&self, event: &Event) {
        if let Err(error) = self.handle(event) {
            eprintln!("tx attention: {error}");
        }
    }
}

/// The marks that still say what their agent's record says; the others are taken back. A sink
/// is delivered at most once, so a mark can outlive its reason.
fn current(core: &TxCore, ui: &Ui) -> Result<Vec<Mark>, VerbError> {
    let mut current = Vec::new();
    for mark in marks::list(ui) {
        let agent = core.get_agent(&core.resolve(&mark.id)?)?;
        if need_of(&agent) == Some(mark.need) {
            current.push(mark);
        } else {
            marks::clear(ui, &mark.session);
        }
    }
    Ok(current)
}

fn write_out(io: &mut VerbIo<'_>, text: &str) -> Result<(), VerbError> {
    io.stdout
        .write_all(text.as_bytes())
        .map_err(|error| failed(format!("write: {error}")))
}

pub struct Attention;

impl Attention {
    /// Shows the first marked agent the operator is not already looking at on `client`'s
    /// terminal: the pane that shows it, else a popup.
    fn next(core: &TxCore, ui: &Ui, client: &str) -> Result<i32, VerbError> {
        let looked_at = marks::looked_at(ui);
        let next = current(core, ui)?
            .into_iter()
            .find(|mark| !looked_at.contains(&mark.session));
        let Some(mark) = next else {
            ui.quiet(&["display-message", "-c", client, NOBODY]);
            return Ok(0);
        };
        if mark.need == Need::Finished {
            marks::clear(ui, &mark.session);
        }
        ui.show_session(client, Show::Focus, &mark.name, None)
            .or_else(|_| ui.show_session(client, Show::Popup, &mark.name, None))
            .map_err(failed)?;
        Ok(0)
    }
}

impl Verb for Attention {
    fn name(&self) -> &'static str {
        "attention"
    }

    fn usage(&self) -> &'static str {
        USAGE
    }

    fn run(&self, core: &TxCore, io: &mut VerbIo<'_>, argv: &[String]) -> Result<i32, VerbError> {
        let ui = Ui::from_core(core);
        let words: Vec<&str> = argv.iter().map(String::as_str).collect();
        match words.as_slice() {
            [] | ["ls"] => {
                let now = events::now();
                let lines: String = current(core, &ui)?
                    .iter()
                    .map(|mark| {
                        let age = reltime(Some(mark.since), now);
                        format!("{}\t{}\t{age}\n", mark.need.name(), mark.name)
                    })
                    .collect();
                write_out(io, &lines)?;
                Ok(0)
            }
            ["ls", "--json"] => {
                let marks: Vec<_> = current(core, &ui)?
                    .iter()
                    .map(|mark| {
                        json!({
                            "id": mark.id,
                            "name": mark.name,
                            "need": mark.need.name(),
                            "since": mark.since,
                        })
                    })
                    .collect();
                write_out(io, &format!("{}\n", serde_json::Value::Array(marks)))?;
                Ok(0)
            }
            ["next", client] => Self::next(core, &ui, client),
            ["clear"] => {
                for mark in marks::list(&ui) {
                    marks::clear(&ui, &mark.session);
                }
                Ok(0)
            }
            _ => {
                let _ = writeln!(io.stderr, "usage: tx {USAGE}");
                Ok(2)
            }
        }
    }
}
