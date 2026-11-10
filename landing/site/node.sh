#!/bin/sh
# Afterburn node installer: the app's node without the app.
#
#   curl -fsSL afterburn.nuts.cash/node.sh | sh
#
# Fetches the native node for this machine, checks it against the published SHA-256 and hands it
# to the same installer the app uses. The node is supervised by launchd (macOS) or systemd
# (Linux) and outlives the app.
#
# Environment:
#   AFTERBURN_NODE_BASE    asset base URL (default https://afterburn.nuts.cash/download/node)
#   AFTERBURN_NODE_SCOPE   user | system | manual (default: user, or system as root on Linux)
#   AFTERBURN_NODE_UPGRADE 1 to replace a supervised, idle node
set -eu

BASE=${AFTERBURN_NODE_BASE:-https://afterburn.nuts.cash/download/node}

die() {
  printf 'afterburn node: %s\n' "$*" >&2
  exit 1
}

case "$(uname -s 2>/dev/null)" in
  Darwin) os=darwin ;;
  Linux) os=linux ;;
  *) die "unsupported system $(uname -s); the node builds for macOS and Linux" ;;
esac
case "$(uname -m 2>/dev/null)" in
  arm64 | aarch64) arch=arm64 ;;
  x86_64 | amd64) arch=x64 ;;
  *) die "unsupported machine $(uname -m)" ;;
esac
platform="$os-$arch"

command -v curl >/dev/null 2>&1 || die "curl is required"
command -v python3 >/dev/null 2>&1 || die "python3 is required"

tmp=$(mktemp -d "${TMPDIR:-/tmp}/afterburn-node.XXXXXX") || die "cannot create a temporary directory"
trap 'rm -rf "$tmp"' EXIT INT TERM HUP

if ! curl -fsSL "$BASE/$platform/build-info.json" -o "$tmp/build-info.json"; then
  printf 'afterburn node: the public build for %s is not up yet.\n' "$platform" >&2
  printf 'The app ships with its node: https://afterburn.nuts.cash/download/\n' >&2
  exit 1
fi

curl -fsSL "$BASE/$platform/afterburn-node" -o "$tmp/afterburn-node" ||
  die "cannot download the $platform node"
curl -fsSL "$BASE/install-node.sh" -o "$tmp/install-node.sh" ||
  die "cannot download install-node.sh"
curl -fsSL "$BASE/install-node.py" -o "$tmp/install-node.py" ||
  die "cannot download install-node.py"

read_info() {
  python3 -c "import json,sys; print(json.load(open(sys.argv[1]))['$1'])" "$tmp/build-info.json"
}
expected=$(read_info sha256) || die "cannot read the published checksum"
build_id=$(read_info build_id) || die "cannot read the published build id"

if command -v sha256sum >/dev/null 2>&1; then
  actual=$(sha256sum "$tmp/afterburn-node" | cut -d' ' -f1)
elif command -v shasum >/dev/null 2>&1; then
  actual=$(shasum -a 256 "$tmp/afterburn-node" | cut -d' ' -f1)
else
  die "need sha256sum or shasum to verify the download"
fi
[ "$actual" = "$expected" ] || die "checksum mismatch for $platform; nothing was installed"
chmod 755 "$tmp/afterburn-node"

scope=${AFTERBURN_NODE_SCOPE:-}
if [ -z "$scope" ]; then
  if [ "$os" = linux ] && [ "$(id -u 2>/dev/null || echo 1)" = 0 ]; then
    scope=system
  else
    scope=user
  fi
fi

set -- --binary "$tmp/afterburn-node" --build-id "$build_id" --scope "$scope"
if [ "${AFTERBURN_NODE_UPGRADE:-0}" = 1 ]; then
  set -- "$@" --upgrade
fi
exec sh "$tmp/install-node.sh" "$@"
