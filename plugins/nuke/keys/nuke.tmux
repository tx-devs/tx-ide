#!/usr/bin/env bash
# keys: Q
# tx key plugin `nuke`: prefix+Q confirms, then runs `tx nuke`: every agent stopped, the IDE's
# tmux server killed and started again, a new tx-assistant. Run by `run-shell` from the server's
# tmux.conf, which `tx update` writes; @tx-bin and @tx-arguments name the `tx` to run.
#
# Composes a tmux.conf fragment and source-file's it, so tmux's own parser handles the quoting.
# Anything the user re-binds later in their own tmux.conf wins, since tmux is last-write-wins.
set -u

TX="$(tmux show-option -gv @tx-bin 2>/dev/null) $(tmux show-option -gv @tx-arguments 2>/dev/null)"
[ -n "${TX// /}" ] || exit 0

CONF=$(mktemp -t tx-nuke-keys.XXXXXX)
trap 'rm -f "$CONF"' EXIT

cat >>"$CONF" <<EOF2
bind -N "Nuke the IDE: stop every agent, start again with a new tx-assistant" Q confirm-before -p "nuke the IDE? every agent stops (y/n)" "run-shell '$TX nuke 2>&1 >/dev/null'"
EOF2

tmux source-file "$CONF"
