//! The `ide` plugin's updater, `ui attach` and `ui view`: compose the core's server config from
//! the key fragments `tx update` published (on `tx update`) and give the core's own back when
//! the plugin is retired, enter the IDE's server, and open, list or close views from a shell.

use std::io;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use tx_client::theme::{ColorDepth, Polarity, Theme};
use tx_client::tmux::{Ui, UiError};
use tx_client::views::{self, ViewKind, ViewRequest};
use tx_core::core::{DEFAULT_TMUX_CONF, TMUX_CONF_HEADER};
use tx_core::host::bundle::KEYS_INSTALLED;
use tx_core::host::setup::ROLES_DIR;
use tx_core::plugin::manifest::RESOURCE_KINDS;
use tx_core::sdk::TX_BIN_ENV;
use tx_core::updates::Outcome;
use tx_core::{TxCore, Updater, Verb, VerbError, VerbIo, keys};

use crate::view_verb;

const HOME_SESSION: &str = "home";
const IDE_FRAGMENT: &str = "ide";
/// Under the data directory: the key fragments `tx update` published, `<id>.tmux` each.
const KEYS_DIR: &str = "keys";
const FRAGMENT_EXTENSION: &str = "tmux";
const LEGACY_HINT: &str = "legacy bindings may remain; restart the server to drop them";
/// What every key option's name starts with: `@tx-key-<plugin>-<action>`.
const KEY_OPTION_PREFIX: &str = "@tx-key-";
/// The server options the composed config sets in its own first lines (see [`conf_text`]); with
/// `TX_BIN` in the server's environment, what a retire takes back next to the fragments' keys.
/// `@tx-bin` is also how the updater recognises a server whose bindings were never recorded.
const CONF_OPTIONS: [&str; 3] = ["@tx-bin", "@tx-data", "@tx-arguments"];
/// What a retire says: the config is core's again, and what a running server keeps of the IDE.
const RESET: &str = "tmux.conf reset to the core default";
const RESTART_HINT: &str = "restart the server to drop the rest of the IDE's look";
/// The home plugin's verb that puts the home TUI into the view's pane; absent when the plugin
/// is disabled.
const HOME_INSTALL_VERB: &str = "_home-install";
/// The resource kind of a role: `tx update` publishes an `agents/…` name under `<data>/roles/`,
/// since `<data>/agents/` holds the agent records.
const ROLES_RESOURCE: &str = "agents";

/// `@tx-data` locates `bin/` and `keys/`; `@tx-arguments` is what every `tx` a key fragment
/// runs carries (the core's `cli_arguments()`, words split by spaces: `install` refuses unsafe
/// paths). `TX_BIN` in the server's environment is the same `tx` as `@tx-bin`: what a plugin
/// program started from a key, a popup or a pane builds its command lines with.
/// `detach-on-destroy on` detaches the clients of an ending session: a client nested in a
/// view's pane goes, and the pane falls back to a shell. The terminal is kept in tmux by its own
/// `no-detach-on-destroy` flag, which `ui attach` sets.
///
/// `fragments` are `(id, text)` pairs in the order they are run. Each runs through the
/// interpreter its first line names, never as an executable file: on macOS the first direct run
/// of a freshly written executable is assessed by the system one file at a time, 0.4 s alone and
/// seconds when many run at once, while the same file run by an interpreter is not.
///
/// Order: these options and every key option ([`key_options`]), the shipped options
/// (`conf/tmux.conf`, the theme), the key fragments, and last the user's own tmux.conf when it
/// exists (`-q`), so the user's values win on server start and on every reload. `user_conf` is
/// `None` when the config and data directories are one, where `<config>/tmux.conf` is this file
/// itself.
fn conf_text(
    exe: &Path,
    data: &Path,
    arguments: &[String],
    keys: &[(String, String)],
    fragments: &[(&str, &str)],
    user_conf: Option<&Path>,
) -> String {
    let data = data.display();
    let exe = exe.display();
    let arguments = arguments.join(" ");
    let mut text = format!(
        "{TMUX_CONF_HEADER}, written by `tx update`. Re-run to update.\n\
         set -s exit-empty on\n\
         set -g history-limit 50000\n\
         set -g detach-on-destroy on\n\
         set -g @tx-bin \"{exe}\"\n\
         set -g @tx-data \"{data}\"\n\
         set -g @tx-arguments \"{arguments}\"\n\
         set-environment -g TX_BIN \"{exe}\"\n",
    );
    for (option, key) in keys {
        text.push_str(&format!("set -g {option} '{key}'\n"));
    }
    text.push_str(&format!(
        "source-file \"{data}/conf/tmux.conf\"\n\
         source-file \"{data}/conf/theme.tmux\"\n"
    ));
    for (id, fragment) in fragments {
        let interpreter = fragment
            .lines()
            .next()
            .and_then(|line| line.strip_prefix("#!"))
            .map(|words| format!("{} ", words.trim()))
            .unwrap_or_default();
        text.push_str(&format!(
            "run-shell \"{interpreter}{data}/keys/{id}.tmux\"\n"
        ));
    }
    if let Some(user_conf) = user_conf {
        text.push_str(&format!("source-file -q \"{}\"\n", user_conf.display()));
    }
    text
}

/// The tmux option a key fragment reads its key from: `@tx-key-<plugin>-<action>`.
fn key_option(plugin: &str, action: &str) -> String {
    format!("{KEY_OPTION_PREFIX}{plugin}-{action}")
}

/// The key table (the `keys` setting, [`tx_core::keys`]) of the plugin whose fragment is `id`.
fn key_table<'a>(core: &'a TxCore, id: &str) -> Option<&'a serde_json::Value> {
    core.plugin_settings(id)?.get(keys::KEYS)
}

/// The key table of every published fragment's plugin as tmux options, fragment order: the key,
/// or empty for an action the user unbound. Values were checked when the plugins loaded (no
/// quote can break the single-quoted option value).
fn key_options(core: &TxCore, fragments: &[(String, String)]) -> Vec<(String, String)> {
    fragments
        .iter()
        .filter_map(|(id, _)| Some((id, key_table(core, id)?.as_object()?)))
        .flat_map(|(id, table)| {
            table.iter().map(move |(action, key)| {
                (
                    key_option(id, action),
                    key.as_str().unwrap_or_default().to_owned(),
                )
            })
        })
        .collect()
}

/// What a key fragment declares on top-level comment lines: `# keys: action …` (the actions it
/// binds in the prefix table), `# root-keys: action …` (in the root table), `# unset: option …`
/// and `# unhook: hook …`. `none` stands for "nothing". Several lines of one kind add up.
/// [`bound`] turns the actions into the tmux keys the plugin's settings give them, the form the
/// record holds.
#[derive(Debug, Default, PartialEq, Eq)]
struct Declared {
    keys: Vec<String>,
    root: Vec<String>,
    unset: Vec<String>,
    unhook: Vec<String>,
}

fn parse_declarations(text: &str) -> Declared {
    let mut declared = Declared::default();
    for line in text.lines() {
        let (target, rest) = if let Some(rest) = line.strip_prefix("# keys:") {
            (&mut declared.keys, rest)
        } else if let Some(rest) = line.strip_prefix("# root-keys:") {
            (&mut declared.root, rest)
        } else if let Some(rest) = line.strip_prefix("# unset:") {
            (&mut declared.unset, rest)
        } else if let Some(rest) = line.strip_prefix("# unhook:") {
            (&mut declared.unhook, rest)
        } else {
            continue;
        };
        target.extend(
            rest.split_whitespace()
                .filter(|word| *word != "none")
                .map(str::to_owned),
        );
    }
    declared
}

/// `declared` with its actions replaced by the tmux keys `table` (the plugin's `keys` setting)
/// binds them to: an unbound action has none, a modifier family nine. A plugin without a key table
/// declares the tmux keys themselves, which stay as they are.
fn bound(declared: Declared, table: Option<&serde_json::Value>) -> Declared {
    if table.is_none() {
        return declared;
    }
    let keys = |actions: Vec<String>| -> Vec<String> {
        actions
            .iter()
            .filter_map(|action| table?.get(action)?.as_str())
            .flat_map(keys::expand)
            .collect()
    };
    Declared {
        keys: keys(declared.keys),
        root: keys(declared.root),
        ..declared
    }
}

/// The first tmux key two actions would bind in one table, as a message naming both settings:
/// the later fragment would silently take the key from the earlier one. Spellings tmux reads as
/// one key (`c-t`, `C-t`) are one key.
fn key_collision(core: &TxCore, fragments: &[(String, String)]) -> Option<String> {
    let mut taken: Vec<(&str, String, String)> = Vec::new();
    for (id, text) in fragments {
        let Some(table) = key_table(core, id) else {
            continue;
        };
        let declared = parse_declarations(text);
        for (key_table, actions) in [("prefix", &declared.keys), ("root", &declared.root)] {
            for action in actions {
                let Some(key) = table.get(action).and_then(serde_json::Value::as_str) else {
                    continue;
                };
                let setting = format!("plugins.{id}.settings.{}.{action}", keys::KEYS);
                for bound in keys::expand(key) {
                    let spelling = keys::canonical(&bound);
                    if let Some((_, _, other)) = taken
                        .iter()
                        .find(|(table, key, _)| *table == key_table && *key == spelling)
                    {
                        let pressed = if key_table == "prefix" {
                            format!("prefix+{bound}")
                        } else {
                            bound
                        };
                        return Some(format!(
                            "{pressed} is set for both {other} and {setting}: change one, or set \
                             one to null"
                        ));
                    }
                    taken.push((key_table, spelling, setting.clone()));
                }
            }
        }
    }
    None
}

/// `<data>/keys/.installed`: one line per key fragment, tab-separated fields
/// `<id>\tkeys=<k k…>\troot=<k k…>\tunset=<o o…>\tunhook=<h h…>`, the keys as the settings bind
/// them; a group may be empty, and a file from before `root=` existed reads as none.
fn render_installed(entries: &[(String, Declared)]) -> String {
    entries
        .iter()
        .map(|(id, d)| {
            format!(
                "{id}\tkeys={}\troot={}\tunset={}\tunhook={}\n",
                d.keys.join(" "),
                d.root.join(" "),
                d.unset.join(" "),
                d.unhook.join(" ")
            )
        })
        .collect()
}

fn parse_installed(text: &str) -> Vec<(String, Declared)> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let id = fields.next().filter(|id| !id.is_empty())?;
            let mut declared = Declared::default();
            for field in fields {
                let words = |rest: &str| rest.split_whitespace().map(str::to_owned).collect();
                if let Some(rest) = field.strip_prefix("keys=") {
                    declared.keys = words(rest);
                } else if let Some(rest) = field.strip_prefix("root=") {
                    declared.root = words(rest);
                } else if let Some(rest) = field.strip_prefix("unset=") {
                    declared.unset = words(rest);
                } else if let Some(rest) = field.strip_prefix("unhook=") {
                    declared.unhook = words(rest);
                }
            }
            Some((id.to_owned(), declared))
        })
        .collect()
}

/// Paths are spliced into double-quoted tmux config lines and into shell command strings, so
/// anything the config parser or a shell would interpret is refused.
fn is_unsafe_path(path: &Path) -> bool {
    path.to_string_lossy()
        .chars()
        .any(|c| c.is_whitespace() || "\"'$#;`\\".contains(c))
}

/// The socket path in a `$TMUX` value (`<socket>,<pid>,<session>`).
fn tmux_env_socket(env: Option<&str>) -> Option<&str> {
    env.and_then(|value| value.split(',').next())
        .filter(|socket| !socket.is_empty())
}

/// Takes back, in a running server, what a key fragment declared: its prefix and root keys are
/// unbound, its options and its hooks unset.
fn take_back(ui: &Ui, declared: &Declared) {
    for key in &declared.keys {
        ui.quiet(&["unbind", "-T", "prefix", key]);
    }
    for key in &declared.root {
        ui.quiet(&["unbind", "-T", "root", key]);
    }
    for option in &declared.unset {
        ui.quiet(&["set", "-gu", option]);
    }
    for hook in &declared.unhook {
        ui.quiet(&["set-hook", "-gu", hook]);
    }
}

/// The words of `before` that `now` no longer lists; all of them when the fragment is gone.
fn dropped(before: &[String], now: Option<&[String]>) -> Vec<String> {
    before
        .iter()
        .filter(|word| !now.is_some_and(|now| now.contains(word)))
        .cloned()
        .collect()
}

fn fail(io: &mut VerbIo<'_>, message: &str) -> Result<i32, VerbError> {
    let _ = writeln!(io.stderr, "{message}");
    Ok(1)
}

fn usage(io: &mut VerbIo<'_>) -> Result<i32, VerbError> {
    let _ = writeln!(io.stderr, "usage: tx ui attach | view <open|ls|close> …");
    Ok(2)
}

pub struct UiVerb;

impl Verb for UiVerb {
    fn name(&self) -> &'static str {
        "ui"
    }

    fn usage(&self) -> &'static str {
        "ui attach | view <open|ls|close>   enter the IDE's server; manage views"
    }

    fn run(&self, core: &TxCore, io: &mut VerbIo<'_>, argv: &[String]) -> Result<i32, VerbError> {
        match argv {
            [sub] if sub == "attach" => attach(core, io),
            [sub, rest @ ..] if sub == "view" => view_verb::run(core, io, rest),
            _ => usage(io),
        }
    }
}

/// Where `tx update` publishes the resource `name`: `<data>/<name>`, an `agents/…` name under
/// `<data>/roles/`. `None` for a name no resource can have: not `<kind>/<path>` with one of the
/// kinds `tx update` publishes, or holding a `..` segment.
fn published_path(data: &Path, name: &str) -> Option<PathBuf> {
    let (kind, path) = name.split_once('/')?;
    if !RESOURCE_KINDS.contains(&kind) || path.split('/').any(|segment| segment == "..") {
        return None;
    }
    Some(if kind == ROLES_RESOURCE {
        data.join(format!("{ROLES_DIR}/{path}"))
    } else {
        data.join(name)
    })
}

/// `_resource <name>`: a resource as `tx update` published it, whichever plugin it is of, for
/// tests and debugging.
pub struct ResourceVerb;

impl Verb for ResourceVerb {
    fn name(&self) -> &'static str {
        "_resource"
    }

    fn usage(&self) -> &'static str {
        "_resource NAME   print a plugin resource (internal)"
    }

    fn run(&self, core: &TxCore, io: &mut VerbIo<'_>, argv: &[String]) -> Result<i32, VerbError> {
        let [name] = argv else {
            let _ = writeln!(io.stderr, "usage: tx _resource NAME");
            return Ok(2);
        };
        let missing = format!("tx _resource: no such resource: {name}");
        let Some(path) = published_path(&core.config().directories.data, name) else {
            return fail(io, &missing);
        };
        match std::fs::read(&path) {
            Ok(bytes) => {
                io.stdout
                    .write_all(&bytes)
                    .map_err(|e| VerbError::Failed(format!("write: {e}")))?;
                Ok(0)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => fail(io, &missing),
            Err(error) => Err(io_failed(&path)(error)),
        }
    }
}

fn io_failed(path: &Path) -> impl FnOnce(io::Error) -> VerbError {
    let path = path.to_path_buf();
    move |e| VerbError::Failed(format!("{}: {e}", path.display()))
}

/// The key fragments `tx update` published under `<data>/keys`, as `(id, text)` pairs in the
/// order the server config runs them: `ide` first, the others by name.
fn published_fragments(data: &Path) -> Result<Vec<(String, String)>, VerbError> {
    let directory = data.join(KEYS_DIR);
    let entries = match std::fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(io_failed(&directory)(error)),
    };
    let mut fragments = Vec::new();
    for entry in entries {
        let path = entry.map_err(io_failed(&directory))?.path();
        if path
            .extension()
            .is_some_and(|extension| extension == FRAGMENT_EXTENSION)
            && let Some(id) = path.file_stem()
        {
            let text = std::fs::read(&path).map_err(io_failed(&path))?;
            fragments.push((
                id.to_string_lossy().into_owned(),
                String::from_utf8_lossy(&text).into_owned(),
            ));
        }
    }
    fragments.sort_by(|(one, _), (other, _)| {
        (one != IDE_FRAGMENT, one).cmp(&(other != IDE_FRAGMENT, other))
    });
    Ok(fragments)
}

/// The terminal background an attached client reports, else the one recorded the last time a
/// terminal answered (`tx update` before the first attach, a server with no client).
fn known_polarity(ui: &Ui, data: &Path) -> Polarity {
    ui.quiet(&["list-clients", "-F", "#{client_theme}"])
        .1
        .lines()
        .map(Polarity::parse)
        .find(|polarity| *polarity != Polarity::Unknown)
        .unwrap_or_else(|| Polarity::recorded(data))
}

/// Generates `<data>/conf/theme.tmux` and `<data>/conf/theme.lua` from the active configuration,
/// and records a known `polarity` for the screens started later, which cannot ask tmux.
fn write_theme_files(core: &TxCore, polarity: Polarity) -> Result<PathBuf, VerbError> {
    let depth = ColorDepth::detect();
    let theme = Theme::for_core(core, polarity, depth)
        .map_err(|error| VerbError::Failed(error.to_string()))?;
    let data = &core.config().directories.data;
    polarity.record(data).map_err(io_failed(data))?;
    let conf_dir = data.join("conf");
    std::fs::create_dir_all(&conf_dir).map_err(io_failed(&conf_dir))?;
    let tmux_path = conf_dir.join("theme.tmux");
    let lua_path = conf_dir.join("theme.lua");
    std::fs::write(&tmux_path, theme.tmux_conf()).map_err(io_failed(&tmux_path))?;
    std::fs::write(&lua_path, theme.nvim_lua()).map_err(io_failed(&lua_path))?;
    Ok(tmux_path)
}

/// `_theme-apply [dark|light]`: regenerates `<data>/conf/theme.tmux` and `theme.lua`, and sources
/// `theme.tmux` on the running server. tmux's `client-dark-theme` / `client-light-theme` hooks run
/// it with the background the terminal reported.
pub struct ThemeApplyVerb;

impl Verb for ThemeApplyVerb {
    fn name(&self) -> &'static str {
        "_theme-apply"
    }

    fn usage(&self) -> &'static str {
        "_theme-apply [dark|light]   regenerate and source theme.tmux and theme.lua (internal)"
    }

    fn run(&self, core: &TxCore, io: &mut VerbIo<'_>, argv: &[String]) -> Result<i32, VerbError> {
        let ui = Ui::from_core(core);
        let data = &core.config().directories.data;
        let polarity = match argv {
            [] => known_polarity(&ui, data),
            [reported] if reported == "dark" || reported == "light" => Polarity::parse(reported),
            _ => {
                let _ = writeln!(io.stderr, "usage: tx {}", self.usage());
                return Ok(2);
            }
        };
        let tmux_path = write_theme_files(core, polarity)?;
        if ui.quiet(&["list-sessions"]).0 {
            ui.run(&["source-file", &tmux_path.to_string_lossy()])
                .map_err(|error| VerbError::Failed(error.to_string()))?;
        }
        Ok(0)
    }
}

/// The `ide` plugin's part of `tx update`: composes the server config from the key fragments
/// `tx update` published, and reloads a running server. Retired, it gives the config back to
/// core.
pub struct IdeUpdater;

impl Updater for IdeUpdater {
    fn update(&self, core: &TxCore, io: &mut VerbIo<'_>) -> Result<Outcome, VerbError> {
        install(core, io)?;
        Ok(Outcome::default())
    }

    fn retire(&self, core: &TxCore, io: &mut VerbIo<'_>) -> Result<Outcome, VerbError> {
        retire(core, io)?;
        Ok(Outcome::default())
    }
}

/// Takes back what [`install`] generated, once the plugin stopped being active. The record of
/// the fragments the config was composed from, `<data>/keys/.installed`, says there is
/// something to take back: without it nothing is done and nothing printed, so a retire is safe
/// to repeat. With it, `<data>/tmux.conf` is core's default again; a running server loses every
/// key, option and hook the record lists, the options and the `TX_BIN` the composed config
/// itself set, and loads that config; the record goes last, so a retire that failed half way is
/// tried again by the next `tx update`. The server is then a plain core server to the updater:
/// enabling the plugin again finds no trace of an unrecorded install. What the fragments set
/// without declaring it (the theme, User0..8, the attach-to table) stays until the server
/// restarts. The key options (`@tx-key-…`) go with the other options.
fn retire(core: &TxCore, io: &mut VerbIo<'_>) -> Result<(), VerbError> {
    let installed = core.config().directories.data.join(KEYS_INSTALLED);
    let entries = match std::fs::read_to_string(&installed) {
        Ok(text) => parse_installed(&text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(io_failed(&installed)(error)),
    };
    let conf = core.config().tmux_conf_path();
    std::fs::write(&conf, DEFAULT_TMUX_CONF).map_err(io_failed(&conf))?;
    let mut out = format!("{RESET}\n");
    let ui = Ui::from_core(core);
    if ui.quiet(&["list-sessions"]).0 {
        for (_, declared) in &entries {
            take_back(&ui, declared);
        }
        for option in CONF_OPTIONS {
            ui.quiet(&["set", "-gu", option]);
        }
        for option in key_options_set(&ui) {
            ui.quiet(&["set", "-gu", &option]);
        }
        ui.quiet(&["set-environment", "-gu", TX_BIN_ENV]);
        ui.run(&["source-file", &conf.to_string_lossy()])
            .map_err(|e| VerbError::Failed(e.to_string()))?;
        out.push_str(RESTART_HINT);
        out.push('\n');
    }
    std::fs::remove_file(&installed).map_err(io_failed(&installed))?;
    io.stdout
        .write_all(out.as_bytes())
        .map_err(|e| VerbError::Failed(format!("write: {e}")))
}

/// The key options ([`key_option`]) a running server holds.
fn key_options_set(ui: &Ui) -> Vec<String> {
    ui.quiet(&["show-options", "-g"])
        .1
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter(|option| option.starts_with(KEY_OPTION_PREFIX))
        .map(str::to_owned)
        .collect()
}

fn install(core: &TxCore, io: &mut VerbIo<'_>) -> Result<(), VerbError> {
    // Canonical: the config holds one spelling of the binary, whatever way `tx` was invoked.
    let exe = tx_core::sdk::tx_bin()
        .canonicalize()
        .map_err(|e| VerbError::Failed(format!("cannot resolve the tx binary: {e}")))?;
    let config = core.config();
    let data = &config.directories.data;
    let arguments = config.cli_arguments();
    let user_conf = config.directories.config.join(tx_core::config::CONF_FILE);
    if let Some(path) = std::iter::once(exe.as_path())
        .chain(arguments.iter().map(Path::new))
        .chain([user_conf.as_path()])
        .find(|path| is_unsafe_path(path))
    {
        return Err(VerbError::Failed(format!(
            "path contains unsafe characters: {}",
            path.display()
        )));
    }
    let texts = published_fragments(data)?;
    if let Some(collision) = key_collision(core, &texts) {
        return Err(VerbError::Failed(collision));
    }
    let installed_text = std::fs::read_to_string(data.join(KEYS_INSTALLED)).ok();
    let previous = installed_text.as_deref().map(parse_installed);
    // A record from before root keys were recorded cannot say which ones to unbind.
    let records_root_keys = installed_text
        .as_deref()
        .is_none_or(|text| text.is_empty() || text.contains("\troot="));
    let fragments: Vec<(String, Declared)> = texts
        .iter()
        .map(|(id, text)| {
            (
                id.clone(),
                bound(parse_declarations(text), key_table(core, id)),
            )
        })
        .collect();
    let run_order: Vec<(&str, &str)> = texts
        .iter()
        .map(|(id, text)| (id.as_str(), text.as_str()))
        .collect();
    let conf = core.config().tmux_conf_path();
    let ui = Ui::from_core(core);
    let server_running = ui.quiet(&["list-sessions"]).0;
    write_theme_files(core, known_polarity(&ui, data))?;
    let text = conf_text(
        &exe,
        data,
        &arguments,
        &key_options(core, &texts),
        &run_order,
        (user_conf != conf).then_some(user_conf.as_path()),
    );
    std::fs::write(&conf, text).map_err(io_failed(&conf))?;
    // Recorded before the reload: a reload that fails in the user's own file has still applied
    // everything before it, and the next run must know what is bound.
    let installed = data.join(KEYS_INSTALLED);
    std::fs::write(&installed, render_installed(&fragments)).map_err(io_failed(&installed))?;

    let mut out = format!("{}\n", conf.display());
    if server_running {
        let legacy_conf_loaded = previous.is_none() && {
            let (ok, value) = ui.quiet(&["show-option", "-gv", "@tx-bin"]);
            ok && !value.trim().is_empty()
        };
        for (id, before) in previous.iter().flatten() {
            let now = fragments.iter().find(|(now, _)| now == id).map(|(_, d)| d);
            take_back(
                &ui,
                &Declared {
                    keys: dropped(&before.keys, now.map(|d| d.keys.as_slice())),
                    root: dropped(&before.root, now.map(|d| d.root.as_slice())),
                    unset: dropped(&before.unset, now.map(|d| d.unset.as_slice())),
                    unhook: dropped(&before.unhook, now.map(|d| d.unhook.as_slice())),
                },
            );
        }
        ui.run(&["source-file", &conf.to_string_lossy()])
            .map_err(|e| VerbError::Failed(e.to_string()))?;
        out.push_str("reloaded\n");
        if (previous.is_none() && legacy_conf_loaded) || !records_root_keys {
            out.push_str(LEGACY_HINT);
            out.push('\n');
        }
    }
    io.stdout
        .write_all(out.as_bytes())
        .map_err(|e| VerbError::Failed(format!("write: {e}")))?;
    Ok(())
}

/// Hands `home` to the `home` plugin's floor (`_home-install`) when the plugin is active: the
/// pane keeps the home TUI, which starts the assistant itself. Its stderr is passed through; a
/// failure is reported and never stops the attach. Without the plugin the home is a plain shell:
/// a floor the plugin left before it was disabled is taken apart.
fn install_floor(core: &TxCore, ui: &Ui, io: &mut VerbIo<'_>) {
    let Some(verb) = core.verb(HOME_INSTALL_VERB) else {
        if let Err(error) = remove_floor(ui) {
            let _ = writeln!(io.stderr, "tx ui attach: home: {error}");
        }
        return;
    };
    let mut stdin = io::empty();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let result = verb.run(
        core,
        &mut VerbIo {
            stdin: &mut stdin,
            stdout: &mut stdout,
            stderr: &mut stderr,
        },
        &[HOME_SESSION.to_owned()],
    );
    let _ = io.stderr.write_all(&stderr);
    if let Err(error) = result {
        let _ = writeln!(io.stderr, "tx ui attach: home: {error}");
    }
}

/// The floor pane of `home` back to tmux's defaults: its `pane-died` hook would run a verb that
/// is gone, and `remain-on-exit` would keep it dead. A dead one gets the shell.
fn remove_floor(ui: &Ui) -> Result<(), UiError> {
    for pane in views::home_panes(ui, HOME_SESSION)?
        .iter()
        .filter(|pane| pane.floor)
    {
        let id = pane.id.as_str();
        let steps: [&[&str]; 4] = [
            &["set-hook", "-pu", "-t", id, "pane-died"],
            &["set-option", "-p", "-t", id, "remain-on-exit", "off"],
            &["set-option", "-pu", "-t", id, "remain-on-exit-format"],
            &["set-option", "-pu", "-t", id, views::HOME_FLOOR_OPTION],
        ];
        for step in steps {
            ui.run(step)?;
        }
        if pane.dead {
            ui.respawn_pane(id, &views::default_shell())?;
        }
    }
    Ok(())
}

/// Creates the `home` view on the core's server unless it is already there: a shell in a window
/// named `home`, with the status bar and the pane-border labels on.
fn ensure_home(ui: &Ui) -> Result<(), VerbError> {
    if ui.has_session(HOME_SESSION) {
        return Ok(());
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_owned());
    views::open(
        ui,
        &ViewRequest {
            name: HOME_SESSION,
            kind: ViewKind::Home,
            program: &views::default_shell(),
            arguments: &[],
            working_directory: Path::new(&home),
            tags: &[],
            parent_id: None,
            bound: None,
        },
    )
    .map_err(|e| VerbError::Failed(e.to_string()))?;
    let target = format!("={HOME_SESSION}:");
    let steps: [&[&str]; 3] = [
        &["set-option", "-t", &target, "status", "on"],
        &[
            "set-window-option",
            "-t",
            &target,
            "pane-border-status",
            "top",
        ],
        &["rename-window", "-t", &target, HOME_SESSION],
    ];
    for step in steps {
        ui.run(step).map_err(|e| VerbError::Failed(e.to_string()))?;
    }
    Ok(())
}

/// The terminal's client flag: when its session ends it moves to another session instead of
/// leaving tmux (`detach-on-destroy on` detaches every other client).
const TERMINAL_FLAG: &str = "no-detach-on-destroy";

fn attach(core: &TxCore, io: &mut VerbIo<'_>) -> Result<i32, VerbError> {
    let ui = Ui::from_core(core);
    ensure_home(&ui)?;
    // Every attach: a home from an older install, or one whose floor was lost, gets it back.
    install_floor(core, &ui, io);
    let env = std::env::var("TMUX").ok();
    match tmux_env_socket(env.as_deref()) {
        Some(socket) if Path::new(socket) == core.socket_path() => {
            let terminal = ui
                .terminal_client()
                .map_err(|e| VerbError::Failed(e.to_string()))?;
            ui.run(&["refresh-client", "-t", &terminal, "-f", TERMINAL_FLAG])
                .and_then(|_| ui.switch_client(HOME_SESSION))
                .map_err(|e| VerbError::Failed(e.to_string()))?;
            return Ok(0);
        }
        Some(socket) => {
            let _ = writeln!(
                io.stderr,
                "tx ui attach: nesting inside another tmux (socket {socket}); its prefix wins \u{2014} press the prefix twice to reach the IDE's keys"
            );
        }
        None => {}
    }
    let _ = io.stdout.flush();
    let error = ui
        .command(&[
            "attach-session",
            "-f",
            TERMINAL_FLAG,
            "-t",
            &format!("={HOME_SESSION}"),
        ])
        .stdin(Stdio::inherit())
        .env_remove("TMUX")
        .exec();
    Err(VerbError::Failed(format!("could not run tmux: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conf_text_is_exact() {
        assert_eq!(
            conf_text(
                Path::new("/opt/tx"),
                Path::new("/s/data"),
                &words(&["--data-dir", "/s/data", "--run-dir", "/r"]),
                &[
                    (key_option("kill", "kill"), "X".to_owned()),
                    (key_option("labels", "sessions"), String::new()),
                ],
                &[
                    ("ide", "#!/usr/bin/env bash\n# keys: none\n"),
                    ("kill", "#!/bin/sh\n"),
                    ("labels", "# no interpreter line\n"),
                ],
                Some(Path::new("/s/config/tmux.conf")),
            ),
            "# tx-core server config, written by `tx update`. Re-run to update.\n\
             set -s exit-empty on\n\
             set -g history-limit 50000\n\
             set -g detach-on-destroy on\n\
             set -g @tx-bin \"/opt/tx\"\n\
             set -g @tx-data \"/s/data\"\n\
             set -g @tx-arguments \"--data-dir /s/data --run-dir /r\"\n\
             set-environment -g TX_BIN \"/opt/tx\"\n\
             set -g @tx-key-kill-kill 'X'\n\
             set -g @tx-key-labels-sessions ''\n\
             source-file \"/s/data/conf/tmux.conf\"\n\
             source-file \"/s/data/conf/theme.tmux\"\n\
             run-shell \"/usr/bin/env bash /s/data/keys/ide.tmux\"\n\
             run-shell \"/bin/sh /s/data/keys/kill.tmux\"\n\
             run-shell \"/s/data/keys/labels.tmux\"\n\
             source-file -q \"/s/config/tmux.conf\"\n"
        );
        assert!(
            !conf_text(Path::new("/x"), Path::new("/d"), &[], &[], &[], None).contains("-q"),
            "no user conf: the config directory is this file's own"
        );
        assert!(DEFAULT_TMUX_CONF.starts_with(TMUX_CONF_HEADER));
    }

    #[test]
    fn a_fragment_runs_through_the_interpreter_of_its_first_line() {
        let line = |text| {
            let conf = conf_text(
                Path::new("/x"),
                Path::new("/d"),
                &[],
                &[],
                &[("a", text)],
                None,
            );
            conf.lines().last().unwrap().to_owned()
        };
        assert_eq!(
            line("#!/usr/bin/env bash\necho"),
            "run-shell \"/usr/bin/env bash /d/keys/a.tmux\""
        );
        assert_eq!(line("#!/bin/sh\n"), "run-shell \"/bin/sh /d/keys/a.tmux\"");
        assert_eq!(line("#!/bin/sh"), "run-shell \"/bin/sh /d/keys/a.tmux\"");
        assert_eq!(line("echo\n#!/bin/sh\n"), "run-shell \"/d/keys/a.tmux\"");
        assert_eq!(line(""), "run-shell \"/d/keys/a.tmux\"");
    }

    #[test]
    fn the_fragments_are_the_published_files_ide_first_then_by_name() {
        let data = tempfile::tempdir().unwrap();
        assert!(published_fragments(data.path()).unwrap().is_empty());
        let keys = data.path().join(KEYS_DIR);
        std::fs::create_dir_all(&keys).unwrap();
        for (name, text) in [
            ("kill.tmux", "# keys: X\n"),
            ("ide.tmux", "#!/usr/bin/env bash\n"),
            ("artifacts.tmux", "# keys: a\n"),
            // Not fragments: the updater's own record, a file left half written, a stray one.
            (".installed", "ide\tkeys=\tunset=\tunhook=\n"),
            (".kill.tmux.123.tmp", "half\n"),
            ("notes.txt", "x\n"),
        ] {
            std::fs::write(keys.join(name), text).unwrap();
        }
        assert_eq!(
            published_fragments(data.path()).unwrap(),
            [
                ("ide".to_owned(), "#!/usr/bin/env bash\n".to_owned()),
                ("artifacts".to_owned(), "# keys: a\n".to_owned()),
                ("kill".to_owned(), "# keys: X\n".to_owned()),
            ]
        );
    }

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn declarations_are_parsed_from_comment_lines() {
        let text = "#!/usr/bin/env bash\n# keys: open\n# root-keys: up\n# unset: a b\n\
                    # unhook: c\nset -u\necho '# keys: ignored'\n  # keys: indented is prose\n";
        assert_eq!(
            parse_declarations(text),
            Declared {
                keys: words(&["open"]),
                root: words(&["up"]),
                unset: words(&["a", "b"]),
                unhook: words(&["c"]),
            }
        );
        assert_eq!(
            parse_declarations("# keys: none\n"),
            Declared::default(),
            "none is no key"
        );
        assert_eq!(
            parse_declarations("# keys: X\n# keys: e t\n").keys,
            words(&["X", "e", "t"])
        );
        assert_eq!(parse_declarations(""), Declared::default());
    }

    #[test]
    fn declared_actions_bind_the_keys_of_the_settings() {
        let table = serde_json::json!({"open": "p", "up": "C-k", "off": null, "pane": "M-"});
        let declared = Declared {
            keys: words(&["open", "up", "off"]),
            root: words(&["up", "pane"]),
            unset: words(&["a"]),
            unhook: Vec::new(),
        };
        let bound = bound(declared, Some(&table));
        assert_eq!(bound.keys, words(&["p", "C-k"]));
        assert_eq!(bound.root[..2], words(&["C-k", "M-1"]));
        assert_eq!(bound.root.len(), 10);
        assert_eq!(bound.unset, words(&["a"]));
        assert_eq!(
            super::bound(parse_declarations("# keys: H\n"), None).keys,
            words(&["H"]),
            "without a key table the declared words are the keys"
        );
    }

    #[test]
    fn installed_file_round_trips() {
        let entries = vec![
            ("ide".to_owned(), Declared::default()),
            (
                "labels".to_owned(),
                Declared {
                    keys: words(&["s"]),
                    root: Vec::new(),
                    unset: words(&["pane-border-format"]),
                    unhook: words(&["after-new-window"]),
                },
            ),
            (
                "kill".to_owned(),
                Declared {
                    keys: words(&["X", "C-x"]),
                    root: words(&["C-k"]),
                    ..Declared::default()
                },
            ),
        ];
        let text = render_installed(&entries);
        assert_eq!(
            text,
            "ide\tkeys=\troot=\tunset=\tunhook=\n\
             labels\tkeys=s\troot=\tunset=pane-border-format\tunhook=after-new-window\n\
             kill\tkeys=X C-x\troot=C-k\tunset=\tunhook=\n"
        );
        assert_eq!(parse_installed(&text), entries);
        assert_eq!(
            parse_installed("labels\tkeys=s\tunset=x\tunhook=\n")[0]
                .1
                .root,
            Vec::<String>::new(),
            "a file from before root= has no root keys"
        );
        assert!(parse_installed("").is_empty());
        assert!(parse_installed("\n\tkeys=a\n").is_empty());
    }

    /// A core on directories of its own, with no server on its socket.
    fn core() -> (tempfile::TempDir, TxCore) {
        let dir = tempfile::Builder::new()
            .prefix("tx")
            .tempdir_in("/tmp")
            .unwrap();
        let core = TxCore::open(tx_core::CoreConfig::new(tx_core::Directories::under(
            dir.path().join("state"),
        )))
        .unwrap();
        (dir, core)
    }

    /// What the updater's `retire` printed.
    fn retired(core: &TxCore) -> String {
        let mut stdout = Vec::new();
        IdeUpdater
            .retire(
                core,
                &mut VerbIo {
                    stdin: &mut io::empty(),
                    stdout: &mut stdout,
                    stderr: &mut Vec::new(),
                },
            )
            .unwrap();
        String::from_utf8(stdout).unwrap()
    }

    #[test]
    fn retire_gives_the_config_back_to_core_and_drops_the_record() {
        let (_dir, core) = core();
        let conf = core.config().tmux_conf_path();
        let installed = core.config().directories.data.join(KEYS_INSTALLED);
        std::fs::write(&conf, "set -g @tx-bin \"/opt/tx\"\n").unwrap();
        std::fs::create_dir_all(installed.parent().unwrap()).unwrap();
        std::fs::write(&installed, "ide\tkeys=C-h\tunset=\tunhook=\n").unwrap();
        // No server runs: nothing to unbind, and nothing to restart.
        assert_eq!(retired(&core), format!("{RESET}\n"));
        assert_eq!(std::fs::read_to_string(&conf).unwrap(), DEFAULT_TMUX_CONF);
        assert!(!installed.exists());
    }

    #[test]
    fn retire_without_the_record_does_and_says_nothing() {
        let (_dir, core) = core();
        let conf = core.config().tmux_conf_path();
        std::fs::write(&conf, "# the operator's own\n").unwrap();
        assert_eq!(retired(&core), "");
        assert_eq!(
            std::fs::read_to_string(&conf).unwrap(),
            "# the operator's own\n"
        );
    }

    #[test]
    fn a_resource_is_published_under_its_name_and_a_role_under_roles() {
        let data = Path::new("/s/data");
        let published = |name| published_path(data, name);
        assert_eq!(
            published("keys/ide.tmux"),
            Some(PathBuf::from("/s/data/keys/ide.tmux"))
        );
        assert_eq!(
            published("nvim/lua/tx-ask.lua"),
            Some(PathBuf::from("/s/data/nvim/lua/tx-ask.lua"))
        );
        assert_eq!(
            published("agents/COMMON.md"),
            Some(PathBuf::from("/s/data/roles/COMMON.md"))
        );
        // Never outside the directory of its kind.
        assert_eq!(
            published("agents//etc/passwd"),
            Some(PathBuf::from("/s/data/roles//etc/passwd"))
        );
        for name in [
            "",
            "tmux.conf",
            "nope/x",
            "roles/COMMON.md",
            "/etc/passwd",
            "keys/../tmux.conf",
            "agents/../../x",
        ] {
            assert_eq!(published(name), None, "{name:?}");
        }
    }

    #[test]
    fn unsafe_path_predicate() {
        for bad in [
            "/a b/x", "/a\tb", "/a\"b", "/a'b", "/a$b", "/a#b", "/a;b", "/a`b", "/a\\b", "/a\nb",
        ] {
            assert!(is_unsafe_path(Path::new(bad)), "{bad:?}");
        }
        for good in [
            "/opt/tx",
            "/Users/x/Rust-Roverprojects/poc_1/a.b",
            "/s/state",
        ] {
            assert!(!is_unsafe_path(Path::new(good)), "{good:?}");
        }
    }

    #[test]
    fn tmux_env_socket_is_the_text_before_the_first_comma() {
        assert_eq!(tmux_env_socket(Some("/a/b.sock,12,0")), Some("/a/b.sock"));
        assert_eq!(tmux_env_socket(Some("/a/b.sock")), Some("/a/b.sock"));
        assert_eq!(tmux_env_socket(Some("")), None);
        assert_eq!(tmux_env_socket(Some(",1,0")), None);
        assert_eq!(tmux_env_socket(None), None);
    }
}
