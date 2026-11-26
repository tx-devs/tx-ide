#!/usr/bin/env bash
# keys: next
# unset: status-left status-left-length @tx-attention-bar @tx-swarm
# unhook: pane-focus-in client-focus-in
# tx key plugin `attention`: the agents that need the operator. Run by `run-shell` from the
# server's tmux.conf, which `tx update` writes.
#
#   prefix+Space    (`keys.next`) go to the first agent that needs you (`tx attention next`): the pane that
#                   shows it, else a popup
#   status-left     the `tx` chip, then at most 2 marked agents and `+N` overflow
#                   (`@tx-attention-bar`), styled via `@tx-attention-style` / `@tx-muted-style`
#   @tx-swarm       the running agents counted by state, shown by the ide's status-right
#   focus hooks     a finished agent stops needing you once you look at it
#
# A mark is the session option @tx-attention, `<need> <since>`, set and taken back by the
# plugin's event sink (plugins/attention/src/marks.rs, kept in LOCKSTEP). `@tx-attention-bar`
# holds the capped summary; `#{E:@tx-attention-bar}` expands its live theme styles on redraw.
#
# Composes a tmux.conf fragment and source-file's it, so tmux's own parser handles the quoting.
# The user's own <config>/tmux.conf is sourced after every fragment, so anything it re-binds wins.
set -u

TX_BIN=$(tmux show-option -gv @tx-bin 2>/dev/null)
ARGUMENTS=$(tmux show-option -gv @tx-arguments 2>/dev/null)
[ -n "$ARGUMENTS" ] || exit 0
[ -n "$TX_BIN" ] || exit 0
TX="$TX_BIN $ARGUMENTS"

# The key is the plugin's `keys.next` setting, which `tx update` writes into this option;
# empty when the user unbound it.
KEY=$(tmux show-option -gqv @tx-key-attention-next)

CONF=$(mktemp -t tx-attention-keys.XXXXXX)
trap 'rm -f "$CONF"' EXIT

# --- Focus hooks ---
# Looking at a finished agent takes its mark back; a waiting one keeps it until it is answered.
# pane-focus-in fires in the agent's session when a pane starts showing it and when a pane
# that showed it before is selected again; client-focus-in covers the first selection of a
# pane whose nested attach was started while another pane had the focus.
REFRESH="run-shell -b \"$TX attention _refresh >/dev/null 2>&1 || true\""

if [ -n "$KEY" ]; then
  cat >>"$CONF" <<EOF2
bind -N "Agents waiting for you" '$KEY' run-shell -b "$TX attention next '#{client_name}' >/dev/null 2>&1 || true"
EOF2
fi

cat >>"$CONF" <<EOF2
set -g status-left-length 80
set -g status-left "#[#{@tx-chip-style}] #{@tx-mark} #[default]#{E:@tx-attention-bar}"
set-hook -g pane-focus-in {
  if -F '#{m:finished *,#{@tx-attention}}' {
    set -u @tx-attention
    $REFRESH
  }
}
set-hook -g client-focus-in {
  if -F '#{m:finished *,#{@tx-attention}}' {
    set -u @tx-attention
    $REFRESH
  }
}
$REFRESH
EOF2

tmux source-file "$CONF"
