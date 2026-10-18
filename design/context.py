#!/usr/bin/env python3
"""The context bundle for a selected card: what an agent needs to answer well.

    python3 design/context.py --node 1234
    python3 design/context.py --selected                 # the session's selection
    python3 design/context.py --link <invite> --selected # a live hosted copy
    python3 design/context.py --node 1234 --shot out.png --scale 2

It reads the built (or hosted) document, walks up from the selected layer to the
component instances that carry the meaning, and prints one JSON object: the
page, the trail of cards from the area to the leaf, their text properties and
sources, a crop of the map around the selection, and the map's version.

This is the movable part of "click a layer and ask": the agent gets the visual
context, the words and the code pointer without the human retyping any of it.
"""
from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path

from build_map import Session, OUT

META = OUT / "figcraft-feature-map.json"
CONTEXT = OUT / "context"

TITLE_PROPS = ("Name", "Title", "Claim", "Name")
DETAIL_PROPS = ("Role", "Fact", "Detail", "Body", "Claim")


def tree(s, page_id):
    return s.tool("get_node", id=page_id, depth=30)


def find(node, node_id, ancestors=()):
    if node.get("id") == node_id:
        return node, ancestors
    for child in node.get("children", []):
        found = find(child, node_id, ancestors + (node,))
        if found:
            return found
    return None


def pages(s):
    return s.tool("inspect_document", depth=0)["pages"]


def locate(s, node_id):
    for page in pages(s):
        root = tree(s, page["id"])
        hit = find(root, node_id)
        if hit:
            return page, *hit
    return None


def first_prop(node, names):
    props = node.get("componentProperties") or {}
    for name in names:
        if props.get(name):
            return props[name]
    return None


def card_of(node):
    return {
        "component": node.get("name"),
        "title": first_prop(node, ("Name", "Title", "Claim")) or node.get("name"),
        "detail": first_prop(node, ("Role", "Fact", "Detail", "Body")),
        "source": first_prop(node, ("Source",)),
        "nodeId": node.get("id"),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--file", default=str(OUT / "figcraft-feature-map.figcraft"))
    parser.add_argument("--link", help="join a hosted copy instead of opening a file")
    parser.add_argument("--node", type=int, help="layer id; default: the session selection")
    parser.add_argument("--selected", action="store_true", help="use the session selection")
    parser.add_argument("--shot", help="where to write the crop (default build/context/<id>.png)")
    parser.add_argument("--scale", type=float, default=1.0)
    args = parser.parse_args()

    s = Session(bridge=False)
    try:
        if args.link:
            s.cmd("collab.join", link=args.link)
            peers = {}
            for _ in range(30):
                time.sleep(0.5)
                peers = s.cmd("collab.peers")
                if peers.get("status") == "live":
                    break
            if peers.get("status") != "live":
                print(f"Could not join: {peers.get('status')} {peers.get('reason') or ''}", file=sys.stderr)
                return 2
        else:
            s.tool("open_file", path=args.file)

        document = s.tool("inspect_document", depth=0)
        node_id = args.node
        if node_id is None:
            selection = document.get("selection") or []
            if not selection:
                print("Nothing is selected. Pass --node, or select a layer in the app.", file=sys.stderr)
                return 2
            node_id = selection[0]

        located = locate(s, node_id)
        if not located:
            print(f"No layer {node_id} in this document.", file=sys.stderr)
            return 2

        page, node, ancestors = located
        instances = [n for n in (*ancestors, node) if n.get("type") == "INSTANCE"]
        trail = [card_of(n) for n in instances]

        nearest = instances[-1] if instances else node
        shot = Path(args.shot) if args.shot else CONTEXT / f"{nearest.get('id')}.png"
        shot.parent.mkdir(parents=True, exist_ok=True)
        try:
            s.tool("export", ids=[nearest.get("id")], path=str(shot), format="png", scale=args.scale)
        except Exception:
            shot = None

        meta = json.loads(META.read_text()) if META.is_file() else {}
        bundle = {
            "map": {k: meta.get(k) for k in ("name", "version", "revision", "date", "anchors", "areas", "capabilities")},
            "page": page["name"],
            "selection": {"id": node_id, "name": node.get("name"), "type": node.get("type")},
            "trail": trail,
            "path": [page["name"]] + [item["title"] for item in trail],
            "screenshot": str(shot) if shot else None,
        }
        print(json.dumps(bundle, indent=1, ensure_ascii=False))
    finally:
        s.close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
