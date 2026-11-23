//! The plugin's settings block of `config.yaml`: every key optional, a wrong value refused by name.

use serde_json::Value;

/// Which finished agents are marked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Finished {
    /// The agents the operator started: no parent, or the assistant as parent.
    Own,
    All,
    None,
}

/// Which marks raise a desktop notification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Desktop {
    Waiting,
    All,
    None,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub finished: Finished,
    /// A line on the terminal's status line.
    pub message: bool,
    pub desktop: Desktop,
    /// The terminal bell.
    pub sound: bool,
    /// The operator's own notifier, run instead of the built-in desktop delivery.
    pub command: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            finished: Finished::Own,
            message: true,
            desktop: Desktop::Waiting,
            sound: false,
            command: None,
        }
    }
}

fn word<T: Copy>(
    settings: &Value,
    key: &str,
    choices: &[(&str, T)],
    default: T,
) -> Result<T, String> {
    let Some(value) = settings.get(key) else {
        return Ok(default);
    };
    value
        .as_str()
        .and_then(|text| choices.iter().find(|(name, _)| *name == text))
        .map(|(_, choice)| *choice)
        .ok_or_else(|| {
            let names = choices.iter().map(|(name, _)| *name).collect::<Vec<_>>();
            format!(
                "settings.{key} must be one of: {}, got {value}",
                names.join(", ")
            )
        })
}

fn flag(settings: &Value, key: &str, default: bool) -> Result<bool, String> {
    match settings.get(key) {
        None => Ok(default),
        Some(Value::Bool(flag)) => Ok(*flag),
        Some(other) => Err(format!("settings.{key} must be true or false, got {other}")),
    }
}

impl Settings {
    pub fn parse(settings: &Value) -> Result<Self, String> {
        let default = Self::default();
        let command = match settings.get("command") {
            None | Some(Value::Null) => None,
            Some(Value::String(command)) => Some(command.clone()),
            Some(other) => {
                return Err(format!(
                    "settings.command must be a command line or null, got {other}"
                ));
            }
        };
        Ok(Self {
            finished: word(
                settings,
                "finished",
                &[
                    ("own", Finished::Own),
                    ("all", Finished::All),
                    ("none", Finished::None),
                ],
                default.finished,
            )?,
            message: flag(settings, "message", default.message)?,
            desktop: word(
                settings,
                "desktop",
                &[
                    ("waiting", Desktop::Waiting),
                    ("all", Desktop::All),
                    ("none", Desktop::None),
                ],
                default.desktop,
            )?,
            sound: flag(settings, "sound", default.sound)?,
            command,
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn no_settings_are_the_defaults() {
        assert_eq!(Settings::parse(&Value::Null), Ok(Settings::default()));
        assert_eq!(Settings::parse(&json!({})), Ok(Settings::default()));
    }

    #[test]
    fn the_shipped_defaults_of_the_manifest_parse_to_the_defaults() {
        let shipped = json!({
            "finished": "own", "message": true, "desktop": "waiting", "sound": false, "command": null
        });
        assert_eq!(Settings::parse(&shipped), Ok(Settings::default()));
    }

    #[test]
    fn every_key_is_read() {
        let settings = json!({
            "finished": "all", "message": false, "desktop": "none", "sound": true, "command": "notify"
        });
        assert_eq!(
            Settings::parse(&settings),
            Ok(Settings {
                finished: Finished::All,
                message: false,
                desktop: Desktop::None,
                sound: true,
                command: Some("notify".to_owned()),
            })
        );
    }

    #[test]
    fn a_wrong_value_is_refused_by_name() {
        assert_eq!(
            Settings::parse(&json!({"finished": "mine"})).unwrap_err(),
            r#"settings.finished must be one of: own, all, none, got "mine""#
        );
        assert_eq!(
            Settings::parse(&json!({"sound": "loud"})).unwrap_err(),
            r#"settings.sound must be true or false, got "loud""#
        );
        assert!(Settings::parse(&json!({"command": 3})).is_err());
    }
}
