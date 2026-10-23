#!/usr/bin/env bash
# Build/package a native headless node from the canonical root workspace; never installs it.
# CARGO_TOOLCHAIN=1.97.1 PROFILE=release ./service/build-node.sh [output-directory]
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
case "$(uname -s):$(uname -m)" in
  Darwin:arm64) PLATFORM=darwin-arm64; DEFAULT_TOOLCHAIN=1.97.1 ;;
  Darwin:x86_64) PLATFORM=darwin-x64; DEFAULT_TOOLCHAIN=1.97.1 ;;
  Linux:aarch64|Linux:arm64) PLATFORM=linux-arm64; DEFAULT_TOOLCHAIN=1.97.1 ;;
  Linux:x86_64) PLATFORM=linux-x64; DEFAULT_TOOLCHAIN=1.97.1 ;;
  *) echo 'Unsupported native platform' >&2; exit 1 ;;
esac
[[ $# -le 1 ]] || { echo 'Usage: build-node.sh [output-directory]' >&2; exit 1; }
[[ -z "${CARGO_BUILD_TARGET:-}" ]] || { echo 'Native packaging does not support CARGO_BUILD_TARGET.' >&2; exit 1; }
OUTPUT="${1:-$ROOT/build/node}"
mkdir -p "$OUTPUT"
OUTPUT="$(cd "$OUTPUT" && pwd)"
PROFILE="${PROFILE:-release}"
case "$PROFILE" in release) CARGO_PROFILE=release ;; debug) CARGO_PROFILE=dev ;; *) echo 'PROFILE must be release or debug' >&2; exit 1 ;; esac
cd "$ROOT"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
cargo "+${CARGO_TOOLCHAIN:-$DEFAULT_TOOLCHAIN}" build --locked --target-dir "$TARGET_DIR" -p mindmap-node --profile "$CARGO_PROFILE"
STAGING="$(mktemp -d "$OUTPUT/.node-build.XXXXXX")"
trap 'rm -rf -- "$STAGING"' EXIT
cp "$TARGET_DIR/$PROFILE/mindmap-node" "$STAGING/mindmap-node"
chmod 755 "$STAGING/mindmap-node"
python3 "$HERE/node-assets.py" record --binary "$STAGING/mindmap-node" --platform "$PLATFORM" --output "$STAGING/build-info.json"
mkdir -p "$OUTPUT/$PLATFORM"
mv "$STAGING/mindmap-node" "$OUTPUT/$PLATFORM/mindmap-node"
mv "$STAGING/build-info.json" "$OUTPUT/$PLATFORM/build-info.json"
cp "$HERE/install-node.sh" "$HERE/install-node.py" "$OUTPUT/"
printf 'Packaged native node: %s/%s\n' "$OUTPUT" "$PLATFORM"
