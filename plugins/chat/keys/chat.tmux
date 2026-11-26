#!/usr/bin/env bash
# keys: fork
# tx key plugin `chat`: forks the tx session in the active pane (a popup asks for the
# new name). Run by `run-shell` from the server's tmux.conf, which `tx update` writes;
# @tx-data names the directory holding bin/tmux-fork-tx-session.
#
# Composes a tmux.conf fragment and source-file's it, so tmux's own parser handles the quoting.
# The user's own <config>/tmux.conf is sourced after every fragment, so anything it re-binds wins.
set -u

DATA=$(tmux show-option -gv @tx-data 2>/dev/null)
[ -n "$DATA" ] || exit 0

# The key is the plugin's `keys.fork` setting, which `tx update` writes into this option;
# empty when the user unbound it.
KEY=$(tmux show-option -gqv @tx-key-chat-fork)
[ -n "$KEY" ] || exit 0

CONF=$(mktemp -t tx-chat-keys.XXXXXX)
trap 'rm -f "$CONF"' EXIT

cat >>"$CONF" <<EOF2
bind -N "Fork this agent" '$KEY' run-shell -b "$DATA/bin/tmux-fork-tx-session '#{pane_id}' '#{client_name}'"
EOF2

tmux source-file "$CONF"
