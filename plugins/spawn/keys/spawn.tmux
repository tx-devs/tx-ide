#!/usr/bin/env bash
# keys: new_agent
# tx key plugin `spawn`: asks for a new agent's name, tags, prompt, project, engine,
# model, effort and agent in a popup on the firing client, then creates the agent session through
# `tx _new-agent`. Run by `run-shell` from the server's tmux.conf, which `tx update`
# writes; @tx-data names the directory holding bin/tmux-new-agent.
#
# Composes a tmux.conf fragment and source-file's it, so tmux's own parser handles the quoting.
# The user's own <config>/tmux.conf is sourced after every fragment, so anything it re-binds wins.
set -u

DATA=$(tmux show-option -gv @tx-data 2>/dev/null)
[ -n "$DATA" ] || exit 0

# The key is the plugin's `keys.new_agent` setting, which `tx update` writes into this option;
# empty when the user unbound it.
KEY=$(tmux show-option -gqv @tx-key-spawn-new_agent)
[ -n "$KEY" ] || exit 0

CONF=$(mktemp -t tx-spawn-keys.XXXXXX)
trap 'rm -f "$CONF"' EXIT

cat >>"$CONF" <<EOF2
bind -N "New agent" '$KEY' run-shell -b "$DATA/bin/tmux-new-agent '#{pane_id}' '#{client_name}'"
EOF2

tmux source-file "$CONF"
