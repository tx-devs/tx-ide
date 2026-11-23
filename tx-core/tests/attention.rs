//! The attention plugin through the binaries: the hooks an engine fires move an agent, the
//! plugin's event sink marks the agent's tmux session, `tx attention` reads the marks, and the
//! key fragment shows them and takes a finished one back on focus.

mod common_cli;

use common_cli::Cli;
use serde_json::Value;

const QUESTION: &str = r#"{"tool_name":"AskUserQuestion","tool_input":{"questions":[]}}"#;

impl Cli {
    fn hook(&self, event: &str, id: &str, stdin: &str) {
        let output = self.txi(&["hook", event], &[("TX_SESSION_ID", id)], stdin);
        assert!(output.status.success(), "{event}: {output:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            "",
            "{event}: the sink says nothing"
        );
    }
    /// The agent works on a prompt.
    fn works(&self, id: &str) {
        self.hook("UserPromptSubmit", id, "{}");
    }
    /// The agent's turn ends.
    fn finishes(&self, id: &str) {
        self.works(id);
        self.hook("Stop", id, "{}");
    }
    /// The agent stops mid-turn on a question to the operator.
    fn asks(&self, id: &str) {
        self.works(id);
        self.hook("PermissionRequest", id, QUESTION);
    }
    /// The need of the mark on the agent's session: `waiting`, `finished`, or empty.
    fn mark(&self, name: &str) -> String {
        self.tmux_out(&[
            "show-options",
            "-vqt",
            &format!("={name}:"),
            "@tx-attention",
        ])
        .split(' ')
        .next()
        .unwrap()
        .to_owned()
    }
    fn create_with(&self, name: &str, flags: &[&str]) -> String {
        let cwd = self.repository(name);
        let path = self.fake_claude_path();
        let mut args = vec![
            "create",
            name,
            "--cwd",
            cwd.to_str().unwrap(),
            "--engine",
            "claude",
            "--prompt",
            "start",
        ];
        args.extend(flags);
        let output = self.run(&args, &[("PATH", path.as_str())], "");
        assert!(output.status.success(), "tx create {name}: {output:?}");
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }
    /// The command of a pane that shows the session `name`: a nested attach, under job control
    /// as the other suites nest.
    fn nest(&self, name: &str) -> String {
        let sock = self.sock().display().to_string();
        format!("bash -c 'set -m; TMUX= tmux -S {sock} attach -t ={name}; exec sh'")
    }
    /// A terminal on the session `name`: a client attached from a pane of another server.
    fn attach_terminal(&self, name: &str) -> String {
        let sock = self.sock().display().to_string();
        self.in_outer_pane(&format!("TMUX= tmux -S {sock} attach -t ={name}"));
        let client = || {
            self.tmux_out(&[
                "list-clients",
                "-t",
                &format!("={name}"),
                "-F",
                "#{client_name}",
            ])
        };
        assert!(
            self.wait_for(5, || !client().is_empty()),
            "no client on {name}"
        );
        client()
    }
}

#[test]
fn an_agent_waiting_on_the_operator_is_marked_until_it_works_again() {
    require_tmux!();
    let cli = Cli::new();
    let id = cli.create("a");
    assert_eq!(cli.mark("a"), "");
    cli.asks(&id);
    assert_eq!(cli.mark("a"), "waiting");
    let listing = cli.ok(&["attention"]);
    assert!(listing.starts_with("waiting\ta\t"), "{listing:?}");
    assert_eq!(cli.ok(&["attention", "ls"]), listing);
    let json: Value = serde_json::from_str(&cli.ok(&["attention", "ls", "--json"])).unwrap();
    assert_eq!(json[0]["id"], id.as_str());
    assert_eq!(json[0]["name"], "a");
    assert_eq!(json[0]["need"], "waiting");

    cli.hook("PostToolUse", &id, "{}");
    assert_eq!(cli.mark("a"), "", "answered: it works again");
    assert_eq!(cli.ok(&["attention"]), "");
}

#[test]
fn a_finished_turn_is_marked_for_the_agents_the_operator_started() {
    require_tmux!();
    let cli = Cli::new();
    std::fs::write(
        cli.dir.path().join("default-roles/TX-ASSISTANT.md"),
        "stand-in role\n",
    )
    .unwrap();
    let own = cli.create("own");
    let worker = cli.create_with("worker", &["--parent", &own]);
    let assistant = cli.create_with(
        "assistant",
        &["--role", "TX-ASSISTANT", "--tag", "tx-system"],
    );
    let asked_for = cli.create_with("asked-for", &["--parent", &assistant]);

    // Idle straight from starting: nothing was worked on.
    cli.hook("SessionStart", &own, r#"{"source":"startup"}"#);
    assert_eq!(cli.mark("own"), "");

    for id in [&own, &worker, &asked_for] {
        cli.finishes(id);
    }
    assert_eq!(cli.mark("own"), "finished");
    assert_eq!(cli.mark("worker"), "", "its result is its parent's");
    assert_eq!(
        cli.mark("asked-for"),
        "finished",
        "the assistant started it for the operator"
    );
    // A worker that needs an answer is the operator's all the same.
    cli.asks(&worker);
    assert_eq!(cli.mark("worker"), "waiting");
    let listing = cli.ok(&["attention"]);
    let names: Vec<&str> = listing
        .lines()
        .map(|line| line.split('\t').nth(1).unwrap())
        .collect();
    assert_eq!(
        names,
        ["worker", "own", "asked-for"],
        "waiting first, then the oldest: {listing:?}"
    );
}

#[test]
fn settings_choose_which_finished_agents_are_marked() {
    require_tmux!();
    let cli = Cli::new();
    cli.write_yaml("plugins:\n  attention: { settings: { finished: all } }\n");
    let own = cli.create("own");
    let worker = cli.create_with("worker", &["--parent", &own]);
    cli.finishes(&worker);
    assert_eq!(cli.mark("worker"), "finished");

    cli.write_yaml("plugins:\n  attention: { settings: { finished: none } }\n");
    cli.finishes(&own);
    assert_eq!(cli.mark("own"), "");
    cli.asks(&own);
    assert_eq!(cli.mark("own"), "waiting", "waiting is always marked");
}

#[test]
fn an_agent_the_operator_is_looking_at_is_not_announced() {
    require_tmux!();
    let cli = Cli::new();
    let told = cli.dir.path().join("told");
    cli.write_yaml(&format!(
        "plugins:\n  attention: {{ settings: {{ command: \"cat >> '{}'\" }} }}\n",
        told.display()
    ));
    let seen = cli.create("seen");
    let unseen = cli.create("unseen");
    cli.attach_terminal("seen");
    cli.finishes(&seen);
    assert_eq!(cli.mark("seen"), "", "its finished turn has been seen");
    cli.asks(&seen);
    assert_eq!(cli.mark("seen"), "waiting", "a question stays open");
    cli.asks(&unseen);
    assert_eq!(cli.mark("unseen"), "waiting");
    assert!(cli.wait_for(5, || told.exists()), "the command never ran");
    let text = std::fs::read_to_string(&told).unwrap();
    let names: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap()["name"].clone())
        .collect();
    assert_eq!(names, ["unseen"], "{text:?}");
}

#[test]
fn the_operators_command_is_told_who_needs_them() {
    require_tmux!();
    let cli = Cli::new();
    let told = cli.dir.path().join("told");
    cli.write_yaml(&format!(
        "plugins:\n  attention: {{ settings: {{ command: \"cat >> '{}'\" }} }}\n",
        told.display()
    ));
    let id = cli.create("a");
    cli.finishes(&id);
    cli.asks(&id);
    assert!(cli.wait_for(5, || told.exists()), "the command never ran");
    let mut lines = Vec::new();
    cli.wait_for(5, || {
        lines = std::fs::read_to_string(&told)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect();
        !lines.is_empty()
    });
    assert_eq!(
        lines,
        [serde_json::json!({
            "id": id, "name": "a", "need": "waiting", "text": "a needs you"
        })],
        "desktop: waiting, the default, leaves a finished turn out"
    );
}

#[test]
fn a_mark_that_outlived_its_reason_is_taken_back() {
    require_tmux!();
    let cli = Cli::new();
    let id = cli.create("a");
    cli.works(&id);
    assert!(
        cli.tmux(&["set-option", "-t", "=a:", "@tx-attention", "waiting 10"])
            .status
            .success()
    );
    assert_eq!(cli.ok(&["attention"]), "");
    assert_eq!(cli.mark("a"), "");

    cli.asks(&id);
    cli.ok(&["attention", "clear"]);
    assert_eq!(cli.mark("a"), "");
    cli.fails(&["attention", "nope"], 2);
}

#[test]
fn next_goes_to_the_pane_showing_the_first_marked_agent() {
    require_tmux!();
    let cli = Cli::new();
    let id = cli.create("a");
    assert!(
        cli.tmux(&[
            "new-session",
            "-d",
            "-s",
            "desk",
            "-x",
            "120",
            "-y",
            "30",
            "cat"
        ])
        .status
        .success()
    );
    assert!(
        cli.tmux(&["split-window", "-d", "-t", "=desk:", &cli.nest("a")])
            .status
            .success()
    );
    let client = cli.attach_terminal("desk");
    let active = || cli.tmux_out(&["display-message", "-p", "-t", "=desk:", "#{pane_index}"]);
    let before = active();

    cli.ok(&["attention", "next", &client]);
    assert_eq!(active(), before, "nobody needs the operator");

    cli.asks(&id);
    assert_eq!(cli.mark("a"), "waiting", "its pane is not the active one");
    cli.ok(&["attention", "next", &client]);
    assert!(
        cli.wait_for(5, || active() != before),
        "pane stayed {before}"
    );
    assert_eq!(cli.mark("a"), "waiting", "looked at, not answered");
}

#[test]
fn the_fragment_shows_the_marks_and_looking_takes_a_finished_one_back() {
    require_tmux!();
    let cli = Cli::new();
    cli.ok(&["update"]);
    let finished = cli.create("finished");
    let waiting = cli.create("waiting");
    assert!(
        cli.wait_for(10, || {
            cli.tmux_out(&["show-hooks", "-g", "pane-focus-in"])
                .contains("@tx-attention")
                && cli
                    .tmux_out(&["list-keys", "-T", "prefix", "Space"])
                    .contains("attention next")
        }),
        "the fragment never loaded"
    );
    cli.finishes(&finished);
    cli.asks(&waiting);
    let status = cli.tmux_out(&["display-message", "-p", "#{E:status-left}"]);
    assert!(status.contains("○ finished"), "{status:?}");
    assert!(status.contains("▲ waiting"), "{status:?}");

    cli.attach_terminal("finished");
    assert!(
        cli.wait_for(5, || cli.mark("finished").is_empty()),
        "the mark stayed"
    );
    assert!(
        cli.tmux(&["new-session", "-d", "-s", "desk", &cli.nest("waiting")])
            .status
            .success()
    );
    assert!(
        cli.wait_for(5, || {
            !cli.tmux_out(&["list-clients", "-t", "=waiting", "-F", "#{client_name}"])
                .is_empty()
        }),
        "no client on waiting"
    );
    assert_eq!(cli.mark("waiting"), "waiting", "looked at, not answered");
    let status = cli.tmux_out(&["display-message", "-p", "#{E:status-left}"]);
    assert!(!status.contains("finished"), "{status:?}");
}
