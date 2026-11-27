//! tx-core plugin `nuke`: `tx nuke` starts the IDE again from scratch. Every running agent is
//! stopped through core (its record ends, its conversation is archived), tx's own tmux server is
//! killed with every view in it, a new tx-assistant is created, and the terminal is attached to
//! the home view of the fresh server, which loads the current tmux config and plugin keys.
//!
//! `nuke` is usually typed inside the server it kills, so the work is `_nuke-run`, run outside
//! it: inside the IDE, `nuke` detaches the terminal client with `detach-client -E`, which runs
//! `_nuke-run` in the terminal in its place; outside tmux, `nuke` does the work itself; inside
//! the server with no terminal client, `_nuke-run --no-attach` is started in a session of its own,
//! its output left in `<data>/plugins/nuke/last.log`. Publishes `keys/nuke.tmux` (prefix+Q).

use std::fs::File;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;

use tx_client::tmux::{Ui, shell_quote};
use tx_client::views;
use tx_core::{Agent, ListFilter, Plugin, PluginError, Registry, TxCore, Verb, VerbError, VerbIo};

pub const PLUGIN_ID: &str = "nuke";
/// The internal verb doing the work outside the server.
const RUN_VERB: &str = "_nuke-run";
const NO_ATTACH_FLAG: &str = "--no-attach";
/// The assistant plugin's verb and the subcommand that creates a new tx-assistant.
const ASSISTANT_VERB: &str = "assistant";
const ASSISTANT_NEW: &str = "new";
/// Under the plugin's data directory: the output of a `_nuke-run` that had no terminal.
const LOG_FILE: &str = "last.log";

#[derive(Clone, Copy, Debug, Default)]
pub struct NukePlugin;

impl Plugin for NukePlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn register(&self, r: &mut Registry) -> Result<(), PluginError> {
        r.verb(Arc::new(Nuke));
        r.verb(Arc::new(NukeRun));
        Ok(())
    }
}

fn failed(error: impl std::fmt::Display) -> VerbError {
    VerbError::Failed(error.to_string())
}

/// The `tx` binary and the flags that open this same core, `words` after them.
fn tx_command(core: &TxCore, words: &[&str]) -> Command {
    let mut command = Command::new(tx_core::sdk::tx_bin());
    command.args(core.config().cli_arguments()).args(words);
    command
}

fn running_agents(core: &TxCore) -> Result<Vec<Agent>, VerbError> {
    core.list_agents(&ListFilter::new().running())?
        .iter()
        .map(|id| core.get_agent(id).map_err(VerbError::from))
        .collect()
}

pub struct Nuke;

impl Verb for Nuke {
    fn name(&self) -> &'static str {
        "nuke"
    }

    fn usage(&self) -> &'static str {
        "nuke [--dry-run]   stop every agent, kill the IDE's tmux server, start it again with a new tx-assistant and attach (prefix+Q); --dry-run: what would be stopped and closed"
    }

    fn run(&self, core: &TxCore, io: &mut VerbIo<'_>, argv: &[String]) -> Result<i32, VerbError> {
        let ui = Ui::from_core(core);
        match argv {
            [] => {}
            [flag] if flag == "--dry-run" => return dry_run(core, &ui, io),
            _ => {
                let _ = writeln!(io.stderr, "usage: tx nuke [--dry-run]");
                return Ok(2);
            }
        }
        if !ui.inside() {
            return nuke(core, &ui, io, true);
        }
        match ui.terminal_client() {
            Ok(client) => {
                let mut words = vec![tx_core::sdk::tx_bin().to_string_lossy().into_owned()];
                words.extend(core.config().cli_arguments());
                words.push(RUN_VERB.to_owned());
                let command = words
                    .iter()
                    .map(|word| shell_quote(word))
                    .collect::<Vec<_>>()
                    .join(" ");
                ui.run(&["detach-client", "-t", &client, "-E", &command])
                    .map_err(failed)?;
            }
            Err(_) => start_detached(core)?,
        }
        Ok(0)
    }
}

/// `_nuke-run` with no terminal to come back to: in a session of its own, so the server's death
/// does not take it along, its output in [`LOG_FILE`].
fn start_detached(core: &TxCore) -> Result<(), VerbError> {
    let directory = core.config().plugin_data_dir(PLUGIN_ID);
    std::fs::create_dir_all(&directory).map_err(failed)?;
    let log: PathBuf = directory.join(LOG_FILE);
    let output =
        File::create(&log).map_err(|error| failed(format!("{}: {error}", log.display())))?;
    let mut command = tx_command(core, &[RUN_VERB, NO_ATTACH_FLAG]);
    command
        .stdin(Stdio::null())
        .stdout(output.try_clone().map_err(failed)?)
        .stderr(output)
        .env_remove("TMUX");
    // SAFETY: `setsid` is async-signal-safe and touches no memory of the parent.
    unsafe {
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    command.spawn().map_err(failed)?;
    Ok(())
}

fn dry_run(core: &TxCore, ui: &Ui, io: &mut VerbIo<'_>) -> Result<i32, VerbError> {
    let mut out = String::new();
    for agent in running_agents(core)? {
        out.push_str(&format!("stop agent {}\n", agent.name));
    }
    if ui.quiet(&["list-sessions"]).0 {
        for view in views::list(ui).map_err(failed)? {
            out.push_str(&format!("close view {} ({})\n", view.name, view.kind));
        }
        out.push_str("kill the tmux server\n");
    }
    if core.verb(ASSISTANT_VERB).is_some() {
        out.push_str("create a new tx-assistant\n");
    }
    out.push_str("attach to home\n");
    io.stdout.write_all(out.as_bytes()).map_err(failed)?;
    Ok(0)
}

pub struct NukeRun;

impl Verb for NukeRun {
    fn name(&self) -> &'static str {
        RUN_VERB
    }

    fn usage(&self) -> &'static str {
        "_nuke-run [--no-attach]   nuke's work, run outside the server it kills (internal)"
    }

    fn run(&self, core: &TxCore, io: &mut VerbIo<'_>, argv: &[String]) -> Result<i32, VerbError> {
        let attach = match argv {
            [] => true,
            [flag] if flag == NO_ATTACH_FLAG => false,
            _ => {
                let _ = writeln!(io.stderr, "usage: tx {RUN_VERB} [{NO_ATTACH_FLAG}]");
                return Ok(2);
            }
        };
        nuke(core, &Ui::from_core(core), io, attach)
    }
}

/// Stops every agent, kills the server, creates a new tx-assistant, then (with `attach`) execs
/// `tx ui attach`, which starts home on the fresh server. An agent or an assistant that fails is
/// reported and never stops the rest.
fn nuke(core: &TxCore, ui: &Ui, io: &mut VerbIo<'_>, attach: bool) -> Result<i32, VerbError> {
    let mut status = 0;
    for agent in running_agents(core)? {
        match core.stop_agent(&agent.id) {
            Ok(stopped) => {
                let _ = writeln!(io.stdout, "stopped {}", stopped.name);
            }
            Err(error) => {
                let _ = writeln!(io.stderr, "tx nuke: {}: {error}", agent.name);
                status = 1;
            }
        }
    }
    if ui.quiet(&["kill-server"]).0 {
        let _ = writeln!(io.stdout, "killed the tmux server");
    }
    if let Some(assistant) = core.verb(ASSISTANT_VERB) {
        match assistant.run(core, io, &[ASSISTANT_NEW.to_owned()]) {
            Ok(0) => {}
            Ok(code) => status = code,
            Err(error) => {
                let _ = writeln!(io.stderr, "tx nuke: tx-assistant: {error}");
                status = 1;
            }
        }
    }
    if !attach {
        return Ok(status);
    }
    let _ = io.stdout.flush();
    let error = tx_command(core, &["ui", "attach"])
        .env_remove("TMUX")
        .exec();
    Err(failed(format!("could not run tx ui attach: {error}")))
}
