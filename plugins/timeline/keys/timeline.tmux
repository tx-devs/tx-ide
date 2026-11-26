#!/usr/bin/env bash
# keys: open
# tx key plugin `timeline`: attaches the resident timeline in a popup (`_timeline-open`);
# q detaches it while the resident (and its transcript cache) stays warm. On a lane, through
# tx-client's `show`: Enter focuses the pane showing its agent, a picks a pane to attach it into
# (attach-to, the `tx-attach-to` table in keys/ide.tmux), p opens it in a popup. Meant for the
# home view.
# Run by `run-shell` from the server's tmux.conf, which `tx update` writes.
#
# Composes a tmux.conf fragment and source-file's it, so tmux's own parser handles the quoting.
# The user's own <config>/tmux.conf is sourced after every fragment, so anything it re-binds wins.
set -u

TX_BIN=$(tmux show-option -gv @tx-bin 2>/dev/null)
ARGUMENTS=$(tmux show-option -gv @tx-arguments 2>/dev/null)
[ -n "$ARGUMENTS" ] || exit 0
[ -n "$TX_BIN" ] || exit 0
TX="$TX_BIN $ARGUMENTS"

# The key is the plugin's `keys.open` setting, which `tx update` writes into this option;
# empty when the user unbound it.
KEY=$(tmux show-option -gqv @tx-key-timeline-open)
[ -n "$KEY" ] || exit 0

CONF=$(mktemp -t tx-timeline-keys.XXXXXX)
trap 'rm -f "$CONF"' EXIT

cat >>"$CONF" <<EOF2
bind -N "Activity timeline" '$KEY' run-shell -b "$TX _timeline-open '#{pane_id}' '#{client_name}' >/dev/null 2>&1 || true"
EOF2

tmux source-file "$CONF"
