//! tx-core plugin `picker`: the fzf session picker (prefix+t). It lists the running agents and
//! the views but the home base in one flat list, newest activity first, without the roles the
//! TAB panel hides. `_list` is its feed (fzf re-runs it to reload), `pick` is the interactive
//! loop with its vim normal and search modes. A pick is shown as on every list of agents
//! ([`Ui::show_session`]): Enter focuses it, `a` attaches it to a pane, `p` opens it in a popup.
//! `_picker-key`, `_roles`, `_toggle-hide` and `_roles-panel` serve the fzf binds. Port of main's `tx attach` without `--host`/`--all` and without the digit
//! picks. Publishes `keys/picker.tmux`.

mod picker_filter;
mod picker_keys;

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use tx_core::events::now;
use tx_core::{Agent, ListFilter, Plugin, PluginError, Registry, TxCore, Verb, VerbError, VerbIo};

use tx_client::render::{
    BOLD, LOCATION_W, PickerEntry, RESET, ROLE_W, Row, listed, picker_namew, picker_rows, role_of,
};
use tx_client::show::Show;
use tx_client::theme::{Role, active};
use tx_client::tmux::Ui;
use tx_client::views::{self, View};

use picker_filter::{chips_line, hidden_path, panel_legend, panel_rows, read_hidden};
use picker_keys::{
    NORMAL_PROMPT, SEARCH_PROMPT, ignored_keys, normal_footer, search_footer, toggle_normal_keys,
};

pub const PLUGIN_ID: &str = "picker";

#[derive(Clone, Copy, Debug, Default)]
pub struct PickerPlugin;

impl Plugin for PickerPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn register(&self, r: &mut Registry) -> Result<(), PluginError> {
        r.verb(Arc::new(List));
        r.verb(Arc::new(Pick));
        r.verb(Arc::new(PickerKey));
        r.verb(Arc::new(Roles));
        r.verb(Arc::new(ToggleHide));
        r.verb(Arc::new(RolesPanel));
        Ok(())
    }
}

const DEFAULT_NAMEW: i64 = 18;
const DEFAULT_COLS: i64 = 80;
const RETRY_PAUSE: Duration = Duration::from_millis(1200);

const PICK_USAGE: &str = "\
usage: tx pick [-f|--filter QUERY]
(--host and --all are not supported)
";

/// `NAMEW` as the picker exports it: ASCII digits only, else 18.
fn namew_from(raw: Option<&str>) -> i64 {
    raw.filter(|raw| !raw.is_empty() && raw.bytes().all(|byte| byte.is_ascii_digit()))
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(DEFAULT_NAMEW)
}

/// Terminal width: the controlling tty, else `COLUMNS`, else 80.
pub fn detect_term_cols() -> i64 {
    tty_columns()
        .or_else(|| {
            std::env::var("COLUMNS")
                .ok()
                .and_then(|raw| raw.trim().parse().ok())
                .filter(|columns| *columns > 0)
        })
        .unwrap_or(DEFAULT_COLS)
}

fn tty_columns() -> Option<i64> {
    let tty = File::open("/dev/tty").ok()?;
    // SAFETY: TIOCGWINSZ fills a winsize we own; the fd is valid for the call.
    let mut size: libc::winsize = unsafe { std::mem::zeroed() };
    let status = unsafe { libc::ioctl(tty.as_raw_fd(), libc::TIOCGWINSZ, &mut size) };
    (status == 0 && size.ws_col > 0).then_some(i64::from(size.ws_col))
}

/// Which binds of other plugins' verbs the picker offers: `t` and ctrl-t need `_edit-tag`, `d`
/// and `y` need `kill`; without them those keys are ignored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binds {
    pub edit_tag: bool,
    pub kill: bool,
}

impl Binds {
    fn of(core: &TxCore) -> Self {
        Self {
            edit_tag: core.verb("_edit-tag").is_some(),
            kill: core.verb("kill").is_some(),
        }
    }
}

/// The fzf argv, a port of main's `Attach::fzf_opts`: normal mode (`❯`) unless a query is
/// given, search mode (`/`) then. `{1}`, `{2}`, `$TX_ARM_FILE` and `$FZF_PORT` stay literal for
/// fzf and the shells it spawns; the file reaches them through the child's environment. Enter,
/// `a` and `p` (alt-a and alt-p while searching, where they type) print the name of their
/// [`Show`] ahead of the selection. `hidden` seeds the chips line.
///
/// Two command prefixes, each a program followed by the core's `cli_arguments()` (see
/// `own_command` and `tx_command`): `own` runs the picker's own verbs (the reload, the key
/// handler, the roles) as children of fzf, one process each; `tx` runs what belongs to other
/// plugins, `_edit-tag` and `kill`, through `tx`.
pub fn fzf_args(
    own: &str,
    tx: &str,
    socket: &Path,
    namew: i64,
    query: &str,
    hidden: &[&str],
    binds: Binds,
) -> Vec<String> {
    let namew = namew.max(0) as usize;
    let socket = socket.display();
    let reload = format!("reload-sync({own} _list)");
    let edit_tag = format!("{tx} _edit-tag {{1}}");
    let chips = format!("{own} _roles --chips");
    let show = |show: Show| format!("print({})+accept", show.name());
    let header_cols = format!(
        "{:<namew$}   {:<LOCATION_W$} {:<7} {:<6} {:<ROLE_W$} TAGS",
        "NAME", "LOCATION", "STARTED", "IDLE", "ROLE"
    );
    let theme = active();
    let accent_ansi = theme.ansi_fg(Role::Accent);
    let fg_ansi = theme.ansi_fg(Role::Text);
    let signal_ansi = theme.ansi_fg(Role::Signal);
    let needs_you = theme.glyphs.needs_you;
    let focus_cmd = format!(
        "printf '{accent_ansi}{BOLD}%s{RESET} {fg_ansi}{BOLD}%s{RESET}\\n' {{1}} {{2}}; {chips}; \
         printf '%s' '{header_cols}'"
    );
    let arm_cmd = format!(
        "printf '{signal_ansi}{BOLD} {needs_you}  Kill \"%s\"? [y/N]{RESET}\\n' {{1}}; {chips}; \
         printf '%s' '{header_cols}'"
    );
    let unbind_normal = toggle_normal_keys("unbind");
    let (prompt, start_mode, footer) = if query.is_empty() {
        (NORMAL_PROMPT, String::new(), normal_footer())
    } else {
        (SEARCH_PROMPT, format!("+{unbind_normal}"), search_footer())
    };
    let enter_search = format!(
        "change-prompt({SEARCH_PROMPT})+{unbind_normal}+execute-silent(: >\"$TX_ARM_FILE\")+transform-header({focus_cmd})+change-footer:{}",
        search_footer()
    );
    let mut args: Vec<String> = [
        "fzf".to_owned(),
        "--exact".to_owned(),
        "--ansi".to_owned(),
        format!("--prompt={prompt}"),
        "--height=100%".to_owned(),
        "--reverse".to_owned(),
        "--delimiter=\t".to_owned(),
        "--with-nth=5..".to_owned(),
        "--listen".to_owned(),
        "--track".to_owned(),
        "--with-shell=sh -c".to_owned(),
        theme.fzf_color_arg(false),
        format!("--header={}\n{header_cols}", chips_line(hidden)),
        format!("--footer={footer}"),
        format!("--query={query}"),
        format!(
            "--bind=start:execute-silent(( while sleep 1; do \
             curl -fsS -XPOST \"localhost:$FZF_PORT\" -d \"{reload}\" >/dev/null 2>&1 || exit 0; \
             done ) &){start_mode}"
        ),
        format!("--bind=ctrl-r:{reload}"),
        format!("--bind=r:{reload}"),
        format!("--bind=enter:{}", show(Show::Focus)),
        format!("--bind=a:{}", show(Show::AttachTo)),
        format!("--bind=alt-a:{}", show(Show::AttachTo)),
        format!("--bind=p:{}", show(Show::Popup)),
        format!("--bind=alt-p:{}", show(Show::Popup)),
        format!("--bind=focus:transform-header({focus_cmd})+execute-silent(: >\"$TX_ARM_FILE\")"),
        // The roles panel runs in the picker's own terminal (fzf hands it over for the
        // duration): a tmux popup cannot nest inside the prefix+t picker popup.
        format!("--bind=tab:execute({own} _roles-panel)+{reload}+transform-header({focus_cmd})"),
        format!(
            "--bind=click-header:execute-silent({own} _toggle-hide --click \"$FZF_CLICK_HEADER_COLUMN\" \"$FZF_CLICK_HEADER_WORD\")+{reload}+transform-header({focus_cmd})"
        ),
    ]
    .into();
    if binds.edit_tag {
        let edit_tag_bind = format!(
            "execute(tmux -S {socket} display-popup -E -h 4 -w 60% \"{edit_tag}\")+{reload}"
        );
        args.push(format!("--bind=ctrl-t:{edit_tag_bind}"));
        args.push(format!("--bind=t:{edit_tag_bind}"));
    } else {
        args.push("--bind=t:ignore".to_owned());
    }
    if binds.kill {
        args.push(format!(
            "--bind=d:execute-silent(printf '%s' {{1}} >\"$TX_ARM_FILE\")+transform-header({arm_cmd})"
        ));
        // `y` stays bound in normal mode (an unbound key would type), so it checks the arm.
        args.push(format!(
            "--bind=y:execute-silent([ -s \"$TX_ARM_FILE\" ] && {tx} kill {{1}} >/dev/null 2>&1; : >\"$TX_ARM_FILE\")+{reload}+transform-header({focus_cmd})"
        ));
    } else {
        args.push("--bind=d:ignore,y:ignore".to_owned());
    }
    args.extend([
        format!("--bind=n:transform-header({focus_cmd})+execute-silent(: >\"$TX_ARM_FILE\")"),
        "--bind=j:down,k:up,g:first,G:last,q:abort".to_owned(),
        "--bind=ctrl-d:half-page-down,ctrl-u:half-page-up".to_owned(),
        format!("--bind=/:{enter_search}"),
        format!("--bind=i:{enter_search}"),
        format!("--bind=esc:transform({own} _picker-key esc)"),
    ]);
    args.extend(
        ignored_keys()
            .iter()
            .map(|key| format!("--bind={key}:ignore")),
    );
    args
}

/// What the picker lists: reconciled with tmux and with the views whose agent is gone closed,
/// the running agents and every view but the home base, less the hidden roles.
struct Listing {
    agents: Vec<Agent>,
    views: Vec<View>,
    hidden: Vec<&'static str>,
}

impl Listing {
    fn gather(core: &TxCore, ui: &Ui) -> Result<Self, VerbError> {
        core.refresh_all()?;
        views::close_orphans(ui, core).map_err(ui_failed)?;
        let ids = core.list_agents(&ListFilter::new().running())?;
        let agents = ids
            .iter()
            .map(|id| core.get_agent(id).map_err(VerbError::from))
            .collect::<Result<_, _>>()?;
        let views = views::list(ui).map_err(ui_failed)?;
        let hidden = read_hidden(&hidden_path(core)).map_err(hidden_failed)?;
        Ok(Listing {
            agents,
            views,
            hidden,
        })
    }

    fn entries(&self) -> impl Iterator<Item = PickerEntry<'_>> {
        self.agents
            .iter()
            .map(PickerEntry::Agent)
            .chain(self.views.iter().map(PickerEntry::View))
            .filter(|entry| listed(*entry) && !self.hidden.contains(&role_of(*entry)))
    }

    /// The widest NAME cell.
    fn longest_name(&self) -> i64 {
        self.entries()
            .map(|entry| entry.name().chars().count() as i64)
            .max()
            .unwrap_or(0)
    }

    /// The picker feed, newline-terminated.
    fn feed(&self, ui: &Ui, namew: i64) -> String {
        let targets = ui.live_targets();
        let attachments = ui.attachment_map();
        let rows: Vec<Row<'_>> = self
            .entries()
            .map(|entry| Row {
                entry,
                target: targets.get(entry.name()).map_or("", String::as_str),
                attached: attachments
                    .iter()
                    .find(|(name, _)| name == entry.name())
                    .map_or(&[][..], |(_, locations)| locations.as_slice()),
            })
            .collect();
        format!("{}\n", picker_rows(&rows, namew, now()))
    }
}

pub struct List;

impl Verb for List {
    fn name(&self) -> &'static str {
        "_list"
    }

    fn usage(&self) -> &'static str {
        "_list   picker feed (internal)"
    }

    fn run(&self, core: &TxCore, io: &mut VerbIo<'_>, _argv: &[String]) -> Result<i32, VerbError> {
        let namew = namew_from(std::env::var("NAMEW").ok().as_deref());
        let ui = Ui::from_core(core);
        let text = Listing::gather(core, &ui)?.feed(&ui, namew);
        io.stdout
            .write_all(text.as_bytes())
            .map_err(|e| VerbError::Failed(format!("write: {e}")))?;
        Ok(0)
    }
}

/// `None` is a usage error; else the query.
fn parse_pick(argv: &[String]) -> Option<String> {
    match argv {
        [] => Some(String::new()),
        [flag, query] if flag == "-f" || flag == "--filter" => Some(query.clone()),
        _ => None,
    }
}

pub struct Pick;

impl Verb for Pick {
    fn name(&self) -> &'static str {
        "pick"
    }

    fn usage(&self) -> &'static str {
        "pick [-f QUERY]   fzf session picker (prefix+t)"
    }

    fn run(&self, core: &TxCore, io: &mut VerbIo<'_>, argv: &[String]) -> Result<i32, VerbError> {
        let Some(query) = parse_pick(argv) else {
            let _ = write!(io.stderr, "{PICK_USAGE}");
            return Ok(2);
        };
        let ui = Ui::from_core(core);
        let namew = picker_namew(
            detect_term_cols(),
            Listing::gather(core, &ui)?.longest_name(),
        );
        // The two-press kill's arm file; removed on drop.
        let arm_file = tempfile::Builder::new()
            .prefix("tx-kill-arm.")
            .rand_bytes(8)
            .tempfile()
            .map_err(|e| VerbError::Failed(format!("arm file: {e}")))?
            .into_temp_path();
        let hidden = read_hidden(&hidden_path(core)).map_err(hidden_failed)?;
        let args = fzf_args(
            &own_command(core)?,
            &tx_command(core),
            core.socket_path(),
            namew,
            &query,
            &hidden,
            Binds::of(core),
        );
        loop {
            let text = Listing::gather(core, &ui)?.feed(&ui, namew);
            let Some(output) = select(&args, namew, &arm_file, &text)? else {
                return Ok(0);
            };
            let (show, selection) = split_show(&output);
            // A pick on an empty list prints only the show line: nothing picked.
            if selection.trim().is_empty() {
                return Ok(0);
            }
            let name = selection.split('\t').next().unwrap_or_default();
            if !ui.has_session(name) {
                eprintln!("tx: session '{name}' does not exist");
                std::thread::sleep(RETRY_PAUSE);
                continue;
            }
            drop(arm_file);
            return show_pick(&ui, show, name);
        }
    }
}

/// fzf's output: the show line ahead of the selection. `--filter` (the tests) prints the
/// selection alone, a focus.
fn split_show(output: &str) -> (Show, &str) {
    let (first, rest) = output.split_once('\n').unwrap_or((output, ""));
    match Show::parse(first) {
        Some(show) => (show, rest),
        None => (Show::Focus, output),
    }
}

/// Shows the pick on the terminal this picker runs on, from the pane it was opened in; outside
/// tmux this terminal attaches it.
fn show_pick(ui: &Ui, show: Show, name: &str) -> Result<i32, VerbError> {
    if !ui.inside() {
        let _ = std::io::stdout().flush();
        return Err(ui_failed(ui.attach_exec(&format!("={name}"))));
    }
    let client = ui
        .display_message("#{client_name}", None)
        .unwrap_or_default();
    let pane = ui.current_pane_id();
    match ui.show_session(&client, show, name, pane.as_deref()) {
        Ok(()) => Ok(0),
        Err(reason) => {
            ui.display_status(&client, &format!("prefix+t: {name}: {reason}"));
            Ok(1)
        }
    }
}

/// Run fzf once over `feed`; `None` on abort or an empty selection.
fn select(
    args: &[String],
    namew: i64,
    arm_file: &Path,
    feed: &str,
) -> Result<Option<String>, VerbError> {
    let mut child = Command::new(&args[0])
        .args(&args[1..])
        .env("NAMEW", namew.to_string())
        .env("TX_ARM_FILE", arm_file)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| VerbError::Failed(format!("could not run fzf: {e}")))?;
    let mut stdin = child.stdin.take();
    let mut stdout = Vec::new();
    let status = std::thread::scope(|scope| {
        // fzf may exit without reading the whole feed; a broken pipe is fine.
        scope.spawn(|| {
            if let Some(stdin) = stdin.as_mut() {
                let _ = stdin.write_all(feed.as_bytes());
            }
            drop(stdin.take());
        });
        if let Some(mut out) = child.stdout.take() {
            let _ = out.read_to_end(&mut stdout);
        }
        child.wait()
    })
    .map_err(|e| VerbError::Failed(format!("fzf: {e}")))?;
    let stdout = String::from_utf8_lossy(&stdout).into_owned();
    Ok((status.success() && !stdout.trim().is_empty()).then_some(stdout))
}

fn ui_failed(error: impl std::fmt::Display) -> VerbError {
    VerbError::Failed(error.to_string())
}

fn hidden_failed(error: std::io::Error) -> VerbError {
    VerbError::Failed(format!("picker hidden roles: {error}"))
}

/// This program and the core's arguments, as the roles panel's binds re-invoke it: its own
/// verbs, run directly.
fn own_command(core: &TxCore) -> Result<String, VerbError> {
    let bin = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .map_err(|e| VerbError::Failed(format!("current_exe: {e}")))?;
    Ok(format!(
        "{} {}",
        bin.display(),
        core.config().cli_arguments().join(" ")
    ))
}

/// `tx` and the core's arguments, for what runs another plugin's verb, or runs later from tmux.
fn tx_command(core: &TxCore) -> String {
    format!(
        "{} {}",
        tx_core::sdk::tx_bin().display(),
        core.config().cli_arguments().join(" ")
    )
}

/// `_picker-key esc`: back to normal mode while searching, else close; prints the fzf actions
/// for `transform(...)`.
pub struct PickerKey;

impl Verb for PickerKey {
    fn name(&self) -> &'static str {
        "_picker-key"
    }

    fn usage(&self) -> &'static str {
        "_picker-key esc   the picker's Esc handler (internal)"
    }

    fn run(&self, _core: &TxCore, io: &mut VerbIo<'_>, argv: &[String]) -> Result<i32, VerbError> {
        let [key] = argv else {
            let _ = writeln!(io.stderr, "usage: tx {}", self.usage());
            return Ok(2);
        };
        if key != "esc" {
            let _ = writeln!(io.stderr, "tx _picker-key: unknown key '{key}'");
            return Ok(2);
        }
        let searching = std::env::var("FZF_PROMPT").is_ok_and(|prompt| prompt == SEARCH_PROMPT);
        writeln!(io.stdout, "{}", picker_keys::esc(searching))
            .map_err(|e| VerbError::Failed(format!("write: {e}")))?;
        Ok(0)
    }
}

/// `_roles [--chips]`: the header's chips line, else the TAB panel's feed.
pub struct Roles;

impl Verb for Roles {
    fn name(&self) -> &'static str {
        "_roles"
    }

    fn usage(&self) -> &'static str {
        "_roles [--chips]   the picker's role chips: header line, else TAB panel feed (internal)"
    }

    fn run(&self, core: &TxCore, io: &mut VerbIo<'_>, argv: &[String]) -> Result<i32, VerbError> {
        let hidden = read_hidden(&hidden_path(core)).map_err(hidden_failed)?;
        let text = if argv.iter().any(|arg| arg == "--chips") {
            chips_line(&hidden)
        } else {
            panel_rows(&hidden)
        };
        writeln!(io.stdout, "{text}").map_err(|e| VerbError::Failed(format!("write: {e}")))?;
        Ok(0)
    }
}

/// `_toggle-hide <role>|--all|--click <column> <line>`: flip a role in the hidden set. `--click`
/// takes the word of the clicked header line under the column; a word that is not a role (a
/// click on the header's other words) is ignored.
pub struct ToggleHide;

impl Verb for ToggleHide {
    fn name(&self) -> &'static str {
        "_toggle-hide"
    }

    fn usage(&self) -> &'static str {
        "_toggle-hide <role>|--all|--click <column> <line>   flip a role the picker hides; --all: all on, or all off when none is hidden (internal)"
    }

    fn run(&self, core: &TxCore, io: &mut VerbIo<'_>, argv: &[String]) -> Result<i32, VerbError> {
        let path = hidden_path(core);
        let word = match argv {
            [all] if all == "--all" => {
                picker_filter::toggle_all(&path).map_err(hidden_failed)?;
                return Ok(0);
            }
            [click, column, line] if click == "--click" => {
                picker_filter::word_at(line, column.parse().unwrap_or_default())
            }
            [word] => word.as_str(),
            _ => {
                let _ = writeln!(io.stderr, "usage: tx {}", self.usage());
                return Ok(2);
            }
        };
        if let Some(role) = picker_filter::role_named(word) {
            picker_filter::toggle(&path, role).map_err(hidden_failed)?;
        }
        Ok(0)
    }
}

/// `_roles-panel`: the TAB roles panel, an fzf over the four roles in the picker's own
/// terminal; space toggles the cursor's role, `a` toggles all, Enter/TAB/q/Esc close.
pub struct RolesPanel;

impl Verb for RolesPanel {
    fn name(&self) -> &'static str {
        "_roles-panel"
    }

    fn usage(&self) -> &'static str {
        "_roles-panel   the picker's TAB roles panel (internal)"
    }

    fn run(&self, core: &TxCore, _io: &mut VerbIo<'_>, _argv: &[String]) -> Result<i32, VerbError> {
        let own = own_command(core)?;
        let reload = format!("reload-sync({own} _roles)");
        let toggle = format!("execute-silent({own} _toggle-hide {{1}})+{reload}");
        let toggle_all = format!("execute-silent({own} _toggle-hide --all)+{reload}");
        let color = active().fzf_color_arg(true);
        let feed = panel_rows(&read_hidden(&hidden_path(core)).map_err(hidden_failed)?);
        let mut child = Command::new("fzf")
            .args([
                "--ansi",
                "--disabled",
                "--reverse",
                "--no-info",
                "--no-separator",
                "--track",
                "--id-nth=1",
                "--delimiter=\t",
                "--with-nth=2..",
                "--height=100%",
                "--with-shell=sh -c",
                "--prompt=  roles ",
                &format!("--header={}", panel_legend()),
                &color,
                &format!("--bind=space:{toggle}"),
                &format!("--bind=a:{toggle_all}"),
                "--bind=j:down,k:up",
                "--bind=enter,tab,q,esc:abort",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .map_err(|e| VerbError::Failed(format!("could not run fzf: {e}")))?;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(feed.as_bytes());
        }
        child
            .wait()
            .map_err(|e| VerbError::Failed(format!("fzf: {e}")))?;
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    const HEADER: &str = "NAME           LOCATION                  STARTED IDLE   ROLE  TAGS";

    const ALL: Binds = Binds {
        edit_tag: true,
        kill: true,
    };

    fn args(binds: Binds) -> Vec<String> {
        fzf_args(
            "/x/tx-picker --data-dir /s",
            "/x/tx --data-dir /s",
            Path::new("/k/tmux.sock"),
            12,
            "",
            &["shell"],
            binds,
        )
    }

    fn bind_of<'a>(args: &'a [String], key: &str) -> Option<&'a str> {
        args.iter().find_map(|arg| {
            arg.strip_prefix("--bind=")
                .and_then(|bind| bind.strip_prefix(key))
                .and_then(|bind| bind.strip_prefix(':'))
        })
    }

    #[test]
    fn namew_env_parsing() {
        assert_eq!(namew_from(Some("7")), 7);
        assert_eq!(namew_from(Some("")), 18);
        assert_eq!(namew_from(Some("7a")), 18);
        assert_eq!(namew_from(None), 18);
    }

    #[test]
    fn fzf_args_are_exact() {
        let own = "/x/tx-picker --data-dir /s";
        let tx = "/x/tx --data-dir /s";
        let reload = format!("reload-sync({own} _list)");
        let chips = format!("{own} _roles --chips");
        let theme = active();
        let accent_ansi = theme.ansi_fg(Role::Accent);
        let fg_ansi = theme.ansi_fg(Role::Text);
        let signal_ansi = theme.ansi_fg(Role::Signal);
        let needs_you = theme.glyphs.needs_you;
        let focus = format!(
            "printf '{accent_ansi}\x1b[1m%s\x1b[0m {fg_ansi}\x1b[1m%s\x1b[0m\\n' {{1}} {{2}}; {chips}; printf '%s' '{HEADER}'"
        );
        let arm = format!(
            "printf '{signal_ansi}\x1b[1m {needs_you}  Kill \"%s\"? [y/N]\x1b[0m\\n' {{1}}; {chips}; printf '%s' '{HEADER}'"
        );
        let unbind = toggle_normal_keys("unbind");
        let search = format!(
            "change-prompt(  / )+{unbind}+execute-silent(: >\"$TX_ARM_FILE\")+transform-header({focus})+change-footer:{}",
            search_footer()
        );
        let mut expected: Vec<String> = vec![
            "fzf".into(),
            "--exact".into(),
            "--ansi".into(),
            "--prompt=  › ".into(),
            "--height=100%".into(),
            "--reverse".into(),
            "--delimiter=\t".into(),
            "--with-nth=5..".into(),
            "--listen".into(),
            "--track".into(),
            "--with-shell=sh -c".into(),
            theme.fzf_color_arg(false),
            format!("--header={}\n{HEADER}", chips_line(&["shell"])),
            format!("--footer={}", normal_footer()),
            "--query=".into(),
            format!(
                "--bind=start:execute-silent(( while sleep 1; do curl -fsS -XPOST \"localhost:$FZF_PORT\" -d \"{reload}\" >/dev/null 2>&1 || exit 0; done ) &)"
            ),
            format!("--bind=ctrl-r:{reload}"),
            format!("--bind=r:{reload}"),
            "--bind=enter:print(focus)+accept".into(),
            "--bind=a:print(attach-to)+accept".into(),
            "--bind=alt-a:print(attach-to)+accept".into(),
            "--bind=p:print(popup)+accept".into(),
            "--bind=alt-p:print(popup)+accept".into(),
            format!("--bind=focus:transform-header({focus})+execute-silent(: >\"$TX_ARM_FILE\")"),
            format!("--bind=tab:execute({own} _roles-panel)+{reload}+transform-header({focus})"),
            format!(
                "--bind=click-header:execute-silent({own} _toggle-hide --click \"$FZF_CLICK_HEADER_COLUMN\" \"$FZF_CLICK_HEADER_WORD\")+{reload}+transform-header({focus})"
            ),
            format!(
                "--bind=ctrl-t:execute(tmux -S /k/tmux.sock display-popup -E -h 4 -w 60% \"{tx} _edit-tag {{1}}\")+{reload}"
            ),
            format!(
                "--bind=t:execute(tmux -S /k/tmux.sock display-popup -E -h 4 -w 60% \"{tx} _edit-tag {{1}}\")+{reload}"
            ),
            format!(
                "--bind=d:execute-silent(printf '%s' {{1}} >\"$TX_ARM_FILE\")+transform-header({arm})"
            ),
            format!(
                "--bind=y:execute-silent([ -s \"$TX_ARM_FILE\" ] && {tx} kill {{1}} >/dev/null 2>&1; : >\"$TX_ARM_FILE\")+{reload}+transform-header({focus})"
            ),
            format!("--bind=n:transform-header({focus})+execute-silent(: >\"$TX_ARM_FILE\")"),
            "--bind=j:down,k:up,g:first,G:last,q:abort".into(),
            "--bind=ctrl-d:half-page-down,ctrl-u:half-page-up".into(),
            format!("--bind=/:{search}"),
            format!("--bind=i:{search}"),
            format!("--bind=esc:transform({own} _picker-key esc)"),
        ];
        expected.extend(
            ignored_keys()
                .iter()
                .map(|key| format!("--bind={key}:ignore")),
        );
        assert_eq!(args(ALL), expected);
    }

    #[test]
    fn a_query_starts_in_search_mode() {
        let actual = fzf_args(
            "/x/tx-picker",
            "/x/tx",
            Path::new("/k/tmux.sock"),
            12,
            "foo",
            &[],
            ALL,
        );
        assert!(actual.contains(&"--prompt=  / ".to_owned()));
        assert!(actual.contains(&format!("--footer={}", search_footer())));
        let start = bind_of(&actual, "start").unwrap();
        assert!(start.ends_with(&format!(") &)+{}", toggle_normal_keys("unbind"))));
    }

    #[test]
    fn digits_are_ignored_in_normal_mode() {
        let actual = args(ALL);
        for digit in '0'..='9' {
            assert_eq!(
                bind_of(&actual, &digit.to_string()),
                Some("ignore"),
                "{digit}"
            );
        }
    }

    #[test]
    fn fzf_args_without_edit_tag_ignores_t() {
        let actual = args(Binds {
            edit_tag: false,
            kill: true,
        });
        assert_eq!(bind_of(&actual, "ctrl-t"), None, "{actual:?}");
        assert_eq!(bind_of(&actual, "t"), Some("ignore"));
        assert!(!actual.iter().any(|arg| arg.contains("_edit-tag")));
        assert_eq!(actual.len(), args(ALL).len() - 1);
        for key in ["d", "y", "n", "ctrl-r"] {
            assert!(bind_of(&actual, key).is_some(), "{key}");
        }
    }

    #[test]
    fn fzf_args_without_kill_ignores_d_and_y() {
        let actual = args(Binds {
            edit_tag: true,
            kill: false,
        });
        assert!(actual.contains(&"--bind=d:ignore,y:ignore".to_owned()));
        assert!(!actual.iter().any(|arg| arg.contains("/s kill ")));
        assert_eq!(actual.len(), args(ALL).len() - 1);
        for key in ["ctrl-t", "t", "n", "ctrl-r"] {
            assert!(bind_of(&actual, key).is_some(), "{key}");
        }
    }

    #[test]
    fn the_show_line_is_split_from_the_selection() {
        let row = "sh\t\tL\t$4\tsh\n";
        assert_eq!(split_show(&format!("popup\n{row}")), (Show::Popup, row));
        assert_eq!(
            split_show(&format!("attach-to\n{row}")),
            (Show::AttachTo, row)
        );
        assert_eq!(split_show(&format!("focus\n{row}")), (Show::Focus, row));
        assert_eq!(split_show(row), (Show::Focus, row));
        assert_eq!(split_show("popup\n"), (Show::Popup, ""));
        assert_eq!(split_show("popup"), (Show::Popup, ""));
        // A session named `popup`, as `--filter` prints it, is a focus.
        let named = "popup\t\tL\t$1\tpopup\n";
        assert_eq!(split_show(named), (Show::Focus, named));
    }
}
