#!/usr/bin/env bash
# keys: open
# tx key plugin `board`: attaches the resident GitHub issue board of the pane's checkout
# in a popup (`_board-open`); q detaches it while the resident stays warm. / filters in GitHub's
# search language, L edits the issue's labels. On a card: s starts a developer agent on the
# issue, Enter focuses the pane of the agent working on it, a picks a pane to attach it into, p
# opens it in a popup; o opens the issue in the browser, c comments, n files a new issue, x
# closes or reopens, m assigns or unassigns you.
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
KEY=$(tmux show-option -gqv @tx-key-board-open)
[ -n "$KEY" ] || exit 0

CONF=$(mktemp -t tx-board-keys.XXXXXX)
trap 'rm -f "$CONF"' EXIT

cat >>"$CONF" <<EOF2
bind -N "GitHub issue board" '$KEY' run-shell -b "$TX _board-open '#{pane_id}' '#{client_name}' '#{pane_current_path}' >/dev/null 2>&1 || true"
EOF2

tmux source-file "$CONF"
