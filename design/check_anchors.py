"""Check every anchor in map_content.py without building the document."""
from __future__ import annotations

import sys

import map_content as C
import sources


def every_anchor():
    yield "root", C.ROOT["source"]
    for branch in C.BRANCHES:
        yield f"area {branch['area']}", branch["source"]
        for leaf in branch["leaves"]:
            yield f"leaf {leaf['title']!r}", leaf["source"]
    for stage in C.ANATOMY["stages"]:
        yield f"stage {stage['name']!r}", stage["source"]
    yield "surfaces", C.ANATOMY["surfaces"]["source"]
    for card in C.DETAIL["cards"]:
        yield f"detail {card['title']!r}", card["source"]


def main() -> int:
    total = 0
    for owner, (path, literal) in every_anchor():
        try:
            src = sources.resolve(path, literal, owner=owner)
        except sources.AnchorError as error:
            print(f"FAIL {error}")
            return 1
        total += 1
        print(f"{src.where():<52} {owner}")
    print(f"\n{total} anchors OK · {len(C.BRANCHES)} areas · "
          f"{sum(len(b['leaves']) for b in C.BRANCHES)} capabilities · "
          f"{sources.version()} · {sources.count_specs()} spec! entries · "
          f"{sources.rust_lines()} Rust lines")
    return 0


if __name__ == "__main__":
    sys.exit(main())
