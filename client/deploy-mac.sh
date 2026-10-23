#!/usr/bin/env bash
# Reconstruct the complete tracked client, build on the Mac, and refresh Mindmap.app.
# Usage: ./deploy-mac.sh [mac-host] [optional-working-client-source]
# NODE_ASSETS=/directory/containing/darwin-arm64/linux-x64/... supplies node bundle assets.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
MAC="${1:-macbook}"
APP="${APP:-Applications/Mindmap.app}"
STAGING=""
trap 'if [[ -n "$STAGING" ]]; then rm -rf -- "$STAGING"; fi' EXIT

if [[ ! "$MAC" =~ ^[a-zA-Z0-9_][a-zA-Z0-9_.@-]*$ ]]; then
  echo 'Use an SSH config alias or user@host; configure ports in SSH config.' >&2
  exit 1
fi
if [[ -n "${2:-}" ]]; then
  SRC="$2"
else
  STAGING="$(mktemp -d)"
  SRC="$STAGING/source"
  mkdir -p "$SRC"
  rsync -a --exclude=.git --exclude=target "$ROOT/server/figcraft/" "$SRC/"
  python3 "$HERE/apply-source-overlay.py" "$SRC"
fi
[[ -f "$SRC/Cargo.toml" ]] || { echo 'Missing client Cargo.toml' >&2; exit 1; }
# Includes manifests, engine/collab fixes, locales, and all prior Mindmap source changes.
rsync -a --exclude=.git --exclude=target --exclude=node_modules "$SRC/" "$MAC:code/figcraft/"
ASSETS=0
if [[ -n "${NODE_ASSETS:-}" ]]; then
  [[ -d "$NODE_ASSETS" ]] || { echo 'NODE_ASSETS must be a directory' >&2; exit 1; }
  rsync -a "$NODE_ASSETS/" "$MAC:code/mindmap-node-assets/"
  ASSETS=1
fi
printf -v APP_QUOTED '%q' "$APP"
ssh "$MAC" "bash -s -- $APP_QUOTED $ASSETS" <<'REMOTE'
set -euo pipefail
APP_TARGET="$1"
case "$APP_TARGET" in
  '~/'*) APP_TARGET="$HOME/${APP_TARGET:2}" ;;
  /*) ;;
  *) APP_TARGET="$HOME/$APP_TARGET" ;;
esac
[[ -d "$APP_TARGET/Contents/MacOS" ]] || { echo 'Create Mindmap.app with build-mac.sh first' >&2; exit 1; }
if [[ "$2" == 1 ]]; then
  mkdir -p "$APP_TARGET/Contents/Resources/node"
  cp -R "$HOME/code/mindmap-node-assets/." "$APP_TARGET/Contents/Resources/node/"
fi
case "$(uname -m)" in
  arm64) PLATFORM=darwin-arm64 ;;
  x86_64) PLATFORM=darwin-x64 ;;
  *) echo 'Unsupported Mac architecture' >&2; exit 1 ;;
esac
[[ -x "$APP_TARGET/Contents/Resources/node/$PLATFORM/mindmap-node" ]] || {
  echo "Missing bundled node for $PLATFORM; provide NODE_ASSETS." >&2; exit 1;
}
cd "$HOME/code/figcraft"
cargo build -p figcraft
cp target/debug/figcraft "$APP_TARGET/Contents/MacOS/Mindmap"
codesign --force --sign - "$APP_TARGET"
printf 'Deployed %s\n' "$APP_TARGET"
REMOTE
