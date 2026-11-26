# The tmux layer and views

tx runs every agent and every view on a tmux server of its own. This page covers that server, how
its configuration is composed from plugins, what a view is, and how sessions nest inside each
other.

## tx's own server

The server listens on `<run>/tmux.sock` (the run directory is created mode 0700) and loads
`<data>/tmux.conf`. Every tmux call core or a plugin makes carries both `-S <socket>` and
`-f <conf>`, so your default tmux server and `~/.tmux.conf` are never touched. Targets are exact
(`=name`), user-controlled arguments come after `--`, and anything ending in `;` is escaped so
tmux cannot read it as a second command. The server exits when its last session ends.

Because the socket is not the default one, plain `tmux ls` will not show tx's sessions. For raw,
read-only inspection, ask tx where something lives:

```sh
tx where worker-1     # socket <TAB> session target <TAB> pane target
IFS=$'\t' read -r socket session pane < <(tx where worker-1)
tmux -S "$socket" capture-pane -p -t "$pane"
```

The shipped roles allow raw tmux for inspection, moving between panes and scrolling only. Anything
that creates, ends or changes an agent or a view goes through `tx`, or the records go stale.

## `tx update`: composing the configuration

Core writes a two-line default `<data>/tmux.conf` the first time it opens (`exit-empty`, the
history limit) and never looks at it again. The IDE's configuration is put there by `tx update`,
in two steps.

**`tx update` publishes the resources.** After the core's update steps it writes, under the data
directory, core's roles and harness and every active plugin's resources (`bin/` and `keys/`
executable, `conf/`, `roles/`, `harness/` and the nvim runtime plain). Files under those
directories that no active plugin provides are deleted, so the installed tree always matches the
enabled plugins and `tx` needs nothing from the source tree at run time. This step is `tx`'s own:
an install without the `ide` plugin has its roles and harness all the same.

**The `ide` plugin composes the server config.** `ide` (`plugins/ide`) has an updater, which
`tx update` runs after the publish: as the program `tx-ide --tx-update` where the plugin is
installed as a bundle, inside `tx` where it is not. It writes `<data>/tmux.conf` from the key
fragments it finds under `<data>/keys`. In order, that file:

1. sets three server options, `@tx-bin` (the `tx` binary, never a plugin's program), `@tx-data`
   and `@tx-arguments` (the flags every `tx` a key runs must carry), puts `TX_BIN` in the
   server's environment (what a plugin program started from a key or a pane runs `tx` with), and
   sets one `@tx-key-<plugin>-<action>` option per entry of every fragment's plugin's key table
   (see [Keys](#keys));
2. sources `conf/tmux.conf` and the theme: options only (mouse, vi copy mode, status line,
   history, clipboard, indexing); no key of tmux's own is changed;
3. runs `keys/ide.tmux` (pane navigation, paging, window cycling, pane selection) and then every
   other `keys/<id>.tmux`, by name; each binds the keys its options hold;
4. sources your own `<config>/tmux.conf` (`~/.config/tx-ide/tmux.conf`) if it exists
   (`source-file -q`).

tmux runs each fragment to its end before it reads the next line, so your file comes last and
wins, on server start and on every `tx update`.
Each fragment is run through the interpreter its first line names (`#!/usr/bin/env bash` gives
`run-shell "/usr/bin/env bash <data>/keys/ide.tmux"`), never executed as a file: on macOS the first
direct run of a freshly written executable is assessed by the system one file at a time, so the
twelve fragments of a first server start after `tx update` cost seconds; through an interpreter
they cost milliseconds. A fragment without a `#!` first line is run as a file.

Because paths are spliced into tmux config lines, the updater refuses a binary, data or config
path containing whitespace, quotes, `$`, `#`, `;`, a backslash or a backtick. It also refuses,
naming both settings and before it writes anything, one key set for two actions in one table
(`prefix+p is set for both plugins.picker.settings.keys.open and …`); `c-t` and `C-t` count as
the same key. With one directory as both config and data, `tx update` refuses before it publishes
anything when your own `tmux.conf` or `nvim/tx.lua` is there, since that is where tx writes and
prunes its own files.

**Reconciliation.** Each key fragment declares, in comments near its top, the actions it binds in
the prefix table (`# keys: open`) and in the root table (`# root-keys: …`), and any global options
or hooks it sets (`# unset: …`, `# unhook: …`). The updater looks each action up in the plugin's
key table and records the tmux keys in `<data>/keys/.installed` (the `keymap` plugin reads the
prefix keys there to tell tx's keys from tmux's). On a running server it compares the previous
record with the new one, unbinds every key that is no longer bound (a key you moved, a key you set
to `null`, every key of a now disabled plugin), clears options and removes hooks of a disabled
plugin, writes the new record, re-sources the configuration and prints `reloaded`. An error in
your own `tmux.conf` stops the reload with tmux's own message, file and line; what came before it
is applied and recorded. A server set up before `.installed` existed, or before it recorded the
root-table keys, gets a hint to restart.

**Retiring.** When `ide` itself is disabled, or leaves the shipped plugins, nothing composes the
config any more, so `tx update` has the plugin take it back: it runs the same program with
`TX_PLUGIN_RETIRE=1`. If `<data>/keys/.installed` exists, the retire writes core's default
`tmux.conf` back, unbinds, unsets and unhooks in a running server everything the record lists
(whichever plugin's fragment declared it), unsets what the composed config itself had set
(`@tx-bin`, `@tx-data`, `@tx-arguments`, every `@tx-key-…` option, and `TX_BIN` in the server's
environment), loads the new config, removes the record, and prints `ide: tmux.conf reset to the
core default`. With a server running it adds `ide: restart the server to drop the rest of the
IDE's look`: the theme, the status line, `User0..8` and the `tx-attach-to` table were set without
being declared, and stay until the server restarts. Without the record the retire does and prints nothing, so the next
`tx update` is silent. A server that lived through a retire is a plain core server again:
enabling `ide` and updating composes the config anew and prints `reloaded`, without the hint to
restart.

## Views

A **view** is any tmux session on tx's server that is not an agent: the `home` base, editors,
diffviews, code tours, shells, the resident timeline, graph and board, the tutor and mockup viewers. A view has no
core record and never shows in `tx ls`. Its fields are tmux session options:

- `@tx-view` is `1` on every view; `@tx-view-kind` is one of `home`, `editor`, `diff`, `tour`,
  `shell`, `timeline`, `graph`, `tutor`, `mockup`, `board`.
- `@tx-view-parent` is the id of the agent it belongs to; `@tx-view-bound` the artifact it shows;
  `@tx-view-tags` and `@tx-view-created` complete it.

Because these live in tmux only, nothing goes stale when the server dies. Views are managed with
`tx ui view open <name> --kind <kind> [--parent <agent>] [--cwd <dir>] [-- program …]`,
`tx ui view ls` (which first closes views whose parent agent is gone) and
`tx ui view close <name>`. Closing and renaming go through a guard that refuses any session
carrying `@tx-id`: an agent is only ever stopped through core. This is the way to run a shell or
any non-LLM program on tx's server; `tx create` starts agents only.

## Nested attaches

Views host other sessions. A **nested attach** runs `TMUX= tmux -S <socket> attach -t <target>`
inside a view's pane, so the agent's terminal appears inside the view with the view's status bar
and border around it. The picker's Enter, the home screen's Enter and the artifacts menu all work
this way. Pane-navigation keys (`C-h`/`C-j`/`C-k`/`C-l`) bubble out of nested sessions, and the
pane border shows the nested session's name and tags.

`tx ui attach` from a bare terminal attaches to `home`. Inside another tmux whose socket is not
tx's it nests and warns that the outer prefix wins (press it twice). Inside tx's own server it
switches the current client to `home` instead.

## Showing a session: focus, attach-to, popup

A plugin that lets the operator pick a session shows it through tx-client, either with
`Ui::show_session(client, show, session, pane)` in-process or with
`tx _show <client> focus|attach-to|popup <session> [<pane>]`. The timeline and prefix+a do;
prefix+a titles its popup with `Ui::show_popup`.

- **focus** moves the terminal to the pane already showing the session.
- **attach-to** lets the operator pick the pane. The `tx-attach-to` key table (in `keys/ide.tmux`)
  takes over the terminal's keys with the window's active border turned gold. h/j/k/l, the
  arrows, a pane number or a click select a pane, starting on `pane`. Enter attaches the session
  there (`tx _attach-into`, which never replaces an agent's own pane), and Esc or q cancel.
- **popup** nest-attaches the session in a popup over the terminal.

`client` may be a nested client: the terminal it is shown on is the one that moves. All three are
safe to call from inside the caller's own popup. The popup waits in a background `run-shell` until
the caller's popup is gone, because a client shows one popup at a time.

## Keys

tx ships tmux's own keys unchanged: the prefix is `C-b`, and splits, layouts, copy mode and the
rest are tmux's defaults. On top of them each plugin binds the keys of its functions, and
disabling the plugin removes them. These are the defaults (`plugin` · `keys.<action>`):

| keys | does | setting |
|---|---|---|
| prefix+m | the home screen in a popup | `home` · `open` |
| prefix+t | the session picker; Enter nests into the session | `picker` · `open` |
| prefix+n | new agent: the form, then the agent | `spawn` · `new_agent` |
| prefix+e | edit the session's name, tags and group | `edit` · `edit` |
| prefix+X | `tx kill` the agent or view in the pane, after a confirmation | `kill` · `kill` |
| prefix+f | fork the agent in the pane into a new worktree | `chat` · `fork` |
| prefix+s | refresh labels, then tmux's session tree | `labels` · `sessions` |
| prefix+a | the artifacts an agent touched; in an artifact's view, back to the agent | `artifacts` · `menu` |
| prefix+T | the resident timeline in a popup | `timeline` · `open` |
| prefix+g | the resident agent graph in a popup | `graph` · `open` |
| prefix+b | the resident GitHub issue board of the pane's checkout in a popup | `board` · `open` |
| prefix+P | the plugins, to switch on and off, in a popup | `switch` · `open` |
| prefix+Space | the next agent that needs you: its pane, else a popup | `attention` · `next` |
| prefix+? | a keyboard of the prefix keys: tx's, tmux's and free ones | `keymap` · `open` |
| prefix+/ | one line to the assistant, with the focus envelope | `assistant` · `ask` |
| C-h C-j C-k C-l | move between nvim splits and panes, out of nested sessions too; the prefix and then the key sends the key itself | `ide` · `pane_left`, `pane_down`, `pane_up`, `pane_right` |
| C-u C-d | page an agent's scrollback (elsewhere the key itself) | `ide` · `scroll_up`, `scroll_down` |
| C-Tab C-S-Tab | next / previous window | `ide` · `window_next`, `window_previous` |
| M-1 … M-9 | select pane 1-9 | `ide` · `select_pane` (`M-`: modifiers only, the digit is added) |

Several of these replace what tmux binds there by default (`/` describe a key, `f` find window,
`m` mark pane, `s` session tree, `t` clock, `n` next window, `Space` next layout, `?` list keys);
the `ide` keys work without the prefix, so a shell or agent in a pane never receives them.
`tx config` lists every key with its current value (`tx config --defaults` with a comment on each),
and the home screen's `?` and prefix+? show the prefix keys live.

### Moving or dropping a key

In `~/.config/tx-ide/config.yaml` (`tx config --path`), write only what you change, then run
`tx update`:

```yaml
plugins:
  picker: { settings: { keys: { open: p } } }    # the picker moves to prefix+p
  spawn:  { settings: { keys: { new_agent: null } } }   # prefix+n is tmux's next-window again*
  ide:
    settings:
      keys:
        pane_right: null          # C-l clears the screen in your shell again
        select_pane: C-M-         # C-M-1 … C-M-9 instead of M-1 … M-9
```

A key is `t`, `C-t`, `M-Left`, `F5`, `C-S-Tab` and the like: modifiers (`C-`, `M-`, `S-`) and one
character or a tmux key name. An unknown action, or a value that is not a key, stops every `tx`
command with an error that names the plugin and the setting. `tx update` on a running server
unbinds the old key at once.

\* What tmux had on that key before tx took it comes back only when the server restarts (the last
session ends, or `tmux -S <socket> kill-server`); until then the key does nothing.

### Your own tmux.conf

Everything else is tmux's own language in `~/.config/tx-ide/tmux.conf` (`<config>/tmux.conf`). tx
never writes it; the composed config sources it last, so anything it sets or binds wins over what
tx ships, including a tx key:

```tmux
set -g prefix C-a                 # your prefix
unbind C-b
bind C-a send-prefix
bind | split-window -h -c "#{pane_current_path}"
bind t display-message "mine"     # takes prefix+t from the picker
```

Keys tx re-sends (the prefix before a nav key, the artifacts key into a nested session) use
whatever prefix is set when you press them, so changing the prefix here needs nothing else. A line
you delete from the file stays in effect until the server restarts.
