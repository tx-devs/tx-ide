//! What the prefix table holds: every key bound in it, what it runs, its note and its owner.

use std::collections::BTreeMap;
use std::path::Path;

use tx_client::tmux::Ui;
use tx_core::host::bundle::KEYS_INSTALLED;

/// Who a bound key belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Owner {
    /// A tx plugin: its id.
    Tx(String),
    /// tmux itself, or the user's own tmux.conf.
    Tmux,
}

#[derive(Clone, Debug)]
pub struct Binding {
    pub command: String,
    /// The bind's `-N` note, when it has one.
    pub note: Option<String>,
    pub owner: Owner,
}

impl Binding {
    /// The note, else the command with tx's directory flags dropped.
    pub fn description(&self) -> String {
        self.note
            .clone()
            .unwrap_or_else(|| without_directory_flags(&self.command))
    }
}

pub struct Bindings {
    /// tmux key name (`e`, `C-h`, `M-Up`, `Space`, …) to its binding.
    pub keys: BTreeMap<String, Binding>,
    /// The prefix key, as tmux names it (`C-a`).
    pub prefix: String,
}

impl Bindings {
    pub fn load(ui: &Ui, data: &Path) -> Result<Bindings, Box<dyn std::error::Error>> {
        let owners = installed_keys(&std::fs::read_to_string(data.join(KEYS_INSTALLED))?);
        let notes: BTreeMap<String, String> = ui
            .run(&["list-keys", "-N", "-T", "prefix"])?
            .lines()
            .filter_map(|line| {
                let (key, note) = line.split_once(char::is_whitespace)?;
                Some((key.to_owned(), note.trim().to_owned()))
            })
            .collect();
        let keys = ui
            .run(&["list-keys", "-T", "prefix"])?
            .lines()
            .filter_map(parse_bind_line)
            .map(|(key, command)| {
                let owner = owners
                    .get(&key)
                    .map_or(Owner::Tmux, |id| Owner::Tx(id.clone()));
                let note = notes.get(&key).cloned();
                (
                    key,
                    Binding {
                        command,
                        note,
                        owner,
                    },
                )
            })
            .collect();
        let prefix = ui
            .run(&["show-options", "-gv", "prefix"])?
            .trim()
            .to_owned();
        Ok(Bindings { keys, prefix })
    }
}

/// `<data>/keys/.installed` (`<id>\tkeys=<k k…>\t…` per line) as key to plugin id.
fn installed_keys(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let id = fields.next()?;
            let keys = fields.find_map(|field| field.strip_prefix("keys="))?;
            Some(
                keys.split_whitespace()
                    .map(|key| (key.to_owned(), id.to_owned()))
                    .collect::<Vec<_>>(),
            )
        })
        .flatten()
        .collect()
}

/// `bind-key [-r] -T prefix <key> <command>` as (key, command). tmux escapes some keys with a
/// backslash (`\"`, `\#`, `\;`), which this drops.
fn parse_bind_line(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("bind-key")?.trim_start();
    let rest = rest.strip_prefix("-r").unwrap_or(rest).trim_start();
    let rest = rest
        .strip_prefix("-T")?
        .trim_start()
        .strip_prefix("prefix")?
        .trim_start();
    let (key, command) = rest.split_once(char::is_whitespace)?;
    let key = key
        .strip_prefix('\\')
        .filter(|unescaped| !unescaped.is_empty())
        .unwrap_or(key);
    Some((key.to_owned(), command.trim().to_owned()))
}

/// tx's binds spell out its directories (`tx --config-dir <path> --data-dir <path> …`); without
/// them the subcommand shows.
fn without_directory_flags(command: &str) -> String {
    let mut words = Vec::new();
    let mut skip_next = false;
    for word in command.split(' ') {
        if skip_next {
            skip_next = false;
        } else if [
            "--config-dir",
            "--data-dir",
            "--state-dir",
            "--cache-dir",
            "--run-dir",
        ]
        .contains(&word)
        {
            skip_next = true;
        } else {
            words.push(word);
        }
    }
    words.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bind_line_gives_its_key_and_command() {
        assert_eq!(
            parse_bind_line("bind-key -r -T prefix h       resize-pane -L 15"),
            Some(("h".to_owned(), "resize-pane -L 15".to_owned()))
        );
        assert_eq!(
            parse_bind_line(r"bind-key    -T prefix \;      last-pane"),
            Some((";".to_owned(), "last-pane".to_owned()))
        );
        assert_eq!(
            parse_bind_line(r"bind-key    -T prefix \\      x"),
            Some(("\\".to_owned(), "x".to_owned()))
        );
    }

    #[test]
    fn installed_keys_map_each_key_to_its_plugin() {
        let owners =
            installed_keys("ide\tkeys=C-h C-j\tunset=\tunhook=\nedit\tkeys=e\tunset=\tunhook=\n");
        assert_eq!(owners.get("C-j").map(String::as_str), Some("ide"));
        assert_eq!(owners.get("e").map(String::as_str), Some("edit"));
    }
}
