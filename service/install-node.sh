#!/usr/bin/env sh
# Portable entry point; keep install-node.py beside this file when copying over SSH.
set -eu
exec python3 "$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)/install-node.py" "$@"
