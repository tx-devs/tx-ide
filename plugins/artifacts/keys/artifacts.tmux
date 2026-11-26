#!/usr/bin/env bash
# keys: menu
# tx key plugin `artifacts`: opens the artifacts menu of the tx session the key fired
# in. While the pane hosts a `tmux` client the key is re-sent one level down, so the bind that
# finally runs sees the NESTED session: `#{session_name}` is that session and `#{client_name}`
# the nested client, and the menu paints inside the pane. A pick is shown through tx-client's
# `show` (attach-to into a pane you pick, or a popup); in a view the key goes back to the
# controlling session. The re-send is `send-prefix` and the key: tmux reads the prefix when the
# key is pressed, so a prefix the user's own tmux.conf sets (sourced after this) is the one sent,
# as for the nav keys. Run by `run-shell` from the server's tmux.conf, which `tx update` writes;
# @tx-data names the directory holding bin/tmux-artifact-menu.
#
# Composes a tmux.conf fragment and source-file's it, so tmux's own parser handles the quoting.
# The user's own <config>/tmux.conf is sourced after every fragment, so anything it re-binds wins.
set -u

DATA=$(tmux show-option -gv @tx-data 2>/dev/null)
[ -n "$DATA" ] || exit 0

# The key is the plugin's `keys.menu` setting, which `tx update` writes into this option;
# empty when the user unbound it.
KEY=$(tmux show-option -gqv @tx-key-artifacts-menu)
[ -n "$KEY" ] || exit 0

CONF=$(mktemp -t tx-artifacts-keys.XXXXXX)
trap 'rm -f "$CONF"' EXIT

cat >>"$CONF" <<EOF2
bind -N "Artifacts of this session" '$KEY' if -F '#{==:#{pane_current_command},tmux}' {
  send-prefix
  send-keys '$KEY'
} {
  run-shell -b "$DATA/bin/tmux-artifact-menu '#{session_name}' '#{client_name}'"
}
EOF2

tmux source-file "$CONF"
