#!/usr/bin/env bash
# keys: open
# tx key plugin `switch`: lists the plugins in a popup; space switches one, Enter writes
# config.yaml and runs `tx update`. Run by `run-shell` from the server's tmux.conf, which
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
KEY=$(tmux show-option -gqv @tx-key-switch-open)
[ -n "$KEY" ] || exit 0

CONF=$(mktemp -t tx-switch-keys.XXXXXX)
trap 'rm -f "$CONF"' EXIT

cat >>"$CONF" <<EOF2
bind -N "Turn plugins on or off" '$KEY' display-popup -E -w 60 -h 30 -x C -y C -T '#[#{@tx-accent-style}] #{@tx-mark}#[default] · plugins ' "$TX switch"
EOF2

tmux source-file "$CONF"
