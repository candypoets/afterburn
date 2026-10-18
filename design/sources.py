"""Resolve (path, literal) anchors against a FigCraft checkout.

An anchor is one line of real code: the path, and a literal that must appear in
that file exactly once. The build resolves it to `path:line`, so a card can
never cite an identifier or a line the code does not have. When a cited line
disappears (renamed, refactored, deleted) the build stops and names the card.
"""
from __future__ import annotations

import os
import sys
from dataclasses import dataclass
from pathlib import Path


class AnchorError(RuntimeError):
    pass


@dataclass(frozen=True)
class Source:
    path: str
    literal: str
    line: int

    def where(self) -> str:
        return f"{self.path}:{self.line}"


def repo_root() -> Path:
    """The FigCraft checkout: --src, $FIGCRAFT_SRC, ./.figcraft-src, ../figcraft-src.

    Relative paths are tried from the current directory first, then from the
    project root (the parent of design/), so the scripts run from anywhere.
    """
    here = Path(__file__).resolve().parent.parent
    for candidate in (sys.argv[sys.argv.index("--src") + 1] if "--src" in sys.argv else None,
                      os.environ.get("FIGCRAFT_SRC"),
                      ".figcraft-src",
                      str(here / ".figcraft-src"),
                      "../figcraft-src",
                      str(here.parent / "figcraft-src")):
        if candidate and (Path(candidate) / "Cargo.toml").is_file():
            return Path(candidate).resolve()
    raise SystemExit("No FigCraft checkout: pass --src DIR, set FIGCRAFT_SRC, or clone to .figcraft-src")


ROOT = repo_root()


def resolve(path: str, literal: str, *, owner: str = "") -> Source:
    file = ROOT / path
    if not file.is_file():
        raise AnchorError(f"{owner}: {path} does not exist")
    lines = file.read_text(encoding="utf-8").splitlines()
    hits = [i + 1 for i, line in enumerate(lines) if literal in line]
    if not hits:
        raise AnchorError(f"{owner}: no line of {path} contains {literal!r}")
    if len(hits) > 1:
        raise AnchorError(f"{owner}: {literal!r} appears {len(hits)} times in {path} "
                          f"(lines {', '.join(map(str, hits))}); cite something unique")
    return Source(path, literal, hits[0])


# The source of the map itself: recorded on the root card and the footer, not a
# card anchor, because it is the checkout rather than a claim about the code.
def revision() -> tuple[str, str]:
    import subprocess

    def git(*args: str) -> str:
        return subprocess.check_output(["git", "-C", str(ROOT), *args], text=True).strip()

    return git("rev-parse", "--short", "HEAD"), git("log", "-1", "--format=%cs")


def count_specs() -> int:
    total = 0
    for file in (ROOT / "crates/engine/src/cmd").rglob("*.rs"):
        total += file.read_text(encoding="utf-8").count("spec!(")
    return total


def rust_lines() -> int:
    total = 0
    for base in ("crates", "apps"):
        for file in (ROOT / base).rglob("*.rs"):
            try:
                total += len(file.read_text(encoding="utf-8").splitlines())
            except OSError:
                pass
    return total


def crate_count() -> int:
    return len([p for p in (ROOT / "crates").iterdir() if p.is_dir()])


def app_count() -> int:
    return len([p for p in (ROOT / "apps").iterdir() if p.is_dir()])


def version() -> str:
    for line in (ROOT / "Cargo.toml").read_text(encoding="utf-8").splitlines():
        if line.startswith("version"):
            return line.split('"')[1]
    return "?"
