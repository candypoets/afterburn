#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
LISTEN="${LISTEN:-0.0.0.0:47910}"
DATA="${DATA:-$PWD/data}"
exec "$PWD/figcraft/target/release/figcraft-server" --listen "$LISTEN" --data "$DATA" "$@"
