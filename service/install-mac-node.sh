#!/usr/bin/env bash
# Compatibility wrapper around the same installer used by the app and headless hosts.
# Usage: install-mac-node.sh [Mindmap.app] [--upgrade]
set -euo pipefail
[[ "$(uname -s)" == Darwin ]] || { echo 'Run this installer on macOS.' >&2; exit 1; }
HERE="$(cd "$(dirname "$0")" && pwd)"
APP="${1:-$HOME/Applications/Mindmap.app}"
if [[ $# -gt 0 ]]; then shift; fi
case "$(uname -m)" in
  arm64) PLATFORM=darwin-arm64 ;;
  x86_64) PLATFORM=darwin-x64 ;;
  *) echo 'Unsupported Mac architecture' >&2; exit 1 ;;
esac
NODE="$APP/Contents/Resources/node/$PLATFORM/mindmap-node"
[[ -x "$NODE" ]] || { echo 'Bundle the native Mindmap node first.' >&2; exit 1; }
BUILD_ID="$("$NODE" build-info | python3 -c 'import json,sys; print(json.load(sys.stdin)["build_id"])')"
exec "$HERE/install-node.sh" --binary "$NODE" --build-id "$BUILD_ID" --scope user "$@"
