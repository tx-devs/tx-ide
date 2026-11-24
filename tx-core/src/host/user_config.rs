//! `<config>/config.yaml`: the user's preferences, read once per invocation.
//!
//! ```yaml
//! agent:                         # defaults for `tx create`; flags override
//!   engine: claude
//!   roles: []                    # empty: the definition's `roles`; any --role replaces the whole list
//!   definition: developer        # the agent definition without --agent; null: roles alone
//!   model: claude-opus-5-5       # null: the engine's own default
//!   effort: 3                    # 1-5 (or the engine's word); null: the engine's own default
//! worktree:
//!   build_outputs: [target]      # paths in an agent's worktree `tx gc` may delete
//! plugins:
//!   engine-codex: { enabled: false }
//!   assistant:
//!     settings: { model: opus, effort: 2 }
//! ```
//! Every key is optional; a missing or empty file is every default. An unknown key, a bad value
//! or malformed YAML is a usage error naming the file.
//!
//! In `plugins:`, no entry enables a plugin. An entry that names no plugin is ignored at run
//! time, whatever its id, so a file written for another set of plugins keeps loading;
//! `tx plugins` lists it as `missing`.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

pub use crate::AgentDefaults;
use serde::Deserialize;

use crate::host::args::UsageError;

pub const FILE_NAME: &str = "config.yaml";

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub agent: AgentDefaults,
    #[serde(default)]
    pub worktree: WorktreeSettings,
    #[serde(default)]
    pub theme: ThemeSettings,
    #[serde(default)]
    pub plugins: BTreeMap<String, Entry>,
}

/// How the tmux status line is drawn: a backgroundless heavy rule (`rule`, default) or a filled
/// traditional bar (`bar`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StatusStyle {
    #[default]
    Rule,
    Bar,
}

/// Which glyph tier tx draws its mark, states and motion with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GlyphTier {
    #[default]
    Unicode,
    Ascii,
}

/// The `theme:` section of `config.yaml`: either a theme name (`theme: tokyo-night`) or a
/// mapping with `name`, `status_style`, `glyphs` and per-role colour overrides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeSettings {
    /// `auto` (detect dark/light via tmux, falling back to `terminal`), a shipped preset
    /// (`tx-dark`, `tx-light`, `terminal`, `tokyo-night`), or a file name under `<config>/themes/`.
    pub name: String,
    pub status_style: StatusStyle,
    pub glyphs: GlyphTier,
    /// Per-role colour overrides (`signal`, `accent`, `working`, `failed`, `info`, `text`,
    /// `muted`, `rule`, `rule_active`, `surface`, `selection`, `on_accent`).
    pub colors: BTreeMap<String, String>,
}

impl Default for ThemeSettings {
    fn default() -> Self {
        Self {
            name: default_theme_name(),
            status_style: StatusStyle::default(),
            glyphs: GlyphTier::default(),
            colors: BTreeMap::new(),
        }
    }
}

fn default_theme_name() -> String {
    "auto".to_owned()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeTable {
    #[serde(default = "default_theme_name")]
    name: String,
    #[serde(default)]
    status_style: StatusStyle,
    #[serde(default)]
    glyphs: GlyphTier,
    #[serde(default)]
    colors: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ThemeValue {
    Name(String),
    Table(ThemeTable),
}

impl<'de> Deserialize<'de> for ThemeSettings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        match ThemeValue::deserialize(deserializer)? {
            ThemeValue::Name(name) => Ok(Self {
                name,
                ..Self::default()
            }),
            ThemeValue::Table(table) => Ok(Self {
                name: table.name,
                status_style: table.status_style,
                glyphs: table.glyphs,
                colors: table.colors,
            }),
        }
    }
}

/// The `worktree:` section.
#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeSettings {
    /// What `tx gc` deletes in the worktree of an ended agent.
    #[serde(default = "default_build_outputs")]
    pub build_outputs: Vec<BuildOutput>,
}

impl Default for WorktreeSettings {
    fn default() -> Self {
        Self {
            build_outputs: default_build_outputs(),
        }
    }
}

fn default_build_outputs() -> Vec<BuildOutput> {
    vec![BuildOutput(PathBuf::from("target"))]
}

/// A path relative to a worktree that names something inside it: plain names only, so no
/// absolute path, no `.` or `..`, and never `.git`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct BuildOutput(PathBuf);

impl BuildOutput {
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl TryFrom<String> for BuildOutput {
    type Error = String;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        let path = PathBuf::from(&text);
        let plain = !text.is_empty()
            && path
                .components()
                .all(|component| matches!(component, Component::Normal(_)));
        if !plain || path.starts_with(".git") {
            return Err(format!(
                "build_outputs: {text:?} must be a path inside the worktree, without `..`, `.` or `.git`"
            ));
        }
        Ok(Self(path))
    }
}

/// Reads `path`; a missing file is every default.
pub fn load(path: &Path) -> Result<Config, UsageError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(UsageError(format!("{}: {error}", path.display()))),
    };
    parse(&text, path)
}

/// `path` only names the file in errors.
pub fn parse(text: &str, path: &Path) -> Result<Config, UsageError> {
    // An empty document (or only comments) is null to serde, not an empty mapping.
    if text.lines().all(|line| {
        let line = line.trim();
        line.is_empty() || line.starts_with('#')
    }) {
        return Ok(Config::default());
    }
    serde_norway::from_str(text).map_err(|error| UsageError(format!("{}: {error}", path.display())))
}

/// Ids of plugins that no longer exist. Their `config.yaml` entries are ignored like any entry
/// that names no plugin; the list only lets a check say why.
/// `hooks`: agent state moved into core and the engine adapters (`txi hook`).
/// `history`: the transcript mirror; core archives conversations through the engines.
/// `menu`: renamed `home`; an old `menu` entry no longer configures anything.
pub const RETIRED: &[&str] = &["hooks", "history", "menu"];

/// One `plugins.<id>` entry of `config.yaml`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    #[serde(default = "yes")]
    enabled: bool,
    #[serde(default)]
    settings: Option<serde_json::Value>,
}

fn yes() -> bool {
    true
}

/// The entries as core's activation reads them: every id of the file, on or off, with its
/// settings (`Null` without any).
pub fn entries(plugins: &BTreeMap<String, Entry>) -> BTreeMap<String, crate::plugin::Entry> {
    plugins
        .iter()
        .map(|(id, entry)| {
            let entry = crate::plugin::Entry {
                enabled: entry.enabled,
                settings: entry.settings.clone().unwrap_or_default(),
            };
            (id.clone(), entry)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EngineKind;

    /// The repository's file spells out exactly the built-in defaults of `agent` and `worktree`,
    /// as a missing file has them. It lists no plugin: `tx setup` adds the selected ones when it
    /// seeds the file.
    #[test]
    fn the_installed_file_is_the_built_in_defaults() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/tx-ide/config.yaml");
        let config = load(&path).unwrap();
        assert_eq!(config.agent, AgentDefaults::default());
        assert_eq!(config.worktree, WorktreeSettings::default());
        assert_eq!(config.theme, ThemeSettings::default());
        assert!(config.plugins.is_empty(), "{:?}", config.plugins);
    }

    #[test]
    fn theme_accepts_a_name_or_a_table() {
        let short = parse_text("theme: tokyo-night\n").unwrap();
        assert_eq!(short.theme.name, "tokyo-night");
        assert_eq!(short.theme.status_style, StatusStyle::Rule);
        assert_eq!(short.theme.glyphs, GlyphTier::Unicode);

        let full = parse_text(
            "theme:\n  name: tx-dark\n  status_style: bar\n  glyphs: ascii\n  colors:\n    signal: \"#ffb000\"\n",
        )
        .unwrap();
        assert_eq!(full.theme.name, "tx-dark");
        assert_eq!(full.theme.status_style, StatusStyle::Bar);
        assert_eq!(full.theme.glyphs, GlyphTier::Ascii);
        assert_eq!(
            full.theme.colors.get("signal").map(String::as_str),
            Some("#ffb000")
        );

        let error = parse_text("theme:\n  status_style: ribbon\n").unwrap_err();
        assert!(error.0.starts_with("/c/config.yaml: "), "{error}");
    }

    fn parse_text(text: &str) -> Result<Config, UsageError> {
        parse(text, Path::new("/c/config.yaml"))
    }

    #[test]
    fn an_empty_file_is_every_default() {
        for text in ["", "\n", "# only a comment\n"] {
            let config = parse_text(text).unwrap();
            assert_eq!(config.agent, AgentDefaults::default(), "{text:?}");
            assert!(config.plugins.is_empty(), "{text:?}");
        }
    }

    #[test]
    fn a_missing_file_is_every_default() {
        let config = load(Path::new("/nonexistent/tx/config.yaml")).unwrap();
        assert_eq!(config.agent.engine, EngineKind::CLAUDE);
        assert!(config.agent.roles.is_empty());
    }

    #[test]
    fn agent_keys_left_out_keep_their_defaults() {
        let config = parse_text("agent:\n  model: sonnet\n").unwrap();
        assert_eq!(
            config.agent,
            AgentDefaults {
                model: Some("sonnet".to_owned()),
                ..AgentDefaults::default()
            }
        );
    }

    #[test]
    fn null_model_and_effort_leave_them_to_the_engine() {
        let config = parse_text("agent:\n  model: null\n  effort: null\n").unwrap();
        assert_eq!(config.agent.model, None);
        assert_eq!(config.agent.effort, None);
    }

    #[test]
    fn every_agent_key_is_read() {
        let config =
            parse_text("agent:\n  engine: codex\n  roles: [QA]\n  definition: null\n  model: gpt-5\n  effort: low\n")
                .unwrap();
        assert_eq!(
            config.agent,
            AgentDefaults {
                engine: EngineKind::CODEX,
                roles: vec!["QA".to_owned()],
                definition: None,
                model: Some("gpt-5".to_owned()),
                effort: Some("low".to_owned()),
            }
        );
    }

    #[test]
    fn effort_is_a_number_or_a_word() {
        for (text, effort) in [
            ("agent:\n  effort: 3\n", "3"),
            ("agent:\n  effort: \"3\"\n", "3"),
            ("agent:\n  effort: high\n", "high"),
            ("agent:\n  effort: \"high\"\n", "high"),
        ] {
            let config = parse_text(text).unwrap();
            assert_eq!(config.agent.effort.as_deref(), Some(effort), "{text:?}");
        }
        let error = parse_text("agent:\n  effort: [3]\n").unwrap_err();
        assert!(
            error.0.contains("a number 1-5 or the engine's word"),
            "{error}"
        );
    }

    #[test]
    fn build_outputs_default_to_target() {
        let paths = |config: Config| {
            config
                .worktree
                .build_outputs
                .iter()
                .map(|output| output.path().to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(paths(parse_text("").unwrap()), [PathBuf::from("target")]);
        assert_eq!(
            paths(parse_text("worktree: {}\n").unwrap()),
            [PathBuf::from("target")]
        );
        assert_eq!(
            paths(parse_text("worktree:\n  build_outputs: [node_modules, web/dist]\n").unwrap()),
            [PathBuf::from("node_modules"), PathBuf::from("web/dist")]
        );
        assert!(paths(parse_text("worktree:\n  build_outputs: []\n").unwrap()).is_empty());
    }

    #[test]
    fn build_outputs_leaving_the_worktree_are_refused() {
        for bad in [
            "/tmp",
            "../x",
            "a/../../x",
            ".",
            "./target",
            ".git",
            ".git/x",
            "\"\"",
        ] {
            let error = parse_text(&format!("worktree:\n  build_outputs: [{bad}]\n")).unwrap_err();
            assert!(error.0.starts_with("/c/config.yaml: "), "{bad}: {error}");
            assert!(error.0.contains("build_outputs"), "{bad}: {error}");
        }
    }

    #[test]
    fn unknown_keys_name_the_file_and_the_key() {
        for (text, key) in [("agnet: {}\n", "agnet"), ("agent:\n  modle: x\n", "modle")] {
            let error = parse_text(text).unwrap_err();
            assert!(error.0.starts_with("/c/config.yaml: "), "{error}");
            assert!(error.0.contains(key), "{error}");
        }
    }

    /// `agent.engine` is any engine kind: whether an engine provides it is settled when
    /// something is launched. What cannot be a kind is refused, naming the file.
    #[test]
    fn any_engine_kind_is_accepted_and_what_is_not_one_is_refused() {
        let config = parse_text("agent:\n  engine: kodex\n").unwrap();
        assert_eq!(config.agent.engine.as_str(), "kodex");
        let error = parse_text("agent:\n  engine: Bad Kind\n").unwrap_err();
        assert!(error.0.starts_with("/c/config.yaml: "), "{error}");
        assert!(
            error
                .0
                .contains("engine kind \"Bad Kind\" must match [a-z0-9-]{1,31}"),
            "{error}"
        );
    }

    #[test]
    fn malformed_yaml_names_the_file() {
        let error = parse_text("agent: [1, 2\n").unwrap_err();
        assert!(error.0.starts_with("/c/config.yaml: "), "{error}");
    }

    #[test]
    fn the_activation_gets_every_entry_of_the_file() {
        let config = parse_text(
            "plugins:\n  hello:\n    settings: { greeting: hi }\n  pikcer: { enabled: false }\n  labels: {}\n",
        )
        .unwrap();
        let entries = entries(&config.plugins);
        let seen: Vec<(&str, bool, &serde_json::Value)> = entries
            .iter()
            .map(|(id, entry)| (id.as_str(), entry.enabled, &entry.settings))
            .collect();
        assert_eq!(
            seen,
            [
                ("hello", true, &serde_json::json!({"greeting": "hi"})),
                ("labels", true, &serde_json::Value::Null),
                ("pikcer", false, &serde_json::Value::Null),
            ]
        );
    }

    #[test]
    fn malformed_yaml_in_plugins_is_a_usage_error() {
        let error = parse_text("plugins: [1, 2\n").unwrap_err();
        assert!(error.0.starts_with("/c/config.yaml: "), "{error}");
    }

    #[test]
    fn unknown_keys_on_an_entry_are_refused() {
        let error = parse_text("plugins:\n  labels: { enabledd: true }\n").unwrap_err();
        assert!(error.0.contains("enabledd"), "{error}");
    }
}
