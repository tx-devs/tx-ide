#!/usr/bin/env bash
# keys: open
# tx key plugin `picker`: opens the tx session picker in a popup. `tx pick` finds the
# pane to nest into by asking tmux directly (`display-message -p #{pane_id}`), so the bind plumbs
# nothing through. Run by `run-shell` from the server's tmux.conf, which `tx update` writes.
# The picker's TAB roles panel runs in this popup's own terminal (fzf hands it over), since a
# tmux popup cannot open inside another; its normal mode needs no tmux keys of its own.
#
# The popup is `-w 118`: wide enough for NAME + the 25-col LOCATION + STARTED/IDLE/ROLE + tag
# chips without wrapping, with ~5 cols of extra NAME headroom. Keep it in lockstep with
# `LOCATION_W` and `picker_namew` in tx-client's render.rs.
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
KEY=$(tmux show-option -gqv @tx-key-picker-open)
[ -n "$KEY" ] || exit 0

CONF=$(mktemp -t tx-picker-keys.XXXXXX)
trap 'rm -f "$CONF"' EXIT

cat >>"$CONF" <<EOF2
bind -N "Find an agent or view" '$KEY' display-popup -E -w 118 -h 30 -x C -y 1 -T '#[#{@tx-accent-style}] #{@tx-mark}#[default] · pick ' "$TX pick"
EOF2

tmux source-file "$CONF"
