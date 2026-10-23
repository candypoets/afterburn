#!/usr/bin/env bash
# Copy this repository's canonical build inputs into an isolated Mac staging directory.
# Usage: [NODE_ASSETS=...] [UPGRADE_NODE=1] ./client/deploy-mac.sh [ssh-alias]
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
MAC="${1:-macbook}"
[[ $# -le 1 ]] || { echo 'Usage: deploy-mac.sh [ssh-alias]; independent client sources are no longer supported.' >&2; exit 1; }
[[ "$MAC" =~ ^[a-zA-Z0-9_][a-zA-Z0-9_.@-]*$ ]] || { echo 'Use an SSH config alias or user@host; configure ports in SSH config.' >&2; exit 1; }
[[ -f "$ROOT/Cargo.toml" && -f "$ROOT/Cargo.lock" ]] || { echo 'Missing canonical root Cargo workspace/lockfile.' >&2; exit 1; }
STAGING="$(mktemp -d)"
trap 'rm -rf -- "$STAGING"' EXIT
# Include tracked files and new nonignored source files; never ship private state or legacy checkouts.
python3 - "$ROOT" "$STAGING/source" <<'PY'
import pathlib, shutil, subprocess, sys
root, destination = map(pathlib.Path, sys.argv[1:])
paths = subprocess.check_output(['git', '-C', str(root), 'ls-files', '--cached', '--others', '--exclude-standard', '-z']).split(b'\0')
for raw in paths:
    if not raw:
        continue
    relative = pathlib.Path(raw.decode())
    name = relative.as_posix()
    if not (name in ('Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'rust-toolchain') or name.startswith(('.cargo/', 'server/figcraft/', 'service/', 'client/'))):
        continue
    if any(part in ('target', '.git', '__pycache__', 'node_modules', 'data') for part in relative.parts):
        continue
    source = root / relative
    if source.is_file():
        target = destination / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, target)
PY
# Random remote staging avoids deleting/overlaying another checkout, including untracked files.
REMOTE="$(ssh "$MAC" 'mkdir -p "$HOME/.cache/mindmap/builds" && mktemp -d "$HOME/.cache/mindmap/builds/full.XXXXXXXX"')"
[[ "$REMOTE" == /*/.cache/mindmap/builds/full.* && "$REMOTE" != *$'\n'* ]] || { echo 'Unexpected remote staging directory.' >&2; exit 1; }
printf -v REMOTE_QUOTED '%q' "$REMOTE"
rsync -a -- "$STAGING/source/" "$MAC:$REMOTE_QUOTED/source/"
ASSETS=0
if [[ -n "${NODE_ASSETS:-}" ]]; then
  [[ -d "$NODE_ASSETS" ]] || { echo 'NODE_ASSETS must be a directory.' >&2; exit 1; }
  rsync -a -- "$NODE_ASSETS/" "$MAC:$REMOTE_QUOTED/assets/"
  ASSETS=1
fi
printf -v REMOTE_ARGS '%q ' "$REMOTE" "${APP:-Applications/Mindmap.app}" "${PROFILE:-release}" "${CARGO_TOOLCHAIN:-1.97.1}" "${INSTALL_NODE:-1}" "${UPGRADE_NODE:-0}" "$ASSETS"
ssh "$MAC" "bash -s -- $REMOTE_ARGS" <<'REMOTE_SCRIPT'
set -euo pipefail
STAGING="$1"
export APP="$2" PROFILE="$3" CARGO_TOOLCHAIN="$4" INSTALL_NODE="$5" UPGRADE_NODE="$6"
case "$APP" in
  '~/'*) APP="$HOME/${APP:2}" ;;
  /*) ;;
  *) APP="$HOME/$APP" ;;
esac
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"
# Preserve the root build cache across snapshots, without mixing sources.
export CARGO_TARGET_DIR="$HOME/.cache/mindmap/target"
if [[ "$7" == 1 ]]; then export NODE_ASSETS="$STAGING/assets"; fi
bash "$STAGING/source/client/build-mac.sh"
printf 'Canonical source snapshot retained at %s/source\n' "$STAGING"
REMOTE_SCRIPT
