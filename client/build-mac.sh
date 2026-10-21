#!/usr/bin/env bash
# Build Mindmap.app from a FigCraft 0.7.0 checkout with this folder's patch applied.
#   ./build-mac.sh [checkout] [join-link] [control-port]
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
SRC="${1:-$HOME/code/figcraft}"
LINK="${2:-}"
PORT="${3:-49125}"
APP="${APP:-$HOME/Applications/Mindmap.app}"

cd "$SRC"
cargo build -p figcraft

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/debug/figcraft "$APP/Contents/MacOS/Mindmap"

TMP="$(mktemp -d)"
mkdir -p "$TMP/AppIcon.iconset"
for s in 16 32 128 256 512; do
  sips -z "$s" "$s" "$HERE/icon-1024.png" --out "$TMP/AppIcon.iconset/icon_${s}x${s}.png" >/dev/null
  sips -z "$((s*2))" "$((s*2))" "$HERE/icon-1024.png" --out "$TMP/AppIcon.iconset/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$TMP/AppIcon.iconset" -o "$APP/Contents/Resources/AppIcon.icns"
rm -rf "$TMP"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>Mindmap</string>
<key>CFBundleDisplayName</key><string>Mindmap</string>
<key>CFBundleExecutable</key><string>Mindmap</string>
<key>CFBundleIdentifier</key><string>dev.mindmap.client</string>
<key>CFBundleIconFile</key><string>AppIcon</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>0.7.0</string>
<key>CFBundleVersion</key><string>1</string>
<key>LSMinimumSystemVersion</key><string>12.0</string>
<key>NSHighResolutionCapable</key><true/>
<key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
</dict></plist>
PLIST
codesign --force --sign - "$APP" >/dev/null 2>&1 || true

if [[ -n "$LINK" ]]; then
  open -a "$APP" --args --map-mode --control "$PORT" "$LINK"
fi
echo "built $APP"
