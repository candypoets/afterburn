#!/usr/bin/env bash
# The private Herdr daemon for mindmap-server; the user never runs this.
#
#   ./private-herdr.sh                    start the headless daemon (foreground; supervise it)
#   ./private-herdr.sh cli agent list     talk to it with the same isolation
#
# Isolation, learned the hard way:
#   - every inherited HERDR_* variable must be unset; HERDR_SOCKET_PATH overrides
#     a session and would point us at the user's daemon
#   - our own XDG_CONFIG_HOME/XDG_STATE_HOME keep sockets, sessions and state
#     inside our data directory
#   - a named session gives the daemon its own socket and workspace list
#   - `herdr server` is the headless mode; stop it only through `cli server stop`,
#     which carries this environment, or the user's daemon is the one that stops
set -euo pipefail
cd "$(dirname "$0")"
ROOT="${HERDR_PRIVATE_HOME:-$PWD/data/agents}"
SESSION="${HERDR_PRIVATE_SESSION:-mindmap}"
mkdir -p "$ROOT/config" "$ROOT/state"

isolated=(
  env
  -u HERDR_SOCKET_PATH -u HERDR_ENV -u HERDR_PANE_ID -u HERDR_TAB_ID
  -u HERDR_WORKSPACE_ID -u HERDR_BIN_PATH
  XDG_CONFIG_HOME="$ROOT/config"
  XDG_STATE_HOME="$ROOT/state"
  HERDR_SESSION="$SESSION"
)

if [[ "${1:-}" == "cli" ]]; then
  shift
  exec "${isolated[@]}" herdr --session "$SESSION" "$@"
fi
exec "${isolated[@]}" herdr --session "$SESSION" server
