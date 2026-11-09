#!/usr/bin/env python3
"""Record native provenance or validate/copy companions without executing foreign binaries."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess

PLATFORMS = ("darwin-arm64", "darwin-x64", "linux-arm64", "linux-x64")


def digest(path):
    result = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            result.update(block)
    return result.hexdigest()


def validate(info):
    if not re.fullmatch(r"[0-9a-f]{64}", info.get("build_id", "")) or info.get("protocol") != 1 or not info.get("version"):
        raise ValueError("Invalid build-info identity/protocol")


def record(binary, platform, output):
    info = json.loads(subprocess.check_output([str(binary.resolve()), "build-info"], text=True, timeout=30))
    validate(info)
    info.update(platform=platform, sha256=digest(binary))
    output.write_text(json.dumps(info, indent=2) + "\n")


def copy_assets(assets, expected, destination, native):
    validate(expected)
    pending = []
    for directory in sorted(assets.iterdir()):
        if directory.name in ("install-node.sh", "install-node.py") and directory.is_file():
            continue
        if not directory.is_dir() or directory.name not in PLATFORMS:
            raise ValueError(f"Unexpected node asset: {directory}")
        info = json.loads((directory / "build-info.json").read_text())
        binary = directory / "afterburn-node"
        if any(info.get(key) != expected[key] for key in ("build_id", "version", "protocol")):
            raise ValueError(f"Companion node was built from different canonical source: {directory.name}")
        if info.get("platform") != directory.name or info.get("sha256") != digest(binary):
            raise ValueError(f"Companion node platform/checksum mismatch: {directory.name}")
        if directory.name != native:
            pending.append((directory, binary))
    # Validate every asset before copying any. Native always comes from this full build.
    for directory, binary in pending:
        target = destination / directory.name
        target.mkdir(parents=True, exist_ok=True)
        shutil.copy2(binary, target / "afterburn-node")
        (target / "afterburn-node").chmod(0o755)
        shutil.copy2(directory / "build-info.json", target / "build-info.json")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    native = commands.add_parser("record")
    native.add_argument("--binary", type=Path, required=True)
    native.add_argument("--platform", choices=PLATFORMS, required=True)
    native.add_argument("--output", type=Path, required=True)
    companions = commands.add_parser("copy")
    companions.add_argument("--assets", type=Path, required=True)
    companions.add_argument("--build-info", type=Path, required=True)
    companions.add_argument("--destination", type=Path, required=True)
    companions.add_argument("--native-platform", choices=PLATFORMS, required=True)
    args = parser.parse_args()
    try:
        if args.command == "record":
            record(args.binary, args.platform, args.output)
        else:
            copy_assets(args.assets, json.loads(args.build_info.read_text()), args.destination, args.native_platform)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        parser.exit(1, f"node assets: {error}\n")


if __name__ == "__main__":
    main()
