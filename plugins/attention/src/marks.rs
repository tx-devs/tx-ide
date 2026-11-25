//! The marks: which agents need the operator. A mark is the option [`MARK_OPTION`] on the
//! agent's own tmux session, `<need> <since>`, so the status line lists the marks with a format
//! alone, a focus hook made of tmux commands takes a finished one back (`keys/attention.tmux`),
//! and a mark goes with its session. The record stays the truth: [`need_of`] reads it.

use tx_client::tmux::Ui;
use tx_core::{Agent, IdleReason, LlmEngineState};

/// Kept in LOCKSTEP with `keys/attention.tmux`, which reads it in the status line and the hooks.
pub const MARK_OPTION: &str = "@tx-attention";
/// Global option rendered into `status-left` (`#{E:@tx-attention-bar}`): at most [`MAX_NAMED`]
/// named entries plus an overflow counter (`▲ +N` / `○ +N`), styled through `@tx-signal-style`
/// and `@tx-muted-style` so dark/light switches take effect live.
pub const BAR_OPTION: &str = "@tx-attention-bar";
/// Maximum individually named agents in the status line before collapsing into `+N`.
pub const MAX_NAMED: usize = 2;

const LIST_FORMAT: &str = "#{@tx-id}\t#{session_id}\t#{session_name}\t#{@tx-attention}";
const CLIENTS_FORMAT: &str = "#{client_tty}\t#{session_id}\t#{client_flags}";
const PANES_FORMAT: &str = "#{pane_tty}\t#{session_id}\t#{window_active}#{pane_active}";

/// Why an agent needs the operator; waiting sorts first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Need {
    /// Stopped mid-turn on an approval, a question or a plan.
    Waiting,
    /// Its turn ended.
    Finished,
}

impl Need {
    pub fn name(self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Finished => "finished",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "waiting" => Some(Self::Waiting),
            "finished" => Some(Self::Finished),
            _ => None,
        }
    }
}

/// What an idle agent's record asks of the operator; `None` while it starts, works or has ended.
pub fn need_of(agent: &Agent) -> Option<Need> {
    (agent.state == LlmEngineState::Idle).then_some(match agent.idle_reason {
        Some(IdleReason::Waiting) => Need::Waiting,
        Some(IdleReason::Finished) | None => Need::Finished,
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    /// The agent's record id (`@tx-id`).
    pub id: String,
    /// The tmux session id (`$N`).
    pub session: String,
    /// The tmux session name.
    pub name: String,
    pub need: Need,
    /// When the agent started needing the operator, epoch seconds.
    pub since: f64,
}

fn parse_mark(value: &str) -> Option<(Need, f64)> {
    let (need, since) = value.split_once(' ')?;
    Some((Need::parse(need)?, since.parse().ok()?))
}

/// The marked sessions of a `LIST_FORMAT` listing, waiting ones first, then the oldest first.
/// A session without a mark, or with a value that is not one, is not listed.
pub fn parse(listing: &str) -> Vec<Mark> {
    let mut marks: Vec<Mark> = listing
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(4, '\t');
            let (id, session, name, value) = (
                fields.next()?,
                fields.next()?,
                fields.next()?,
                fields.next()?,
            );
            let (need, since) = parse_mark(value)?;
            Some(Mark {
                id: id.to_owned(),
                session: session.to_owned(),
                name: name.to_owned(),
                need,
                since,
            })
        })
        .collect();
    marks.sort_by(|left, right| {
        (left.need, left.since)
            .partial_cmp(&(right.need, right.since))
            .expect("a parsed time is a number")
    });
    marks
}

/// The ids of the sessions the operator is looking at, from a `CLIENTS_FORMAT` and a
/// `PANES_FORMAT` listing: the session of every focused terminal (a client whose tty is not a
/// pane), then, while the active pane of that session's current window runs a nested attach,
/// the session attached there. A nested client's own `focused` flag is not read: it is set
/// until the pane it runs in first loses the focus, even in a pane that never had it.
pub fn parse_looked_at(clients: &str, panes: &str) -> Vec<String> {
    let clients: Vec<(&str, &str, &str)> = clients
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(3, '\t');
            Some((fields.next()?, fields.next()?, fields.next()?))
        })
        .collect();
    let panes: Vec<(&str, &str, &str)> = panes
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(3, '\t');
            Some((fields.next()?, fields.next()?, fields.next()?))
        })
        .collect();
    let terminals = clients.iter().filter(|(tty, _, flags)| {
        flags.split(',').any(|flag| flag == "focused")
            && !panes.iter().any(|(pane_tty, _, _)| pane_tty == tty)
    });
    let mut looked_at: Vec<String> = Vec::new();
    for (_, session, _) in terminals {
        let mut session = *session;
        // A session is visited once: one that nests itself ends the walk.
        while !looked_at.iter().any(|seen| seen == session) {
            looked_at.push(session.to_owned());
            let nested = panes
                .iter()
                .find(|(_, host, active)| *host == session && *active == "11")
                .and_then(|(pane_tty, _, _)| clients.iter().find(|(tty, _, _)| tty == pane_tty));
            match nested {
                Some((_, inner, _)) => session = inner,
                None => break,
            }
        }
    }
    looked_at
}

pub fn list(ui: &Ui) -> Vec<Mark> {
    parse(&ui.quiet(&["list-sessions", "-F", LIST_FORMAT]).1)
}

/// The ids of the sessions the operator is looking at (see [`parse_looked_at`]).
pub fn looked_at(ui: &Ui) -> Vec<String> {
    let (_, clients) = ui.quiet(&["list-clients", "-F", CLIENTS_FORMAT]);
    let (_, panes) = ui.quiet(&["list-panes", "-a", "-F", PANES_FORMAT]);
    parse_looked_at(&clients, &panes)
}

/// The mark on the session `session`, if any.
pub fn read(ui: &Ui, session: &str) -> Option<Need> {
    let value = ui.display_message(&format!("#{{{MARK_OPTION}}}"), Some(session))?;
    parse_mark(&value).map(|(need, _)| need)
}

/// Formats sorted marks for `status-left`: at most [`MAX_NAMED`] named agents, plus an overflow
/// badge (`▲ +N` when any hidden mark is waiting, `○ +N` otherwise). Empty when no agent is marked.
pub fn format_bar(marks: &[Mark]) -> String {
    if marks.is_empty() {
        return String::new();
    }
    let mut parts = Vec::new();
    for mark in marks.iter().take(MAX_NAMED) {
        let name = mark.name.replace('#', "##");
        match mark.need {
            Need::Waiting => parts.push(format!("#[#{{@tx-signal-style}}]▲ {name}#[default]")),
            Need::Finished => parts.push(format!("#[#{{@tx-muted-style}}]○ {name}#[default]")),
        }
    }
    if marks.len() > MAX_NAMED {
        let overflow = marks.len() - MAX_NAMED;
        let has_waiting = marks[MAX_NAMED..]
            .iter()
            .any(|mark| mark.need == Need::Waiting);
        if has_waiting {
            parts.push(format!("#[#{{@tx-signal-style}}]▲ +{overflow}#[default]"));
        } else {
            parts.push(format!("#[#{{@tx-muted-style}}]○ +{overflow}#[default]"));
        }
    }
    format!("{} ", parts.join(" "))
}

/// Recomputes [`BAR_OPTION`] on the tmux server from the current session marks.
pub fn refresh_bar(ui: &Ui) {
    let rendered = format_bar(&list(ui));
    if rendered.is_empty() {
        ui.quiet(&["set-option", "-gu", BAR_OPTION]);
    } else {
        ui.quiet(&["set-option", "-g", BAR_OPTION, &rendered]);
    }
}

pub fn set(ui: &Ui, session: &str, need: Need, since: f64) {
    let value = format!("{} {since}", need.name());
    ui.quiet(&["set-option", "-t", session, MARK_OPTION, &value]);
    refresh_bar(ui);
}

pub fn clear(ui: &Ui, session: &str) {
    ui.quiet(&["set-option", "-u", "-t", session, MARK_OPTION]);
    refresh_bar(ui);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mark(id: &str, session: &str, name: &str, need: Need, since: f64) -> Mark {
        Mark {
            id: id.to_owned(),
            session: session.to_owned(),
            name: name.to_owned(),
            need,
            since,
        }
    }

    #[test]
    fn marks_list_waiting_first_then_the_oldest() {
        let listing = "a\t$1\tone\tfinished 10\n\
                       b\t$2\ttwo\twaiting 30.5\n\
                       c\t$3\tthree\t\n\
                       d\t$4\tfour\twaiting 20\n\
                       \t$5\tview\t\n";
        assert_eq!(
            parse(listing),
            [
                mark("d", "$4", "four", Need::Waiting, 20.0),
                mark("b", "$2", "two", Need::Waiting, 30.5),
                mark("a", "$1", "one", Need::Finished, 10.0),
            ]
        );
    }

    #[test]
    fn a_value_that_is_not_a_mark_is_not_listed() {
        for value in ["waiting", "soon 10", "waiting ten", "waiting  10"] {
            assert_eq!(parse(&format!("a\t$1\tone\t{value}\n")), [], "{value:?}");
        }
    }

    /// A terminal on `desk`, whose window 1 is current: its active pane nests `agent`, the
    /// other pane nests `other`; `hidden` is nested in a pane of window 2.
    const PANES: &str = "/dev/ttys010\t$1\t10\n\
                         /dev/ttys011\t$1\t11\n\
                         /dev/ttys012\t$1\t01\n\
                         /dev/ttys020\t$2\t11\n\
                         /dev/ttys021\t$3\t11\n\
                         /dev/ttys022\t$4\t11\n";

    fn clients(terminal_flags: &str) -> String {
        format!(
            "/dev/ttys001\t$1\t{terminal_flags}\n\
             /dev/ttys010\t$3\tattached,focused,UTF-8\n\
             /dev/ttys011\t$2\tattached,UTF-8\n\
             /dev/ttys012\t$4\tattached,focused,UTF-8\n"
        )
    }

    #[test]
    fn the_operator_looks_at_the_terminals_session_and_down_its_active_panes() {
        assert_eq!(
            parse_looked_at(&clients("attached,focused,UTF-8"), PANES),
            ["$1", "$2"],
            "not the pane beside it, nor the other window, whatever their clients' flags"
        );
    }

    #[test]
    fn a_terminal_without_the_focus_looks_at_nothing() {
        assert_eq!(parse_looked_at(&clients("attached,UTF-8"), PANES), [""; 0]);
    }

    #[test]
    fn a_session_nested_in_itself_ends_the_walk() {
        let clients = "/dev/ttys001\t$1\tattached,focused\n/dev/ttys010\t$1\tattached\n";
        assert_eq!(parse_looked_at(clients, "/dev/ttys010\t$1\t11\n"), ["$1"]);
    }

    #[test]
    fn bar_caps_at_two_named_marks_with_overflow() {
        assert_eq!(format_bar(&[]), "");
        let marks = vec![
            mark("1", "$1", "alpha", Need::Waiting, 10.0),
            mark("2", "$2", "beta", Need::Waiting, 20.0),
            mark("3", "$3", "gamma", Need::Waiting, 30.0),
            mark("4", "$4", "delta", Need::Finished, 40.0),
        ];
        let bar = format_bar(&marks);
        assert!(bar.contains("▲ alpha"), "{bar}");
        assert!(bar.contains("▲ beta"), "{bar}");
        assert!(!bar.contains("gamma"), "{bar}");
        assert!(bar.contains("▲ +2"), "{bar}");

        let finished_overflow = vec![
            mark("1", "$1", "alpha", Need::Waiting, 10.0),
            mark("2", "$2", "beta", Need::Finished, 20.0),
            mark("3", "$3", "gamma", Need::Finished, 30.0),
        ];
        let bar = format_bar(&finished_overflow);
        assert!(
            bar.contains("▲ alpha") && bar.contains("○ beta") && bar.contains("○ +1"),
            "{bar}"
        );
    }
}
