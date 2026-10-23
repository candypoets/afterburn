#!/usr/bin/env python3
"""Apply a checked, idempotent client source overlay to a copy of server/figcraft."""
import hashlib
import json
from pathlib import Path
import shutil
import sys

HERE = Path(__file__).resolve().parent


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest() if path.is_file() else None


def safe_path(root, relative):
    path = root / relative
    if path.resolve() != root.resolve() and root.resolve() not in path.resolve().parents:
        raise SystemExit("Overlay path escapes its checkout")
    if path.is_symlink():
        raise SystemExit(f"Refusing to overwrite symlink: {relative}")
    return path


def main():
    if len(sys.argv) != 2:
        raise SystemExit("Usage: apply-source-overlay.py CHECKOUT (a copy of server/figcraft)")
    checkout = Path(sys.argv[1]).resolve()
    if not (checkout / "Cargo.toml").is_file():
        raise SystemExit("Expected an existing FigCraft checkout")
    manifest = json.loads((HERE / "source-overlay.json").read_text())
    if manifest.get("version") != 1:
        raise SystemExit("Unsupported source overlay format")
    overlay = HERE / "source-overlay"
    # Validate every input before changing any file; never replace unrelated local edits.
    for entry in manifest["files"]:
        source = safe_path(overlay, entry["path"])
        target = safe_path(checkout, entry["path"])
        if digest(source) != entry["sha256"]:
            raise SystemExit(f"Overlay checksum mismatch: {entry['path']}")
        if digest(target) not in (entry["base_sha256"], entry["sha256"]):
            raise SystemExit(f"Checkout has unrelated changes: {entry['path']}")
    for entry in manifest["deleted"]:
        target = safe_path(checkout, entry["path"])
        if digest(target) not in (None, entry["base_sha256"]):
            raise SystemExit(f"Checkout has unrelated changes: {entry['path']}")
    for entry in manifest["files"]:
        target = safe_path(checkout, entry["path"])
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(overlay / entry["path"], target)
    for entry in manifest["deleted"]:
        safe_path(checkout, entry["path"]).unlink(missing_ok=True)
    print(f"Applied {len(manifest['files'])} source files.")


if __name__ == "__main__":
    main()
