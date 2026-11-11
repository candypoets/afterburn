#!/usr/bin/env bash
# Build and package Afterburn for Linux (the app plus the headless node it installs on a repo host):
#
#   $DIST/afterburn-linux-<arch>.AppImage   any distro with glibc >= the build host's
#   $DIST/afterburn-linux-<arch>.tar.gz     plain tree: bin/afterburn, bin/afterburn-node
#
# Usage: client/build-linux.sh [--skip-build] [--formats "appimage tar"]
# Env:   PROFILE=release|debug DIST=... VERSION=... APPIMAGE_TOOL_DIR=...
#
# Needs: cargo, and (for the AppImage) curl plus a runnable appimagetool; it is downloaded into
# APPIMAGE_TOOL_DIR when missing. Build on the oldest glibc you support (CI: Ubuntu 22.04).
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"

SKIP_BUILD=0
FORMATS="appimage tar"
while [ $# -gt 0 ]; do
  case "$1" in
    --skip-build) SKIP_BUILD=1; shift ;;
    --formats) FORMATS="$2"; shift 2 ;;
    -h | --help) sed -n '2,12p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

[[ "$(uname -s)" == Linux ]] || { echo 'Build the Linux packages on Linux.' >&2; exit 1; }
case "$(uname -m)" in
  x86_64) ARCH=x64; APPIMAGE_ARCH=x86_64 ;;
  aarch64 | arm64) ARCH=arm64; APPIMAGE_ARCH=aarch64 ;;
  *) echo 'Unsupported Linux architecture' >&2; exit 1 ;;
esac
[[ -z "${CARGO_BUILD_TARGET:-}" ]] || { echo 'Native packaging does not support CARGO_BUILD_TARGET.' >&2; exit 1; }
PROFILE="${PROFILE:-release}"
case "$PROFILE" in release) CARGO_PROFILE=release ;; debug) CARGO_PROFILE=dev ;; *) echo 'PROFILE must be release or debug' >&2; exit 1 ;; esac
VERSION="${VERSION:-$(awk '
    /^\[/ { in_pkg = ($0 == "[package]") ; next }
    in_pkg && $1 == "version" { gsub(/[" ]/, "", $3); print $3; exit }
  ' "$ROOT/service/Cargo.toml")}"
[[ -n "$VERSION" ]] || { echo 'Could not read the Afterburn version from service/Cargo.toml.' >&2; exit 1; }
DIST="${DIST:-$ROOT/dist/release}"
mkdir -p "$DIST"
DIST="$(cd "$DIST" && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
BASENAME="afterburn-linux-$ARCH"
BIN="$TARGET_DIR/$CARGO_PROFILE"

echo "==> Afterburn $VERSION for linux-$ARCH ($FORMATS)"
if [[ "$SKIP_BUILD" == 0 ]]; then
  (cd "$ROOT" && cargo build --locked --target-dir "$TARGET_DIR" -p figcraft -p afterburn-node --profile "$CARGO_PROFILE")
fi
[[ -x "$BIN/figcraft" && -x "$BIN/afterburn-node" ]] || {
  echo "Missing $BIN/figcraft or $BIN/afterburn-node; build without --skip-build." >&2
  exit 1
}

WORK="$TARGET_DIR/linux-package"
APPDIR="$WORK/Afterburn.AppDir"
STAGE="$WORK/root"
rm -rf "$WORK"
install -Dm755 "$BIN/figcraft" "$STAGE/bin/afterburn"
install -Dm755 "$BIN/afterburn-node" "$STAGE/bin/afterburn-node"

for format in $FORMATS; do
  case "$format" in
    tar)
      OUT="$DIST/$BASENAME.tar.gz"
      rm -f "$OUT"
      tar -C "$STAGE" -czf "$OUT" .
      printf 'Packaged %s\n' "$OUT"
      ;;
    appimage)
      install -d "$APPDIR/usr/bin" "$APPDIR/usr/share/applications"
      install -m755 "$STAGE/bin/afterburn" "$STAGE/bin/afterburn-node" "$APPDIR/usr/bin/"
      install -m644 "$HERE/icon-1024.png" "$APPDIR/afterburn.png"
      cp "$APPDIR/afterburn.png" "$APPDIR/.DirIcon"
      cat >"$APPDIR/afterburn.desktop" <<'DESKTOP'
[Desktop Entry]
Type=Application
Name=Afterburn
Comment=The canvas for coding agents
Exec=afterburn %F
Icon=afterburn
Categories=Development;Graphics;
Terminal=false
StartupWMClass=afterburn
DESKTOP
      install -m644 "$APPDIR/afterburn.desktop" "$APPDIR/usr/share/applications/afterburn.desktop"
      cat >"$APPDIR/AppRun" <<'APPRUN'
#!/bin/sh
# Runs the app from inside the mounted AppImage. $APPDIR is set by the AppImage runtime.
HERE="${APPDIR:-$(dirname "$(readlink -f "$0")")}"
exec "$HERE/usr/bin/afterburn" "$@"
APPRUN
      chmod +x "$APPDIR/AppRun"
      TOOL="${APPIMAGE_TOOL_DIR:-$TARGET_DIR}/appimagetool-$APPIMAGE_ARCH.AppImage"
      if [[ ! -x "$TOOL" ]]; then
        mkdir -p "$(dirname "$TOOL")"
        curl -fsSL -o "$TOOL" \
          "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-$APPIMAGE_ARCH.AppImage"
        chmod +x "$TOOL"
      fi
      OUT="$DIST/$BASENAME.AppImage"
      rm -f "$OUT"
      # The tool is itself an AppImage; extract-and-run keeps this working on hosts without FUSE.
      ARCH="$APPIMAGE_ARCH" APPIMAGE_EXTRACT_AND_RUN=1 VERSION="$VERSION" \
        "$TOOL" --no-appstream "$APPDIR" "$OUT" >/dev/null
      chmod +x "$OUT"
      printf 'Packaged %s\n' "$OUT"
      ;;
    *) echo "unknown format: $format" >&2; exit 2 ;;
  esac
done

(cd "$DIST" && sha256sum "$BASENAME".* >"$BASENAME.sha256")
printf 'Wrote %s\n' "$DIST/$BASENAME.sha256"
