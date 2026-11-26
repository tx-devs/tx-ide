//! The picker's vim-style normal mode and the Esc handler behind `_picker-key`. Port of main's
//! `crates/tx/src/picker_keys.rs` without the row-number digits.

use tx_client::render::{BOLD, RESET};
use tx_client::theme::{Role, active};

pub const NORMAL_PROMPT: &str = "  › ";
pub const SEARCH_PROMPT: &str = "  / ";

const ACTION_KEYS: [&str; 16] = [
    "j", "k", "g", "G", "ctrl-d", "ctrl-u", "/", "i", "d", "t", "r", "q", "y", "n", "a", "p",
];

/// Every printable key but the action keys, and space: bound to `ignore` in normal mode.
pub fn ignored_keys() -> Vec<String> {
    (b'!'..=b'~')
        .map(|byte| char::from(byte).to_string())
        .filter(|key| !ACTION_KEYS.contains(&key.as_str()))
        .chain(["space".to_owned()])
        .collect()
}

/// `(` and `)` go in a `[...]` argument: they would end a `(...)` one.
pub fn toggle_normal_keys(action: &str) -> String {
    let (parens, rest): (Vec<String>, Vec<String>) = ACTION_KEYS
        .iter()
        .map(ToString::to_string)
        .chain(ignored_keys())
        .partition(|key| key == "(" || key == ")");
    format!(
        "{action}({})+{action}[{}]",
        rest.join(","),
        parens.join(",")
    )
}

pub fn normal_footer() -> String {
    let accent = active().ansi_fg(Role::Accent);
    format!(
        "{accent}{BOLD} NORMAL{RESET}  j/k move · enter focus · a attach to a pane · p popup · i or / search · d kill · t tags · q quit"
    )
}

pub fn search_footer() -> String {
    let accent = active().ansi_fg(Role::Accent);
    format!(
        "{accent}{BOLD} SEARCH{RESET}  esc normal mode · enter focus · alt-a attach · alt-p popup"
    )
}

/// Esc: back to normal mode while searching, else close the picker.
pub fn esc(searching: bool) -> String {
    if searching {
        format!(
            "change-prompt({NORMAL_PROMPT})+{}+change-footer:{}",
            toggle_normal_keys("rebind"),
            normal_footer()
        )
    } else {
        "abort".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn esc_leaves_search_then_closes() {
        assert!(esc(true).starts_with("change-prompt(  › )+rebind("));
        assert!(esc(true).contains("NORMAL"));
        assert_eq!(esc(false), "abort");
    }

    #[test]
    fn digits_are_not_keys() {
        let ignored = ignored_keys();
        for digit in '0'..='9' {
            assert!(ignored.contains(&digit.to_string()), "{digit}");
        }
        for key in ["j", "/", "a", "p", "d", "y"] {
            assert!(!ignored.contains(&key.to_owned()), "{key}");
        }
        assert!(ignored.contains(&"space".to_owned()));
    }

    #[test]
    fn toggling_puts_parens_in_brackets() {
        let unbind = toggle_normal_keys("unbind");
        assert!(unbind.starts_with("unbind(j,k,g,G,ctrl-d,ctrl-u,/,i,d,t,r,q,y,n,a,p,"));
        assert!(unbind.ends_with("+unbind[(,)]"));
    }
}
