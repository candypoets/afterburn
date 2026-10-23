#!/usr/bin/env python3
"""Capture the complete ignored client checkout delta against server/figcraft."""
import hashlib
import json
import os
from pathlib import Path
import shutil

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
SOURCE = ROOT / "client-src"
BASE = ROOT / "server/figcraft"
OVERLAY = HERE / "source-overlay"
SKIP = {".git", "target", "node_modules", "dist", ".scratch", ".habitat", ".cache"}


def files(root):
    result = {}
    for directory, dirs, names in os.walk(root):
        dirs[:] = [name for name in dirs if name not in SKIP]
        for name in names:
            path = Path(directory) / name
            if path.is_symlink():
                raise SystemExit(f"Refusing symlink in source inventory: {path.relative_to(root)}")
            result[path.relative_to(root).as_posix()] = path
    return result


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    source, base = files(SOURCE), files(BASE)
    changed, deleted = [], []
    for relative, path in sorted(source.items()):
        before = digest(base[relative]) if relative in base else None
        after = digest(path)
        if before == after:
            continue
        target = OVERLAY / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(path, target)
        changed.append({"path": relative, "base_sha256": before, "sha256": after})
    for relative, path in sorted(base.items()):
        if relative not in source:
            deleted.append({"path": relative, "base_sha256": digest(path)})
    keep = {entry["path"] for entry in changed}
    if OVERLAY.exists():
        for relative, path in files(OVERLAY).items():
            if relative not in keep:
                path.unlink()
    manifest = {"version": 1, "baseline": "server/figcraft", "files": changed, "deleted": deleted}
    (HERE / "source-overlay.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"Captured {len(changed)} changed/new files and {len(deleted)} deletions.")


if __name__ == "__main__":
    main()
