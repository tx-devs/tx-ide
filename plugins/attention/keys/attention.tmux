#!/usr/bin/env bash
# keys: Space
# unset: status-left status-left-length
# unhook: pane-focus-in client-focus-in
# tx key plugin `attention`: the agents that need the operator. Run by `run-shell` from the
# server's tmux.conf, which `tx update` writes.
#
#   prefix+Space    go to the first agent that needs you (`tx attention next`): the pane that
#                   shows it, else a popup
#   status-left     the marked agents: ▲ waiting on you, ○ finished
#   focus hooks     a finished agent stops needing you once you look at it
#
# A mark is the session option @tx-attention, `<need> <since>`, set and taken back by the
# plugin's event sink (plugins/attention/src/marks.rs, kept in LOCKSTEP). Everything here reads
# it with formats and tmux commands alone: no program runs per redraw or per focus change.
#
# Composes a tmux.conf fragment and source-file's it, so tmux's own parser handles the quoting.
# Anything the user re-binds later in their own tmux.conf wins, since tmux is last-write-wins.
set -u

TX_BIN=$(tmux show-option -gv @tx-bin 2>/dev/null)
ARGUMENTS=$(tmux show-option -gv @tx-arguments 2>/dev/null)
[ -n "$ARGUMENTS" ] || exit 0
[ -n "$TX_BIN" ] || exit 0
TX="$TX_BIN $ARGUMENTS"

CONF=$(mktemp -t tx-attention-keys.XXXXXX)
trap 'rm -f "$CONF"' EXIT

# --- Status line ---
# One entry per marked session, in tmux's session order; nothing when no agent is marked.
# conf/tmux.conf leaves status-left empty and zero wide; this fragment runs after it. Hex
# literals duplicate `tx_client::palette` (tx-client/src/palette.rs):
#   WARN     #e0af68   (waiting, the graph's "needs you")
#   DIM_FG   #a9b1d6   (finished, the status line's own text)
WAITING='#[fg=#e0af68#,bold] ▲ #{session_name}#[default]'
FINISHED='#[fg=#a9b1d6] ○ #{session_name}#[default]'
MARKS="#{S:#{?#{m:waiting *,#{@tx-attention}},$WAITING,#{?#{m:finished *,#{@tx-attention}},$FINISHED,}}}"

# --- Focus hooks ---
# Looking at a finished agent takes its mark back; a waiting one keeps it until it is answered.
# pane-focus-in fires in the agent's session when a pane starts showing it and when a pane
# that showed it before is selected again; client-focus-in covers the first selection of a
# pane whose nested attach was started while another pane had the focus.
SEEN="if -F '#{m:finished *,#{@tx-attention}}' 'set -u @tx-attention'"

cat >>"$CONF" <<EOF2
bind -N "Agents waiting for you" Space run-shell -b "$TX attention next '#{client_name}' >/dev/null 2>&1 || true"
set -g status-left-length 80
set -g status-left "$MARKS"
set-hook -g pane-focus-in "$SEEN"
set-hook -g client-focus-in "$SEEN"
EOF2

tmux source-file "$CONF"
