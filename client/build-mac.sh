#!/usr/bin/env bash
# Full native install: one canonical workspace build supplies the app and its standalone helper.
# APP=... PROFILE=release CARGO_TOOLCHAIN=1.97.1 [NODE_ASSETS=...] ./client/build-mac.sh
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
[[ $# == 0 ]] || { echo 'Usage: build-mac.sh (APP, PROFILE, CARGO_TOOLCHAIN, NODE_ASSETS, INSTALL_NODE, UPGRADE_NODE are environment options)' >&2; exit 1; }
[[ "$(uname -s)" == Darwin ]] || { echo 'Build the full Mac install on macOS.' >&2; exit 1; }
case "$(uname -m)" in
  arm64) PLATFORM=darwin-arm64 ;;
  x86_64) PLATFORM=darwin-x64 ;;
  *) echo 'Unsupported Mac architecture' >&2; exit 1 ;;
esac
[[ -z "${CARGO_BUILD_TARGET:-}" ]] || { echo 'Native packaging does not support CARGO_BUILD_TARGET.' >&2; exit 1; }
for command in cargo python3 sips iconutil codesign; do
  command -v "$command" >/dev/null || { echo "Missing required command: $command" >&2; exit 1; }
done
APP="${APP:-$HOME/Applications/Mindmap.app}"
[[ "$APP" == *.app ]] || { echo 'APP must name a .app bundle.' >&2; exit 1; }
mkdir -p "$(dirname "$APP")"
APP="$(cd "$(dirname "$APP")" && pwd)/$(basename "$APP")"
[[ ! -L "$APP" ]] || { echo 'APP must not be a symlink.' >&2; exit 1; }
if [[ -e "$APP" ]]; then
  python3 - "$APP" <<'PY'
import pathlib, plistlib, sys
app = pathlib.Path(sys.argv[1])
with (app / 'Contents/Info.plist').open('rb') as stream:
    if plistlib.load(stream).get('CFBundleIdentifier') != 'dev.mindmap.client':
        sys.exit('Refusing to replace a bundle that is not Mindmap.')
PY
fi
PROFILE="${PROFILE:-release}"
case "$PROFILE" in release) CARGO_PROFILE=release ;; debug) CARGO_PROFILE=dev ;; *) echo 'PROFILE must be release or debug' >&2; exit 1 ;; esac
case "${INSTALL_NODE:-1}:${UPGRADE_NODE:-0}" in 0:0|0:1|1:0|1:1) ;; *) echo 'INSTALL_NODE and UPGRADE_NODE must be 0 or 1.' >&2; exit 1 ;; esac
cd "$ROOT"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
# Do not split this invocation: app and native node must share one resolved dependency graph.
cargo "+${CARGO_TOOLCHAIN:-1.97.1}" build --locked --target-dir "$TARGET_DIR" -p figcraft -p mindmap-node --profile "$CARGO_PROFILE"
STAGING="$(mktemp -d "$(dirname "$APP")/.mindmap-build.XXXXXX")"
cleanup() {
  if [[ -d "$STAGING/previous.app" && ! -e "$APP" ]]; then
    mv "$STAGING/previous.app" "$APP" || { echo "Previous app retained at $STAGING/previous.app" >&2; return; }
  fi
  rm -rf -- "$STAGING"
}
trap cleanup EXIT
BUNDLE="$STAGING/Mindmap.app"
RESOURCES="$BUNDLE/Contents/Resources"
mkdir -p "$BUNDLE/Contents/MacOS" "$RESOURCES/node/$PLATFORM" "$STAGING/AppIcon.iconset"
cp "$TARGET_DIR/$PROFILE/figcraft" "$BUNDLE/Contents/MacOS/Mindmap"
cp "$TARGET_DIR/$PROFILE/mindmap-node" "$RESOURCES/node/$PLATFORM/mindmap-node"
cp "$ROOT/service/install-node.sh" "$ROOT/service/install-node.py" "$RESOURCES/"
python3 "$ROOT/service/node-assets.py" record --binary "$RESOURCES/node/$PLATFORM/mindmap-node" --platform "$PLATFORM" --output "$RESOURCES/node/$PLATFORM/build-info.json"
if [[ -n "${NODE_ASSETS:-}" ]]; then
  python3 "$ROOT/service/node-assets.py" copy --assets "$NODE_ASSETS" --build-info "$RESOURCES/node/$PLATFORM/build-info.json" --destination "$RESOURCES/node" --native-platform "$PLATFORM"
fi
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" "$HERE/icon-1024.png" --out "$STAGING/AppIcon.iconset/icon_${size}x${size}.png" >/dev/null
  sips -z "$((size*2))" "$((size*2))" "$HERE/icon-1024.png" --out "$STAGING/AppIcon.iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$STAGING/AppIcon.iconset" -o "$RESOURCES/AppIcon.icns"
python3 - "$BUNDLE" <<'PY'
import pathlib, plistlib, sys
bundle = pathlib.Path(sys.argv[1])
plist = {
    'CFBundleName': 'Mindmap', 'CFBundleDisplayName': 'Mindmap',
    'CFBundleExecutable': 'Mindmap', 'CFBundleIdentifier': 'dev.mindmap.client',
    'CFBundleIconFile': 'AppIcon', 'CFBundlePackageType': 'APPL',
    'CFBundleShortVersionString': '0.1.0', 'CFBundleVersion': '1',
    'LSMinimumSystemVersion': '12.0', 'NSHighResolutionCapable': True,
    'LSApplicationCategoryType': 'public.app-category.developer-tools',
}
with (bundle / 'Contents/Info.plist').open('wb') as stream:
    plistlib.dump(plist, stream)
PY
# Sign nested native executables before the containing bundle. Foreign ELF assets are data.
for binary in "$RESOURCES"/node/darwin-*/mindmap-node; do
  [[ -f "$binary" ]] || continue
  codesign --force --sign - "$binary"
done
# Signing can change Mach-O bytes; refresh their sidecar checksums without executing them.
python3 - "$RESOURCES/node" <<'PY'
import hashlib, json, pathlib, sys
for path in pathlib.Path(sys.argv[1]).glob('darwin-*/build-info.json'):
    info = json.loads(path.read_text())
    info['sha256'] = hashlib.sha256((path.parent / 'mindmap-node').read_bytes()).hexdigest()
    path.write_text(json.dumps(info, indent=2) + '\n')
PY
codesign --force --sign - "$BUNDLE"
codesign --verify --deep --strict "$BUNDLE"
if [[ -e "$APP" ]]; then mv "$APP" "$STAGING/previous.app"; fi
if ! mv "$BUNDLE" "$APP"; then
  if [[ -d "$STAGING/previous.app" ]]; then mv "$STAGING/previous.app" "$APP"; fi
  exit 1
fi
printf 'Installed app: %s\n' "$APP"
if [[ "${INSTALL_NODE:-1}" == 1 ]]; then
  if [[ "${UPGRADE_NODE:-0}" == 1 ]]; then
    "$ROOT/service/install-mac-node.sh" "$APP" --upgrade
  else
    "$ROOT/service/install-mac-node.sh" "$APP"
  fi
else
  echo 'Package-only mode: helper registration deferred (INSTALL_NODE=0).'
fi
