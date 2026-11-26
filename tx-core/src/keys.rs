//! A plugin's key table: the `keys` mapping of its settings, action name → tmux key. The
//! plugin's default settings name every action it has; the user's block may move one
//! (`keys.<action>: <key>`) or unbind it (`null`). A default ending in `-` (`M-`) is a modifier
//! family: the action binds that modifier with each digit 1-9, and a user's value is modifiers
//! too. The front end turns the table into tmux bindings; core only checks the user's block.

use serde_json::Value;

use crate::core::TxCore;
use crate::plugin::PluginError;

/// The settings key that holds a plugin's key table.
pub const KEYS: &str = "keys";

/// Refuses a user settings block whose `keys` names an action `defaults` does not have, or holds
/// a value that is not a tmux key (or, for a modifier family, not modifiers). No `keys`: nothing
/// to check.
pub fn check(plugin: &str, defaults: &Value, user: &Value) -> Result<(), PluginError> {
    let Some(table) = user.get(KEYS) else {
        return Ok(());
    };
    let refuse = |reason: String| PluginError::Register {
        plugin: plugin.to_owned(),
        reason,
    };
    let Value::Object(table) = table else {
        return Err(refuse(format!(
            "settings.{KEYS} must map an action to a tmux key, got {table}"
        )));
    };
    let known = defaults.get(KEYS).and_then(Value::as_object);
    for (action, value) in table {
        let Some(default) = known.and_then(|known| known.get(action)) else {
            let actions: Vec<&str> = known
                .into_iter()
                .flat_map(|known| known.keys().map(String::as_str))
                .collect();
            return Err(refuse(format!(
                "settings.{KEYS}.{action}: no such action (this plugin has: {})",
                if actions.is_empty() {
                    "none".to_owned()
                } else {
                    actions.join(", ")
                }
            )));
        };
        let family = default.as_str().is_some_and(|key| key.ends_with('-'));
        let valid = match value {
            Value::Null => true,
            Value::String(key) if family => is_modifiers(key),
            Value::String(key) => is_tmux_key(key),
            _ => false,
        };
        if !valid {
            let expected = if family {
                "modifiers such as M- or C-M- (the digits 1-9 are added), or null"
            } else {
                "a tmux key such as t, C-t, M-Left or F5, or null"
            };
            return Err(refuse(format!(
                "settings.{KEYS}.{action}: {value} is not {expected}"
            )));
        }
    }
    Ok(())
}

/// tmux's names for keys that are not one character (case-insensitive, as tmux reads them).
const NAMED: &[&str] = &[
    "Tab", "BTab", "Space", "BSpace", "Enter", "Escape", "Up", "Down", "Left", "Right", "Home",
    "End", "IC", "Insert", "DC", "Delete", "NPage", "PageDown", "PgDn", "PPage", "PageUp", "PgUp",
];

/// Modifiers (`C-`, `M-`, `S-`) and then one key: a printable ASCII character, a named key, or
/// F1-F12. A quote or backslash is refused: the key is spliced into single-quoted tmux config.
pub fn is_tmux_key(text: &str) -> bool {
    let mut base = text;
    while let Some(rest) = strip_modifier(base).filter(|rest| !rest.is_empty()) {
        base = rest;
    }
    let mut characters = base.chars();
    let single = match (characters.next(), characters.next()) {
        (Some(character), None) => Some(character),
        _ => None,
    };
    match single {
        Some(character) => character.is_ascii_graphic() && !"'\"\\".contains(character),
        None => {
            NAMED.iter().any(|name| name.eq_ignore_ascii_case(base))
                || base
                    .strip_prefix(['F', 'f'])
                    .and_then(|number| number.parse::<u8>().ok())
                    .is_some_and(|number| (1..=12).contains(&number))
        }
    }
}

/// One or more modifiers and nothing else: `M-`, `C-M-`.
pub fn is_modifiers(text: &str) -> bool {
    let mut rest = text;
    while let Some(after) = strip_modifier(rest) {
        rest = after;
    }
    rest.is_empty() && !text.is_empty()
}

fn strip_modifier(text: &str) -> Option<&str> {
    let mut characters = text.chars();
    match (characters.next(), characters.next()) {
        (Some('C' | 'c' | 'M' | 'm' | 'S' | 's'), Some('-')) => Some(&text[2..]),
        _ => None,
    }
}

/// One spelling per key, for telling whether two settings name the same one: tmux reads
/// modifiers in any case and order (`c-M-x` is `M-C-x`) and key names in any case (`pgup`), but
/// a single character as written (`t` is not `T`).
pub fn canonical(key: &str) -> String {
    let mut modifiers = Vec::new();
    let mut base = key;
    while let Some(rest) = strip_modifier(base).filter(|rest| !rest.is_empty()) {
        modifiers.push(base[..1].to_ascii_uppercase());
        base = rest;
    }
    modifiers.sort();
    modifiers.dedup();
    let base = if base.chars().count() > 1 {
        base.to_ascii_lowercase()
    } else {
        base.to_owned()
    };
    modifiers
        .into_iter()
        .map(|modifier| format!("{modifier}-"))
        .chain([base])
        .collect()
}

/// How a message names what the user pressed for a plugin's prefix-table action: `prefix+t` with
/// the key the settings give it, or the plugin's id when the action is unbound (it ran some other
/// way).
pub fn label(core: &TxCore, plugin: &str, action: &str) -> String {
    match core
        .plugin_settings(plugin)
        .and_then(|settings| settings.get(KEYS)?.get(action)?.as_str())
    {
        Some(key) => format!("prefix+{key}"),
        None => plugin.to_owned(),
    }
}

/// The tmux keys one table entry binds: a modifier family is its nine digits, a key itself.
pub fn expand(key: &str) -> Vec<String> {
    if key.ends_with('-') {
        (1..=9).map(|digit| format!("{key}{digit}")).collect()
    } else {
        vec![key.to_owned()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn one_key_has_one_canonical_spelling() {
        assert_eq!(canonical("c-t"), canonical("C-t"));
        assert_eq!(canonical("m-C-x"), canonical("C-M-x"));
        assert_eq!(canonical("pgup"), canonical("PgUp"));
        assert_eq!(canonical("C-Tab"), "C-tab");
        assert_ne!(canonical("t"), canonical("T"));
        assert_ne!(canonical("C-t"), canonical("C-T"));
        assert_eq!(canonical("-"), "-");
        assert_eq!(canonical("M--"), "M--");
    }

    #[test]
    fn plausible_keys() {
        for key in [
            "t", "T", "/", "-", "|", ";", "#", "C-t", "M-Left", "C-S-Tab", "c-h", "F5", "f12",
            "C--", "Space", "pageup", "M-C-x",
        ] {
            assert!(is_tmux_key(key), "{key:?}");
        }
        for key in [
            "",
            " ",
            "tt",
            "C-",
            "F13",
            "F0",
            "'",
            "\"",
            "\\",
            "C-'",
            "é",
            "open picker",
            "X-t",
            "None",
        ] {
            assert!(!is_tmux_key(key), "{key:?}");
        }
    }

    #[test]
    fn modifier_families() {
        for text in ["M-", "C-M-", "s-"] {
            assert!(is_modifiers(text), "{text:?}");
        }
        for text in ["", "M", "M-1", "-", "X-"] {
            assert!(!is_modifiers(text), "{text:?}");
        }
        assert_eq!(expand("M-").len(), 9);
        assert_eq!(expand("M-")[0], "M-1");
        assert_eq!(expand("C-Tab"), ["C-Tab"]);
    }

    #[test]
    fn a_user_table_is_checked_against_the_defaults() {
        let defaults = json!({"keys": {"open": "t", "pane": "M-"}});
        let ok = |user| check("picker", &defaults, &user);
        assert!(ok(json!(null)).is_ok());
        assert!(ok(json!({"other": 1})).is_ok());
        assert!(ok(json!({"keys": {"open": "p", "pane": "C-M-"}})).is_ok());
        assert!(ok(json!({"keys": {"open": null, "pane": null}})).is_ok());
        let error = ok(json!({"keys": {"opne": "p"}})).unwrap_err().to_string();
        assert!(
            error.contains("picker") && error.contains("keys.opne"),
            "{error}"
        );
        assert!(error.contains("open, pane"), "{error}");
        let error = ok(json!({"keys": {"open": "two words"}}))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("keys.open") && error.contains("two words"),
            "{error}"
        );
        assert!(ok(json!({"keys": {"pane": "M-1"}})).is_err());
        assert!(ok(json!({"keys": {"open": "M-"}})).is_err());
        assert!(ok(json!({"keys": {"open": 1}})).is_err());
        assert!(ok(json!({"keys": null})).is_err());
        assert!(check("x", &json!(null), &json!({"keys": {"a": "b"}})).is_err());
    }
}
