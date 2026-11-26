<p>
  <img src="docs-site/assets/tx-mark.svg" alt="tx" width="80" height="44">
</p>

# tx-ide

tx-ide is an IDE in tmux for a swarm of interactive coding agents. Each agent is an engine's own
CLI, Claude Code or Codex, unwrapped: it runs in a git worktree of its own, with a durable record
on disk, inside a detached session on tx's **own** tmux server. It outlives the command that
started it: attaching shows its native terminal, detaching leaves it running, and an ended agent
respawns into the same worktree and conversation. Agents and the operator message each other with
`tx send`. Every action, whether the operator, a key binding or an agent takes it, goes through one
front end: `tx`.

Everything around the agents is a plugin:

- **The home** (`tx ui attach`, prefix+m): projects, their agents and views, live; one key starts,
  enters, stops or resumes an agent.
- **Agent definitions**: `developer`, `qa`, `orchestrator`, `layout-author` and `claude` ship, each
  with its prompt, tools, skills and MCP servers; yours sit beside them and win by name.
- **The graph** (prefix+g): the agents by lineage, every message between them travelling its edge
  live; send, fork, stop, respawn and open artifacts from it.
- **The timeline** (prefix+T): swimlanes of each agent's working and idle time, and of messages.
- **The board** (prefix+b): the checkout's GitHub issues as a kanban, with the agents on each
  issue; one key starts an agent on an issue.
- **Artifacts** (prefix+a): durable, versioned deliverables: documents, diffs opened in diffview,
  code tours walked in nvim.
- **Layouts**: a window of agents, commands and shells, saved as YAML and opened in any project.
- **Attention** (prefix+Space): the agents that stopped on you, or finished a turn you are owed,
  on the status line and in a notification; one key goes to the next one.
- **The assistant** (prefix+/): a resident agent that knows tx, asked one line from any pane and
  told which pane.
- **Views**: nvim, shells and diffviews beside the agents, on the same server.
- The session picker (prefix+t), the new-agent form (prefix+n), fork (prefix+f), mockups for
  designing a screen with an agent, a tutor, and `tx gc` for the build output of ended agents.

How it all works is in the guide, <https://sok205.github.io/tx-ide-rs/guide/>, next to the tx-core
API docs. `tx help` lists every verb and flag.

## Install

```sh
git clone <this-repo> ~/tx-ide-rs
~/tx-ide-rs/install --dry-run   # print what every step would do, change nothing
~/tx-ide-rs/install             # on a terminal: a checklist of the shipped plugins, all ticked
tx ui attach                    # the IDE: the home
```

Needs Rust 1.92+, tmux, jq and git. What else depends on the plugins you take: each one names the
programs it runs (`fzf`, `nvim`, an engine CLI: `claude`, the default engine and the assistant's,
`codex`, `cursor-agent` or `agy`), and `install` warns about every one that a selected plugin needs and `PATH` does
not have.

**Which plugins.** tx's features are plugins, and an install takes a selection of them. A plugin
outside the selection is not built and not installed. Three presets ship: `headless` (what an
agent needs without the IDE: the engines, artifacts, chat, the pane labels, `tx setup` itself),
`ide` (that and the IDE) and `full` (everything, the default).

```sh
install                          # on a terminal: the checklist; 1, 2, 3 tick a preset, space a row
install --preset headless        # a preset: headless, ide, full, or one of your own
install --plugins mockup,tour    # exactly these, and the plugins they require
install --yes                    # ask nothing: full
install --link-dotfiles          # also link ~/.config/nvim, ~/.tmux.conf and the IDE keys to config/
install --upgrade                # after a git pull: the same selection, rebuilt
```

Without a terminal, and with `--yes`, the selection is `full`. A preset of your own is
`~/.config/tx-ide/presets/<name>.yaml` (see the guide's Files page). A plugin outside the
selection is not installed: its verbs are unknown to `tx`.

**What lands where.**

- `tx` and `txi` in cargo's bin directory (`~/.cargo/bin`): the only two programs on `PATH`.
- The selected plugins under `<data>/shipped/<id>/` (the manifest, the program, the resources),
  built with `cargo build --release` and laid out by
  `tx update --bundles plugins --binaries target/release`; the selection itself is recorded in
  `<data>/shipped/.selection`. The same `tx update` runs the pending update steps, publishes the
  shipped roles and harness and every plugin's keys and scripts under the data directory, and
  runs each plugin's updater (the IDE's tmux config).
- Nothing under `~/.config/tx-ide`: that directory is yours. tx's defaults come with the binary
  and the plugins, and what you put there is laid over them (see [Configuration](#configuration)).
- Your dotfiles are left alone unless you ask: with `--link-dotfiles`, or the checklist's last
  row, `~/.config/nvim`, `~/.tmux.conf` and `~/.config/tx-ide/tmux.conf` become links to this
  repository's `config/nvim`, `config/tmux.conf` and `config/tx-ide/tmux.conf`, the maintainer's
  own editor, tmux setup and IDE keys (what was there moves to `<path>.bak.<stamp>`). tx needs
  none of them.

It is safe to re-run; a run with another `--preset` or `--plugins` changes the selection, and
`tx update` removes the plugins that left it.

**Upgrading.** `git pull`, then `install --upgrade`. It asks nothing, resolves the recorded
preset again (so a plugin that is new in `full` arrives), rebuilds and lays everything out, and
never links a dotfile. After rebuilding `tx` by hand, run `tx update`: it runs
the update steps a newer tx needs, such as moving files (the last one run is in
`<data>/update-version`; other verbs warn while one is pending), publishes the roles, harness
and plugin resources again, then reinstalls the tmux config, which names the binary's path. With
`tx update --bundles plugins --binaries target/debug --link` the shipped plugins are links into
the checkout and the build, so a rebuild is live.

Coming from the previous `tx` (removed on 2026-11-02; its last tree is commit `7607347`),
`install` also switches the machine once: the old install stays runnable as `tx-old`, its hook
entries leave `~/.claude/settings.json` and `~/.codex/hooks.json` after a backup, and
`tx doctor --fix-hooks` removes what older versions wired (it is the `chat` plugin's verb, so
that last step runs when `chat` is selected).

## First steps

The prefix is `C-a`.

```sh
tx ui attach
```

lands on the home, the floor of the `home` view: projects, their agents and views. The first
attach also starts the assistant in the background.

- **Start an agent**: `n` in the home, or prefix+n anywhere, opens the new-agent form; from a
  shell, `tx create NAME --cwd DIR --prompt TEXT`. The agent runs in its own worktree of DIR's
  checkout at `HEAD`, as the `developer` definition unless told otherwise, and makes its own
  branch there.
- **Go in and out**: Enter on an agent nests its terminal into the home's pane. `C-a C-a d`
  detaches the nested agent and the home comes back; the agent keeps running. `C-a d` detaches the
  terminal itself; `tx ui attach` brings everything back as it was.
- **Talk**: type into the agent, or `tx send NAME -- TEXT` from anywhere (run inside an agent, the
  message is signed as that agent). prefix+/ asks the assistant one line from any pane.
- **Watch**: prefix+g the graph of agents and their messages, prefix+T the timeline, prefix+b the
  issue board, prefix+t the picker, prefix+a the artifacts an agent touched.
- **Finish**: `x` then `y` in the home, or `tx stop NAME`, ends an agent; Enter on an ended agent,
  or `tx respawn NAME`, resumes it in the same worktree and conversation. `tx gc` deletes the build
  output of agents ended more than a day ago.

## Keys

| keys | does |
|---|---|
| prefix+m | the home in a popup |
| prefix+t | the session picker |
| prefix+n | new agent: the form, then the agent in the pane's directory |
| prefix+f | fork the agent in the pane into a new worktree |
| prefix+e | edit the agent's name, tags and group |
| prefix+X | stop the agent in the pane, or close the view, after a y/n prompt |
| prefix+/ | ask the assistant one line |
| prefix+g | the agent graph |
| prefix+T | the timeline |
| prefix+b | the GitHub issue board of the pane's checkout |
| prefix+P | switch plugins on and off |
| prefix+Space | go to the next agent that needs you |
| prefix+a | in an agent: the artifacts it touched, its parent and children; in a view: back to its agent |
| prefix+s | refresh the labels, then `choose-tree` |
| prefix+\| / prefix+- | split right / below |
| C-h / C-j / C-k / C-l | move between nvim splits and panes, out of nested sessions too |
| C-u / C-d | page an agent's scrollback |
| C-a C-a d | leave a nested agent or view; it keeps running |

In the home: j/k move, Enter attaches (or resumes an ended agent), → unfolds an agent's detail, n
starts an agent, x stops one, r resumes one, a its artifacts, / asks the assistant, f finds, w the
saved layouts, t the timeline, b the next backdrop, p the plugins, ? every plugin's keys, q closes
the popup. The graph lists its keys on `?`.

## Commands

| verb | does |
|---|---|
| `tx create NAME --cwd DIR --prompt T` | start an agent in its own worktree of DIR's checkout; `--agent`, `--engine`, `--model`, `--effort`, `--tag`, `--parent` and more override `config.yaml` |
| `tx ls [--ended] [--json]` | the running agents, or the ended ones |
| `tx send NAME -- TEXT` | type a message into an agent, in an envelope saying who sent it |
| `tx fork NAME --prompt T` | a new agent continuing NAME's conversation, in its own worktree |
| `tx stop NAME`, `tx interrupt NAME` | end the agent its engine's way; abort its running turn |
| `tx respawn NAME` | start an ended agent again, in the same worktree and conversation |
| `tx show NAME`, `tx where NAME`, `tx attach NAME` | its record as JSON; its tmux socket, session and pane; attach this terminal to it |
| `tx rename NAME NEW`, `tx tag NAME TAG…` | rename it; replace its tags |
| `tx gc [--older-than 1d] [--dry-run]` | delete the build output (`worktree.build_outputs`) in the worktrees of agents that ended longer ago than that |
| `tx config [--defaults \| --path]` | the settings in use: your `config.yaml` laid over tx's defaults, your values marked; `--defaults`: every default with its documentation; `--path`: your file |
| `tx setup [--bundles DIR] [--preset NAME \| --plugins a,b,c \| --upgrade] [--list]` | choose which shipped plugins this machine has (a preset, ids, or a checklist on a terminal), record the selection in `<data>/shipped/.selection` |
| `tx harness ls`, `tx harness check [--plugins [ID…]]` | list and validate the agent definitions, skills and MCP servers; `tx harness check` then validates the installed plugins (manifests, programs, resources, clashes, `requires`, the config, the shipped stamp), and `--plugins` checks only those |
| `tx update [--bundles DIR --binaries DIR [--link]]` | bring this machine up to this tx: lay the shipped plugins out from a checkout's `plugins/` and its build (with `--bundles`), run the pending update steps, publish the roles, harness and plugin resources under the data directory, run each plugin's updater |
| `tx engines`, `tx plugins` | the enabled engines; every plugin and whether it is on |

Plugins add `ui`, `home`, `pick`, `kill`, `whoami`, `chat`, `doctor`, `assistant`, `artifact`,
`group`, `tour`, `timeline`, `board`, `switch`, `attention`, `graph`, `layout`, `mockup` and `tutor`. `tx help` prints every verb
with its flags; verbs starting with `_` are internal, run by key bindings and hooks.

## Agent definitions

An agent's system prompt is its **roles**, prompt text agents share (`COMMON` under every shipped
definition but `claude`, `TX-ASSISTANT` for the assistant), then its **agent definition**: Claude's
agent format, `name`, `description`, `tools`, `mcpServers`, `disallowedTools`, `skills` and
`roles`, with the prompt as the body.

| definition | what it is |
|---|---|
| `developer` | the default: writes code with the operator; short chat, the reasoning in artifacts |
| `qa` | verifies one PR in a sandbox, the way a person would use the product; never edits the repository |
| `orchestrator` | runs a multi-team programme of implementers and QA agents; never writes code |
| `layout-author` | writes one saved layout from a description; the home's layout list starts it |
| `claude` | plain Claude Code: every built-in tool, no prompt of its own |

`tx create NAME --agent qa …` picks one; `--skill`, `--mcp` and `--tool` add to it. The shipped
agent definitions, the tx skills (`tx-sessions`, `tx-artifacts`, `tx-diff-review`, `tx-code-tours`,
`tx-mockup`, `tx-agents`) and `mcp.json` live in `config/tx/` and are installed under
`<data>/harness`. Yours go in `<config>/harness/` (`agents/<name>.md`,
`skills/<name>/SKILL.md`, `mcp.json`) and win by name, as your roles do in `<config>/roles/`.
`tx harness ls` lists what resolves; `tx harness check` validates it.

## Configuration

tx's defaults come with the binary and the installed plugins; yours live in `~/.config/tx-ide`,
which tx never writes. What you put there is laid over the defaults, so it survives every
update, and a default tx changes later still reaches you for every key you left alone.

| file | what it changes |
|---|---|
| `config.yaml` | settings: only the keys you change |
| `tmux.conf` | the IDE's tmux server: loaded last, so your prefix, bindings and options win |
| `nvim/tx.lua` | nvim views: run last, after your own nvim config and tx's runtime |
| `harness/`, `roles/` | your agents, skills, MCP servers and roles; one of the same name replaces a shipped one, and `roles/<ROLE>.local.md` adds to a role |
| `layouts/`, `themes/` | saved pane layouts; themes of your own |

`tx config` prints the settings in use with your values marked, `tx config --defaults` every
default with its documentation, `tx config --path` your file (`tx help` and `--config FILE` name
it too). Every key is optional:

```yaml
agent:                         # defaults for `tx create`; a flag given replaces its key
  engine: claude               # claude | codex | cursor | antigravity
  roles: []                    # empty: the definition's own `roles`
  definition: developer        # the agent definition without --agent; null: roles alone
  model: claude-opus-5-5       # null: the engine's own default
  effort: 3                    # 1-5: low, medium, high, xhigh, max (or the engine's own word)

worktree:
  build_outputs: [target]      # what `tx gc` may delete in an ended agent's worktree

plugins:                       # one entry per plugin (`tx plugins`); a missing entry is on
  engine-codex: { enabled: false }              # switches a plugin off
  assistant:
    settings: { effort: 3 }                     # laid over the plugin's own settings, key by key
```

A plugin's settings and their defaults are the `settings:` of its `plugin.yaml`; `tx config
--defaults` prints them for every installed plugin. Which shipped plugins an install takes is
`tx setup`'s to record (`<data>/shipped/.selection`), not this file's:
`tx setup --bundles <repo>/plugins --preset headless | ide | full` selects by preset (the plugins
of tier `core`; `core` and `ide`; all of them), `--plugins a,b,c` by id, and on a terminal with
neither it shows a checklist. `--list` prints every preset with its plugins, among them your own
in `~/.config/tx-ide/presets/<name>.yaml`.

`model` applies only to `agent.engine`: with `--engine codex`, Codex runs its own default.
`engine-claude`'s `claude_settings` are Claude Code `settings.json` keys passed to every Claude
launch: yours are laid over the ones tx ships, a key set to `null` is left out, and your `hooks`
run next to tx's.

### Theme

tx has no palette of its own: by default every colour it draws is the terminal's own foreground
or one of its sixteen colours, so it wears your terminal's theme. `theme:` in `config.yaml`
changes that for the tmux frame, tx's own screens, the fzf pickers and the nvim forms at once.

```yaml
theme:
  name: terminal        # terminal | tokyo-night | a file in themes/
  glyphs: unicode       # unicode | ascii
  colors:               # optional: roles to replace in the named theme
    accent: magenta     # one of the terminal's sixteen by name, or #rrggbb
```

A theme of your own is `~/.config/tx-ide/themes/<name>.yaml`: a shipped theme as `base`, with
`colors`, and `dark` or `light` for one background only. `tx update` applies a change, to a
running server at once, and reports a role or a colour it does not know instead of skipping it.
`NO_COLOR` removes every colour and keeps bold, dim and reverse. The roles, the theme file and
how the theme reaches each surface are in the guide's
[theme page](docs-site/guide/src/theme.md).

## Where files live

Five directories, XDG style (on macOS too), named after the application: `tx-ide`, or
`TX_APPNAME`, which renames all five as `NVIM_APPNAME` does. Each also takes a flag
(`--config-dir` …) and a variable (`TX_CONFIG_DIR` …), which every agent's session carries.

| directory | default | holds |
|---|---|---|
| config | `~/.config/tx-ide` | yours alone, never written by tx: `config.yaml`, `tmux.conf`, `nvim/tx.lua`, `roles/`, `harness/`, `layouts/`, `themes/` |
| data | `~/.local/share/tx-ide` | the agent records with their conversation archives, `log.jsonl`, the agents' `worktrees/`, plugin data, the shipped roles and harness, the IDE's tmux config |
| state | `~/.local/state/tx-ide` | locks, each agent's assembled prompt |
| cache | `~/.cache/tx-ide` | Claude's built-in tool list, per Claude version |
| run | `$XDG_RUNTIME_DIR/tx-ide`, else `$TMPDIR/tx-ide.$USER` | `tmux.sock`, tx's own tmux server |

## Development

```sh
cargo test --workspace                                  # needs tmux and git; private sockets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --features fuzz                  # the slow property-based suites too
cargo run -p tx-core -- help                            # the tx binary
```

One Cargo workspace: `tx-core`, the library every plugin depends on (records, lifecycle,
messaging, the harness, the plugin host, the plugin SDK) and the `tx` and `txi` binaries (`txi`
is the entry point the engines' hooks run); `tx-client`, the tmux and nvim layer the IDE plugins
share; and one crate per plugin under `plugins/`. The architecture, writing an engine or a
plugin, and every file are in the guide; it and the API docs are built from `docs-site/` (see
`docs-site/README.md`).

Branches, pull requests and releases are in [CONTRIBUTING.md](CONTRIBUTING.md).
