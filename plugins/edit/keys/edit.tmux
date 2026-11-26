#!/usr/bin/env bash
# keys: edit
# tx key plugin `edit`: edits the name, tags and group of the tx session in the active
# pane, in a popup over that pane. Run by `run-shell` from the server's tmux.conf, which
# `tx update` writes; @tx-data names the directory holding bin/tmux-edit-tx-session.
#
# Composes a tmux.conf fragment and source-file's it, so tmux's own parser handles the quoting.
# The user's own <config>/tmux.conf is sourced after every fragment, so anything it re-binds wins.
set -u

DATA=$(tmux show-option -gv @tx-data 2>/dev/null)
[ -n "$DATA" ] || exit 0

# The key is the plugin's `keys.edit` setting, which `tx update` writes into this option;
# empty when the user unbound it.
KEY=$(tmux show-option -gqv @tx-key-edit-edit)
[ -n "$KEY" ] || exit 0

CONF=$(mktemp -t tx-edit-keys.XXXXXX)
trap 'rm -f "$CONF"' EXIT

cat >>"$CONF" <<EOF2
bind -N "Rename or retag this agent" '$KEY' run-shell -b "$DATA/bin/tmux-edit-tx-session '#{pane_id}' '#{client_name}'"
EOF2

tmux source-file "$CONF"
