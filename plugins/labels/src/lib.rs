//! tx-core plugin `labels`: what a session is called, seen from tmux. `whoami` and the internal
//! feeds tmux and fzf call (`_pane-info`, `_tmux-name`, `_relabel`, `_pane-label`,
//! `_label-program`), plus the
//! `keys/labels.tmux` fragment: prefix+s session tree, pane-border labels.

use std::sync::Arc;

use tx_core::{
    Agent, AgentId, CoreError, IdleReason, ListFilter, LlmEngineState, Plugin, PluginError,
    Registry, TxCore, Verb, VerbError, VerbIo, events,
};

use tx_client::render::{PickerEntry, activity_at, reltime};
use tx_client::theme::{Role, active};
use tx_client::tmux::Ui;

pub const PLUGIN_ID: &str = "labels";

const NAME_OPTION: &str = "@tx-name";
const REMOTE_OPTION: &str = "@remote-session";

#[derive(Clone, Copy, Debug, Default)]
pub struct LabelsPlugin;

impl Plugin for LabelsPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn register(&self, r: &mut Registry) -> Result<(), PluginError> {
        r.verb(Arc::new(PaneInfo));
        r.verb(Arc::new(TmuxName));
        r.verb(Arc::new(Relabel));
        r.verb(Arc::new(Whoami));
        r.verb(Arc::new(PaneLabel));
        r.verb(Arc::new(LabelProgram));
        Ok(())
    }
}

fn write_out(io: &mut VerbIo<'_>, text: &str) -> Result<(), VerbError> {
    io.stdout
        .write_all(text.as_bytes())
        .map_err(|e| VerbError::Failed(format!("write: {e}")))
}

/// `None` when nothing has that name or id.
fn resolve_or_not_found(core: &TxCore, token: &str) -> Result<Option<AgentId>, VerbError> {
    match core.resolve(token) {
        Ok(id) => Ok(Some(id)),
        Err(CoreError::NotFound(_)) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub struct PaneInfo;

impl Verb for PaneInfo {
    fn name(&self) -> &'static str {
        "_pane-info"
    }

    fn usage(&self) -> &'static str {
        "_pane-info <session_id>   name and tags, two lines (internal)"
    }

    fn run(&self, core: &TxCore, io: &mut VerbIo<'_>, argv: &[String]) -> Result<i32, VerbError> {
        let text = argv
            .first()
            .and_then(|raw| raw.parse::<AgentId>().ok())
            .and_then(|id| core.get_agent(&id).ok())
            .map_or_else(
                || "\n\n".to_owned(),
                |session| format!("{}\n{}\n", session.name, session.tags.join(",")),
            );
        write_out(io, &text)?;
        Ok(0)
    }
}

pub struct TmuxName;

impl Verb for TmuxName {
    fn name(&self) -> &'static str {
        "_tmux-name"
    }

    fn usage(&self) -> &'static str {
        "_tmux-name <name>   the live tmux target, if any (internal)"
    }

    fn run(&self, core: &TxCore, io: &mut VerbIo<'_>, argv: &[String]) -> Result<i32, VerbError> {
        let Some(token) = argv.first() else {
            let _ = writeln!(io.stderr, "usage: tx _tmux-name <name>");
            return Ok(2);
        };
        let Some(id) = resolve_or_not_found(core, token)? else {
            return Ok(0);
        };
        let session = core.get_agent(&id)?;
        if let Some(target) = Ui::from_core(core).live_targets().get(&session.name) {
            write_out(io, &format!("{target}\n"))?;
        }
        Ok(0)
    }
}

pub struct Relabel;

impl Verb for Relabel {
    fn name(&self) -> &'static str {
        "_relabel"
    }

    fn usage(&self) -> &'static str {
        "_relabel   refresh every session's @tx-name (internal)"
    }

    fn run(&self, core: &TxCore, _io: &mut VerbIo<'_>, _argv: &[String]) -> Result<i32, VerbError> {
        let ui = Ui::from_core(core);
        let live = ui.live_targets();
        let ids = core.list_agents(&ListFilter::new().running())?;
        for id in ids {
            let session = core.get_agent(&id)?;
            if !live.contains_key(&session.name) {
                continue;
            }
            let label = if session.tags.is_empty() {
                session.name.clone()
            } else {
                format!("{} [{}]", session.name, session.tags.join(","))
            };
            let _ = ui.set_session_option(&session.name, NAME_OPTION, &label);
        }
        Ok(0)
    }
}

pub struct Whoami;

impl Verb for Whoami {
    fn name(&self) -> &'static str {
        "whoami"
    }

    fn usage(&self) -> &'static str {
        "whoami   the session this terminal runs in"
    }

    fn run(&self, core: &TxCore, io: &mut VerbIo<'_>, _argv: &[String]) -> Result<i32, VerbError> {
        let ui = Ui::from_core(core);
        if !ui.inside() {
            let _ = writeln!(io.stderr, "tx whoami: not inside the tx tmux server");
            return Ok(1);
        }
        let raw = ui
            .current_session_name()
            .ok_or_else(|| VerbError::Failed("could not read the tmux session name".into()))?;
        let name = match resolve_or_not_found(core, &raw)? {
            Some(id) => core.get_agent(&id)?.name,
            None => raw,
        };
        write_out(io, &format!("{name}\n"))?;
        Ok(0)
    }
}

pub struct PaneLabel;

impl Verb for PaneLabel {
    fn name(&self) -> &'static str {
        "_pane-label"
    }

    fn usage(&self) -> &'static str {
        "_pane-label <pane_id> [rev]   pane-border label (internal)"
    }

    fn run(&self, core: &TxCore, io: &mut VerbIo<'_>, argv: &[String]) -> Result<i32, VerbError> {
        let Some(pane) = argv.first().filter(|pane| !pane.is_empty()) else {
            return Ok(0);
        };
        let ui = Ui::from_core(core);
        let theme = active();
        let label = if let Some(remote) = ui.pane_option(pane, REMOTE_OPTION) {
            format!("(r) {remote}")
        } else if let Some(inner) = ui.inner_for_pane(pane) {
            nested_label(core, &inner)
        } else {
            format!(
                "#[{}]{}#[default]",
                theme.tmux_fg_style(Role::Muted),
                theme.glyphs.empty
            )
        };
        write_out(io, &label)?;
        Ok(0)
    }
}

/// The program the border label runs: the plugin's own when it is installed as a bundle (tmux
/// then starts it directly, not through `tx`), `tx` when it is compiled in. The key fragment asks
/// once, when the server's configuration loads.
pub struct LabelProgram;

impl Verb for LabelProgram {
    fn name(&self) -> &'static str {
        "_label-program"
    }

    fn usage(&self) -> &'static str {
        "_label-program   the program the pane-border label runs (internal)"
    }

    fn run(&self, _core: &TxCore, io: &mut VerbIo<'_>, _argv: &[String]) -> Result<i32, VerbError> {
        let program = std::env::current_exe()
            .map_err(|error| VerbError::Failed(format!("current_exe: {error}")))?;
        write_out(io, &format!("{}\n", program.display()))?;
        Ok(0)
    }
}

/// A pane's header: the agent's state, its name, how long it has been in that state, its tags.
/// The state leads, so it is found at the left of a header however wide the pane is. The raw
/// tmux name when the session has no record.
fn nested_label(core: &TxCore, inner: &str) -> String {
    let theme = active();
    let session = core.resolve(inner).and_then(|id| core.get_agent(&id)).ok();
    let Some(session) = session else {
        return format!("#[#{{?pane_active,bold,nobold}}]{inner}#[default]");
    };
    let mut label = format!(
        "{}#[#{{?pane_active,bold,nobold}}]{}#[default]{}",
        state_mark(&session),
        session.name,
        state_age(&session, events::now())
    );
    for tag in &session.tags {
        label.push_str(&format!("  #[fg={}]{tag}#[default]", theme.tag_tmux(tag)));
    }
    label
}

/// The state's glyph and a space: green while the agent works, the signal when it rests on the
/// operator (an approval, a question, a plan), muted otherwise; nothing after it ended.
fn state_mark(session: &Agent) -> String {
    let theme = active();
    let glyphs = theme.glyphs;
    let (glyph, role) = match session.state {
        LlmEngineState::Working => (glyphs.working, Role::Working),
        LlmEngineState::Starting => (glyphs.starting, Role::Muted),
        LlmEngineState::Idle if session.idle_reason == Some(IdleReason::Waiting) => {
            (glyphs.needs_you, Role::Signal)
        }
        LlmEngineState::Idle => (glyphs.idle, Role::Muted),
        LlmEngineState::Ended => return String::new(),
    };
    format!("#[{}]{glyph}#[default] ", theme.tmux_fg_style(role))
}

/// The age since a resting agent's last activity (`0m` for the first minute), in the signal
/// behind `needs you` when it rests on the operator; nothing while it works, starts or after
/// it ended.
fn state_age(session: &Agent, now: f64) -> String {
    if session.state != LlmEngineState::Idle {
        return String::new();
    }
    let theme = active();
    let since = activity_at(PickerEntry::Agent(session));
    let age = if now - since < 60.0 {
        "0m".to_owned()
    } else {
        reltime(Some(since), now)
    };
    if session.idle_reason == Some(IdleReason::Waiting) {
        format!(
            "  #[{}]needs you {age}#[default]",
            theme.tmux_fg_style(Role::Signal)
        )
    } else {
        format!("  #[{}]{age}#[default]", theme.tmux_fg_style(Role::Muted))
    }
}
