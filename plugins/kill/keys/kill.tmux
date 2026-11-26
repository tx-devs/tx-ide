#!/usr/bin/env bash
# keys: kill
# tx key plugin `kill`: confirms, then ends the agent or view in the active pane through
# `tx kill`, so the record stays honest. Run by `run-shell` from the server's tmux.conf, which
# `tx update` writes; @tx-data names the directory holding bin/tmux-kill-tx-session.
#
# Composes a tmux.conf fragment and source-file's it, so tmux's own parser handles the quoting.
# The user's own <config>/tmux.conf is sourced after every fragment, so anything it re-binds wins.
set -u

DATA=$(tmux show-option -gv @tx-data 2>/dev/null)
[ -n "$DATA" ] || exit 0

# The key is the plugin's `keys.kill` setting, which `tx update` writes into this option;
# empty when the user unbound it.
KEY=$(tmux show-option -gqv @tx-key-kill-kill)
[ -n "$KEY" ] || exit 0

CONF=$(mktemp -t tx-kill-keys.XXXXXX)
trap 'rm -f "$CONF"' EXIT

cat >>"$CONF" <<EOF2
bind -N "Close the agent or view in this pane" '$KEY' run-shell "$DATA/bin/tmux-kill-tx-session '#{pane_id}' '#{session_name}'"
EOF2

tmux source-file "$CONF"
