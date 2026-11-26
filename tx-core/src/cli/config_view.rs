//! `tx config`: the config as tx reads it. The defaults are assembled, never stored: the core's
//! own (`defaults.yaml`: `agent:`, `worktree:` and `theme:`) and, for every installed plugin, the
//! `settings` of its manifest. The user's `<config>/config.yaml` holds only what they change
//! and is laid over them ([`crate::merge_settings`]), so nothing ever has to write the defaults
//! into that file.

use std::fmt::Write;
use std::path::Path;

use serde_json::Value;

use crate::host::args::UsageError;
use crate::host::user_config::RETIRED;
use crate::plugin::{Index, MANIFEST_FILE};

/// The core's own defaults, with the comments that document each key.
pub const CORE_DEFAULTS: &str = include_str!("../../defaults.yaml");

const PLUGINS_KEY: &str = "plugins";
const SETTINGS_KEY: &str = "settings";

/// Every default spelled out, as one commented config file: what `tx config --defaults` prints.
/// A plugin's settings are the `settings:` block of its `plugin.yaml` as written, comments and
/// all.
pub fn defaults_text(index: &Index) -> String {
    let mut text = format!(
        "# tx's defaults, for this build and the plugins installed here. Your own config.yaml\n\
         # (`tx config --path`) holds only the keys you change; `tx config` shows the two merged.\n\n\
         {CORE_DEFAULTS}\n\
         # The plugins that are installed. `enabled: false` switches one off; `tx plugins` shows the\n\
         # result.\n\
         {PLUGINS_KEY}:"
    );
    let mut any = false;
    for installed in &index.plugins {
        let Ok(manifest) = &installed.manifest else {
            continue;
        };
        any = true;
        if manifest.settings.is_null() {
            write!(text, "\n  {}: {{ enabled: true }}", installed.id).unwrap();
            continue;
        }
        write!(text, "\n  {}:\n    enabled: true", installed.id).unwrap();
        let written = std::fs::read_to_string(installed.directory.join(MANIFEST_FILE))
            .ok()
            .and_then(|manifest| settings_block(&manifest));
        match written {
            Some(Block::Inline(value)) => write!(text, "\n    {SETTINGS_KEY}: {value}").unwrap(),
            Some(Block::Lines(lines)) => {
                write!(text, "\n    {SETTINGS_KEY}:").unwrap();
                for line in lines {
                    if line.is_empty() {
                        text.push('\n');
                    } else {
                        write!(text, "\n      {line}").unwrap();
                    }
                }
            }
            // The file changed under the index: the values without their comments.
            None => {
                write!(text, "\n    {SETTINGS_KEY}:").unwrap();
                let settings = serde_norway::to_string(&manifest.settings)
                    .expect("settings that were read from YAML are written as YAML");
                for line in settings.lines() {
                    write!(text, "\n      {line}").unwrap();
                }
            }
        }
    }
    if !any {
        text.push_str(" {}");
    }
    text.push('\n');
    text
}

/// The `settings:` of a manifest as its file has it.
#[derive(Debug, PartialEq, Eq)]
enum Block {
    /// On the key's own line: `settings: { editor: nvim }`.
    Inline(String),
    /// On the lines under the key, with their common indentation removed.
    Lines(Vec<String>),
}

/// The top-level `settings:` of a manifest's text: its value as written, comments included.
fn settings_block(manifest: &str) -> Option<Block> {
    let mut lines = manifest.lines();
    let rest = lines.find_map(|line| line.strip_prefix("settings:"))?;
    let inline = rest.trim();
    if !inline.is_empty() && !inline.starts_with('#') {
        return Some(Block::Inline(inline.to_owned()));
    }
    let top_level = |line: &str| !line.starts_with([' ', '#']) && !line.trim().is_empty();
    let mut block: Vec<&str> = lines.take_while(|line| !top_level(line)).collect();
    while block
        .last()
        .is_some_and(|line| line.trim().is_empty() || !line.starts_with(' '))
    {
        block.pop();
    }
    let indent = block
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start().len())
        .min()?;
    Some(Block::Lines(
        block
            .iter()
            .map(|line| line.get(indent..).unwrap_or("").trim_end().to_owned())
            .collect(),
    ))
}

/// A config file's text as a value; `Null` for an empty one. `path` only names the file in
/// errors.
pub fn value(text: &str, path: &Path) -> Result<Value, UsageError> {
    serde_norway::from_str(text).map_err(|error| UsageError(format!("{}: {error}", path.display())))
}

/// The user's config laid over the defaults, one line per value; a value the user's file sets is
/// marked as theirs, and as redundant when it only repeats the default.
pub fn effective_text(defaults: &Value, mut user: Value, path: &Path) -> String {
    if let Some(plugins) = user.get_mut(PLUGINS_KEY).and_then(Value::as_object_mut) {
        plugins.retain(|id, _| !RETIRED.contains(&id.as_str()));
        // `settings: null` is no settings of the user's: the plugin's own stay.
        for entry in plugins.values_mut().filter_map(Value::as_object_mut) {
            entry.retain(|key, value| key != SETTINGS_KEY || !value.is_null());
        }
    }
    let mut merged = defaults.clone();
    // An empty file is null, which would replace the defaults whole.
    if !user.is_null() {
        crate::merge_settings(&mut merged, user.clone());
    }
    let mut text = format!(
        "# tx's defaults with {} laid over them. `tx config --defaults` documents every key.\n",
        path.display()
    );
    for (key, value) in merged.as_object().into_iter().flatten() {
        emit(&mut text, 0, key, value, defaults.get(key), user.get(key));
    }
    text
}

fn emit(
    text: &mut String,
    depth: usize,
    key: &str,
    merged: &Value,
    default: Option<&Value>,
    user: Option<&Value>,
) {
    let indent = "  ".repeat(depth);
    let key = scalar(&Value::String(key.to_owned()));
    if let Some(entries) = merged.as_object().filter(|entries| !entries.is_empty()) {
        writeln!(text, "{indent}{key}:").unwrap();
        for (name, value) in entries {
            emit(
                text,
                depth + 1,
                name,
                value,
                default.and_then(|default| default.get(name)),
                user.and_then(|user| user.get(name)),
            );
        }
        return;
    }
    let line = format!("{indent}{key}: {}", flow(merged));
    match user {
        None => writeln!(text, "{line}").unwrap(),
        Some(user) if Some(user) == default => {
            writeln!(text, "{line:<44} # yours, same as the default").unwrap();
        }
        Some(_) => writeln!(text, "{line:<44} # yours").unwrap(),
    }
}

/// A value on one line: a scalar as YAML writes it, a list in YAML's flow style, an empty
/// mapping as `{}`.
fn flow(value: &Value) -> String {
    match value {
        Value::Array(items) => {
            let items: Vec<String> = items.iter().map(flow).collect();
            format!("[{}]", items.join(", "))
        }
        Value::Object(entries) => {
            let entries: Vec<String> = entries
                .iter()
                .map(|(key, value)| {
                    format!("{}: {}", scalar(&Value::String(key.clone())), flow(value))
                })
                .collect();
            format!("{{{}}}", entries.join(", "))
        }
        scalar_value => scalar(scalar_value),
    }
}

fn scalar(value: &Value) -> String {
    serde_norway::to_string(value)
        .expect("a scalar serialises")
        .trim_end()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::user_config::{self, AgentDefaults, ThemeSettings, WorktreeSettings};

    fn path() -> &'static Path {
        Path::new("/c/config.yaml")
    }

    /// The core's own defaults say exactly what a missing config file means.
    #[test]
    fn the_core_defaults_are_the_built_in_defaults() {
        let config = user_config::parse(CORE_DEFAULTS, path()).unwrap();
        assert_eq!(config.agent, AgentDefaults::default());
        assert_eq!(config.worktree, WorktreeSettings::default());
        assert_eq!(config.theme, ThemeSettings::default());
        assert!(config.plugins.is_empty());
    }

    #[test]
    fn a_manifests_settings_are_taken_as_written() {
        assert_eq!(
            settings_block("id: a\nsettings: { editor: nvim }   # the editor\nverbs: []\n"),
            Some(Block::Inline("{ editor: nvim }   # the editor".to_owned()))
        );
        assert_eq!(
            settings_block(
                "id: a\nsettings:\n  # what it runs\n  editor: nvim\n\n  keys:\n    open: t  # prefix+t\n\n# next\nverbs: []\n"
            ),
            Some(Block::Lines(
                [
                    "# what it runs",
                    "editor: nvim",
                    "",
                    "keys:",
                    "  open: t  # prefix+t"
                ]
                .map(str::to_owned)
                .to_vec()
            ))
        );
        assert_eq!(settings_block("id: a\nverbs: []\n"), None);
        assert_eq!(settings_block("id: a\nsettings:\nverbs: []\n"), None);
    }

    #[test]
    fn the_users_values_are_laid_over_the_defaults_and_marked() {
        let defaults = value(
            "agent: { engine: claude, effort: 3, roles: [] }\n\
             plugins:\n  assistant: { enabled: true, settings: { model: opus, effort: 2 } }\n  chat: { enabled: true }\n",
            path(),
        )
        .unwrap();
        let user = value(
            "agent: { effort: 4, engine: claude }\n\
             plugins:\n  assistant: { settings: { effort: 3 } }\n  chat: { enabled: false, settings: null }\n  hooks: { enabled: true }\n",
            path(),
        )
        .unwrap();
        let text = effective_text(&defaults, user, path());
        let lines: Vec<&str> = text.lines().skip(1).collect();
        assert_eq!(
            lines,
            [
                "agent:",
                "  engine: claude                             # yours, same as the default",
                "  effort: 4                                  # yours",
                "  roles: []",
                "plugins:",
                "  assistant:",
                "    enabled: true",
                "    settings:",
                "      model: opus",
                "      effort: 3                              # yours",
                "  chat:",
                "    enabled: false                           # yours",
            ],
            "a retired plugin's entry is left out\n{text}"
        );
    }

    #[test]
    fn an_empty_user_file_shows_the_defaults_unmarked() {
        let defaults = value(CORE_DEFAULTS, path()).unwrap();
        let text = effective_text(&defaults, Value::Null, path());
        assert!(!text.contains("# yours"), "{text}");
        assert!(text.contains("  definition: developer\n"), "{text}");
        assert!(text.contains("  build_outputs: [target]\n"), "{text}");
    }
}
