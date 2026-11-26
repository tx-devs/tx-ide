#!/usr/bin/env bash
# keys: open
# tx key plugin `keymap`: opens a keyboard of the prefix shortcuts in a popup, at the
# picker's size and place. tx keys are blue, tmux's own grey, free keys dark; pressing a key
# shows what it does. Esc closes it. Run by `run-shell` from the server's tmux.conf, which
# `tx update` writes.
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
KEY=$(tmux show-option -gqv @tx-key-keymap-open)
[ -n "$KEY" ] || exit 0

CONF=$(mktemp -t tx-keymap-keys.XXXXXX)
trap 'rm -f "$CONF"' EXIT

cat >>"$CONF" <<EOF2
bind -N "Show the shortcuts" '$KEY' display-popup -E -w 118 -h 30 -x C -y 1 -T '#[#{@tx-accent-style}] #{@tx-mark}#[default] · keys ' "$TX keymap"
EOF2

tmux source-file "$CONF"
