#!/usr/bin/env bash
# keys: ask
# tx key plugin `assistant`: asks the tx-assistant from any pane. A command prompt takes
# one line; the bind parks it in the paste buffer `tx-assistant-input` (no shell quoting of the
# typed text) and `tx assistant ask --from-buffer --pane <firing pane>` reads it, warms the
# assistant and delivers it as the operator typing in that pane's session, behind the focus
# envelope (run-shell expands `#{pane_id}` against the client that pressed the key). Run by
# `run-shell` from the server's tmux.conf, which `tx update` writes.
#
# Composes a tmux.conf fragment and source-file's it, so tmux's own parser handles the `{ }`
# block. The user's own <config>/tmux.conf is sourced after every
# fragment, so anything it re-binds wins.
set -u

TX_BIN=$(tmux show-option -gv @tx-bin 2>/dev/null)
ARGUMENTS=$(tmux show-option -gv @tx-arguments 2>/dev/null)
[ -n "$ARGUMENTS" ] || exit 0
[ -n "$TX_BIN" ] || exit 0
TX="$TX_BIN $ARGUMENTS"

# The key is the plugin's `keys.ask` setting, which `tx update` writes into this option;
# empty when the user unbound it.
KEY=$(tmux show-option -gqv @tx-key-assistant-ask)
[ -n "$KEY" ] || exit 0

CONF=$(mktemp -t tx-assistant-keys.XXXXXX)
trap 'rm -f "$CONF"' EXIT

cat >>"$CONF" <<EOF2
bind -N "Ask the tx assistant" '$KEY' command-prompt -p "tx-assistant>" {
  set-buffer -b tx-assistant-input "%%"
  run-shell -b "$TX assistant ask --from-buffer --pane '#{pane_id}'"
}
EOF2

tmux source-file "$CONF"
