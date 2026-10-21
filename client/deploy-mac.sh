#!/usr/bin/env bash
# Copy the Rust client sources to the Mac, build them there and refresh Mindmap.app.
#   ./deploy-mac.sh [mac-host] [local-source]
set -euo pipefail
MAC="${1:-macbook}"
SRC="${2:-$PWD/client-src}"
APP="${APP:-~/Applications/Mindmap.app}"

rsync -a "$SRC/crates/ui-egui/src/" "$MAC:code/figcraft/crates/ui-egui/src/"
rsync -a "$SRC/apps/figcraft/src/" "$MAC:code/figcraft/apps/figcraft/src/"

ssh "$MAC" "cd ~/code/figcraft && set -o pipefail && cargo build -p figcraft 2>&1 | tail -3"
ssh "$MAC" "cp ~/code/figcraft/target/debug/figcraft $APP/Contents/MacOS/Mindmap && codesign --force --sign - $APP >/dev/null 2>&1"
echo "deployed $APP"
