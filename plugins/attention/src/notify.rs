//! Telling the operator: a line on the terminal's status line, a desktop notification, the bell.
//! A desktop notification is the terminal's own (OSC 9), written to the terminal client's tty,
//! past tmux: no helper program, and it reaches the terminal over ssh too. A terminal that is
//! not known to show one gets none; `settings.command`, the operator's own notifier, replaces
//! the built-in delivery for any terminal.

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::process::{Command, Stdio};

use tx_client::tmux::Ui;

use crate::marks::Need;

const CLIENTS_FORMAT: &str = "#{client_name}\t#{client_tty}\t#{client_termtype}";
const MESSAGE_MILLISECONDS: &str = "5000";
/// The start of `#{client_termtype}` for the terminals that show an OSC 9 notification.
const OSC9_TERMINALS: [&str; 3] = ["iTerm2", "ghostty", "WezTerm"];

/// A client that is a terminal, not a nested attach in one of the server's panes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Terminal {
    pub client: String,
    pub tty: String,
    /// What the terminal answered when tmux asked its name and version; empty when it did not.
    pub kind: String,
}

impl Terminal {
    fn shows_osc9(&self) -> bool {
        OSC9_TERMINALS
            .iter()
            .any(|known| self.kind.starts_with(known))
    }
}

/// The clients of a `CLIENTS_FORMAT` listing whose tty is not one of `panes` (`#{pane_tty}` per
/// line): a nested attach runs in a pane, a terminal does not.
pub fn parse_terminals(clients: &str, panes: &str) -> Vec<Terminal> {
    let panes: Vec<&str> = panes.lines().collect();
    clients
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(3, '\t');
            Some((fields.next()?, fields.next()?, fields.next()?))
        })
        .filter(|(_, tty, _)| !panes.contains(tty))
        .map(|(client, tty, kind)| Terminal {
            client: client.to_owned(),
            tty: tty.to_owned(),
            kind: kind.to_owned(),
        })
        .collect()
}

pub fn terminals(ui: &Ui) -> Vec<Terminal> {
    let (_, clients) = ui.quiet(&["list-clients", "-F", CLIENTS_FORMAT]);
    let (_, panes) = ui.quiet(&["list-panes", "-a", "-F", "#{pane_tty}"]);
    parse_terminals(&clients, &panes)
}

/// What the operator reads; control characters of the name are left out.
pub fn text(name: &str, need: Need) -> String {
    let name: String = name.chars().filter(|c| !c.is_control()).collect();
    match need {
        Need::Waiting => format!("{name} needs you"),
        Need::Finished => format!("{name} finished"),
    }
}

/// `text` on the status line of every terminal, for a few seconds.
pub fn message(ui: &Ui, terminals: &[Terminal], text: &str) {
    // tmux expands the message as a format.
    let text = text.replace('#', "##");
    for terminal in terminals {
        ui.quiet(&[
            "display-message",
            "-c",
            &terminal.client,
            "-d",
            MESSAGE_MILLISECONDS,
            &text,
        ]);
    }
}

fn write_to(tty: &str, bytes: &[u8]) -> io::Result<()> {
    OpenOptions::new().write(true).open(tty)?.write_all(bytes)
}

/// A desktop notification through every terminal that shows OSC 9.
pub fn desktop(terminals: &[Terminal], text: &str) -> io::Result<()> {
    terminals
        .iter()
        .filter(|terminal| terminal.shows_osc9())
        .try_for_each(|terminal| write_to(&terminal.tty, format!("\x1b]9;{text}\x07").as_bytes()))
}

/// The bell of every terminal.
pub fn sound(terminals: &[Terminal]) -> io::Result<()> {
    terminals
        .iter()
        .try_for_each(|terminal| write_to(&terminal.tty, b"\x07"))
}

/// The operator's notifier: `command` run by `sh`, the JSON `payload` and a newline on its
/// standard input. Not waited for.
pub fn command(command: &str, payload: &serde_json::Value) -> io::Result<()> {
    let mut child = Command::new("sh")
        .args(["-c", command])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let mut input = child.stdin.take().expect("the input is piped");
    writeln!(input, "{payload}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_client_in_a_pane_is_not_a_terminal() {
        let clients = "/dev/ttys001\t/dev/ttys001\tiTerm2 3.7.7\n\
                       /dev/ttys009\t/dev/ttys009\ttmux 3.5a\n\
                       /dev/ttys020\t/dev/ttys020\t\n";
        let panes = "/dev/ttys009\n/dev/ttys004\n";
        let terminals = parse_terminals(clients, panes);
        assert_eq!(
            terminals
                .iter()
                .map(|terminal| (terminal.tty.as_str(), terminal.kind.as_str()))
                .collect::<Vec<_>>(),
            [("/dev/ttys001", "iTerm2 3.7.7"), ("/dev/ttys020", "")]
        );
        assert!(terminals[0].shows_osc9());
        assert!(!terminals[1].shows_osc9());
    }

    #[test]
    fn the_text_names_the_agent_and_drops_control_characters() {
        assert_eq!(text("api", Need::Waiting), "api needs you");
        assert_eq!(text("a\x07pi\x1b", Need::Finished), "api finished");
    }
}
