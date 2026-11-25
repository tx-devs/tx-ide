#!/usr/bin/env bash
# keys: ?
# tx key plugin `keymap`: prefix+? opens a keyboard of the prefix shortcuts in a popup, at the
# picker's size and place. tx keys are blue, tmux's own grey, free keys dark; pressing a key
# shows what it does. Esc closes it. Run by `run-shell` from the server's tmux.conf, which
# `tx update` writes.
#
# Composes a tmux.conf fragment and source-file's it, so tmux's own parser handles the quoting.
# Anything the user re-binds later in their own tmux.conf wins, since tmux is last-write-wins.
set -u

TX_BIN=$(tmux show-option -gv @tx-bin 2>/dev/null)
ARGUMENTS=$(tmux show-option -gv @tx-arguments 2>/dev/null)
[ -n "$ARGUMENTS" ] || exit 0
[ -n "$TX_BIN" ] || exit 0
TX="$TX_BIN $ARGUMENTS"

CONF=$(mktemp -t tx-keymap-keys.XXXXXX)
trap 'rm -f "$CONF"' EXIT

cat >>"$CONF" <<EOF2
bind -N "Show the shortcuts" ? display-popup -E -w 118 -h 30 -x C -y 1 -T '#[#{@tx-accent-style}] #{@tx-mark}#[default] · keys ' "$TX keymap"
EOF2

tmux source-file "$CONF"
