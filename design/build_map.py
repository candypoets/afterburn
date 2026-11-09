#!/usr/bin/env python3
"""Build "FigCraft · feature map" from the FigCraft source.

    python3 build_map.py --check        # resolve every anchor, build nothing
    python3 build_map.py                # build/figcraft-feature-map.figcraft + shots/
    python3 build_map.py --publish      # ... and publish it to the FigCraft host

Feature first. Branch cards are areas of the tool and hold their Leaf cards in a
slot, so the containment on the canvas is the containment in the model. Every
Leaf's Source is a (path, literal) anchor resolved against the checkout before
anything is drawn; a missing line stops the build.

Two extra screens: a detail screen for one area (the "zoom in" a human asks
for), reached by a prototype link from the map; and a map-mode sketch, which is
a proposal drawn in FigCraft rather than a claim about the code.

Components are turned on by an MCP session driving the same headless engine the
app uses (figcraft-cli mcp). Publishing goes through the host's bridge, which
adds publish_project and keeps the server key out of this script.
"""
from __future__ import annotations

import json
import os
import subprocess
import sys
import time
from pathlib import Path

from fc import Doc, H, Vt, T
import map_content as C
import map_kit as K
import sources

PROJECT = Path(__file__).resolve().parent.parent
OUT = PROJECT / "build"
SHOTS = OUT / "shots"
CLI = Path(os.environ.get("FIGCRAFT_CLI", "/root/figcraft-host/vendor/figcraft-cli"))
BRIDGE = Path(os.environ.get("FIGCRAFT_BRIDGE", "/root/figcraft-host/app/mcp_bridge.py"))
SERVER_DATA = Path(os.environ.get("FIGCRAFT_SERVER_DATA", "/root/figcraft-host/data/files"))

# Where the detail screen sits, to the right of the map.
DETAIL_X = 2912


# ---------------------------------------------------------------- anchors

def resolved():
    """Resolve every anchor once; return the content with path:line strings."""
    def src(owner, pair):
        return sources.resolve(pair[0], pair[1], owner=owner).where()

    root = dict(C.ROOT, where=src("root", C.ROOT["source"]))
    branches = []
    for branch in C.BRANCHES:
        leaves = [dict(leaf, where=src(f"leaf {leaf['title']!r}", leaf["source"]))
                  for leaf in branch["leaves"]]
        branches.append(dict(branch, where=src(f"area {branch['area']}", branch["source"]),
                             leaves=leaves))
    stages = [dict(stage, where=src(f"stage {stage['name']!r}", stage["source"]))
              for stage in C.ANATOMY["stages"]]
    surfaces = dict(C.ANATOMY["surfaces"],
                    where=src("surfaces", C.ANATOMY["surfaces"]["source"]))
    anatomy = {"title": C.ANATOMY["title"], "lede": C.ANATOMY["lede"],
               "stages": stages, "surfaces": surfaces}
    detail = dict(C.DETAIL,
                  cards=[dict(card, where=src(f"detail {card['title']!r}", card["source"]))
                         for card in C.DETAIL["cards"]])
    return root, branches, anatomy, detail


def anchor_count(root, branches, anatomy, detail):
    return (1 + len(branches) + sum(len(b["leaves"]) for b in branches)
            + len(anatomy["stages"]) + 1 + len(detail["cards"]))


# ---------------------------------------------------------------- MCP

class Session:
    """MCP over stdio. Through the host bridge when publishing, so it can inject the key."""

    def __init__(self, bridge: bool):
        command = [sys.executable, str(BRIDGE)] if bridge else [str(CLI), "mcp"]
        self.p = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        self.n = 0
        self._rpc("initialize", {"protocolVersion": "2025-03-26", "capabilities": {},
                                 "clientInfo": {"name": "build_map", "version": "1"}})
        self._send({"jsonrpc": "2.0", "method": "notifications/initialized"})

    def _send(self, message):
        self.p.stdin.write(json.dumps(message) + "\n")
        self.p.stdin.flush()

    def _rpc(self, method, params):
        self.n += 1
        self._send({"jsonrpc": "2.0", "id": self.n, "method": method, "params": params})
        while True:
            line = self.p.stdout.readline()
            if not line:
                raise RuntimeError("MCP session closed")
            message = json.loads(line)
            if message.get("id") == self.n:
                if "error" in message:
                    raise RuntimeError(message["error"])
                return message["result"]

    def tool(self, tool_name, **args):
        r = self._rpc("tools/call", {"name": tool_name, "arguments": args})
        text = r["content"][0].get("text", "")
        if r.get("isError"):
            raise RuntimeError(f"{tool_name} {args}: {text[:600]}")
        try:
            return json.loads(text)
        except ValueError:
            return text

    def cmd(self, command, **params):
        return self.tool("run_command", command=command, params=params)

    def close(self):
        self.p.terminate()


def default_text(main, layer):
    node = K.find(main, layer)
    if node is None:
        raise SystemExit(f"no layer {layer!r} in {main.name}")
    return node.kw["text"]


# ---------------------------------------------------------------- document

def build_document(root, branches, anatomy, detail):
    revision, date = sources.revision()
    lines = sources.rust_lines()
    specs = sources.count_specs()
    anchors = anchor_count(root, branches, anatomy, detail)
    leaves = sum(len(b["leaves"]) for b in branches)

    doc = Doc("FigCraft · feature map")
    comp_page = doc.page("Components", bg="#E3E0D9")
    map_page = doc.page("Features", bg=K.BG)
    anatomy_page = doc.page("Anatomy", bg=K.BG)
    mode_page = doc.page("Map mode", bg=K.BG)

    def label(text, x, y):
        doc.place(comp_page, T(text, 10, font=K.MONO, color=K.MUTED, name=text, ls=8), x, y)

    # ---- Components page: 12 area variants of Branch and Leaf, and the singles
    doc.place(comp_page, K.heading(
        "Components",
        "Every card in this file is an instance. Edit a main here and every page follows — "
        "that is what makes the map cheap to regenerate and review.", w=3000), 0, 0)

    areas = [b["area"] for b in branches]
    branch_mains = {area: K.branch_main(area) for area in areas}
    leaf_mains = {area: K.leaf_main(area) for area in areas}
    stage_main = K.stage_main()
    surfaces_main = K.surfaces_main()
    root_main = K.root_main()
    arrow_main = K.arrow()

    label("BRANCH · variant by Area · the Code slot holds its leaves", 0, 170)
    for i, area in enumerate(areas):
        doc.place(comp_page, branch_mains[area], (i % 4) * (K.BRANCH_W + 48),
                  210 + (i // 4) * 180)

    label("LEAF · variant by Area · one capability, one source line", 0, 780)
    for i, area in enumerate(areas):
        doc.place(comp_page, leaf_mains[area], (i % 4) * (K.LEAF_W + 48),
                  820 + (i // 4) * 140)

    label("STAGE · one step of a command's path", 0, 1250)
    doc.place(comp_page, stage_main, 0, 1290)
    label("SURFACES · the one-id card", 380, 1250)
    doc.place(comp_page, surfaces_main, 380, 1290)

    label("ROOT CARD", 0, 1600)
    doc.place(comp_page, root_main, 0, 1640)
    label("ARROW", 480, 1600)
    doc.place(comp_page, arrow_main, 480, 1640)

    # ---- Features page
    heading_map = K.heading(
        "FigCraft — feature by feature",
        "A Figma-class tool in pure Rust: what it can do, area by area, and the line of code "
        "that makes each thing work. Read a branch top to bottom; the ink is the area. The "
        "Components area opens a detail screen — the first zoom.", w=2700)
    doc.place(map_page, heading_map, 64, 56)

    root_slot = Vt(name="Root slot", gap=0, w=K.ROOT_W)
    columns = [Vt(name=f"Branches {n}", gap=18, w=K.BRANCH_W) for n in "ABC"]
    footer = T(f"Generated by design/build_map.py from storytold/figcraft {sources.version()} · "
               f"{revision} · {date} · {anchors} anchors verified · features first, code to prove it",
               10, font=K.MONO, color=K.MUTED, w=1500, name="Footer")
    content = H(
        Vt(root_slot,
           K.key_card(),
           K.stats_card([
               f"{len(branches)} areas · {leaves} capabilities",
               f"{anchors} anchors verified",
               f"{specs} command specs",
               f"{sources.version()} at {revision}",
               f"~{round(lines / 1000)}k Rust lines",
           ]),
           gap=20, w=K.ROOT_W, name="Root column"),
        *columns, gap=56, name="Afterburn")
    map_block = Vt(content, footer, gap=48, pad=[0, 0, 96, 0], name="Map block")
    doc.place(map_page, map_block, 64, 400)

    detail_screen = K.detail_screen(detail)
    detail_caption = K.caption(detail["caption"])
    doc.place(map_page, detail_screen, DETAIL_X, 400)
    doc.place(map_page, detail_caption, DETAIL_X, 360)

    # ---- Anatomy page
    heading_anatomy = K.heading(anatomy["title"], anatomy["lede"], w=1800)
    doc.place(anatomy_page, heading_anatomy, 64, 56)
    lane = H(name="Lane", gap=14, cross="min")
    surfaces_slot = Vt(name="Surfaces slot", gap=0, w=K.SURFACES_W)
    note = T(anatomy["surfaces"]["note"], 12, font=K.SANS, color=K.MUTED, w=K.SURFACES_W,
             name="Note")
    anatomy_block = Vt(lane, surfaces_slot, note, gap=32, pad=[0, 0, 96, 0], name="Anatomy block")
    doc.place(anatomy_page, anatomy_block, 64, 400)

    # ---- Map mode page: a drawn proposal, not a claim about the code
    heading_mode = K.heading(C.MAP_MODE["title"], C.MAP_MODE["lede"], w=1200)
    doc.place(mode_page, heading_mode, 64, 56)
    mode_frame = K.map_mode_screen(C.MAP_MODE)
    doc.place(mode_page, mode_frame, 64, 400)

    return doc, {
        "pages": {"components": comp_page, "features": map_page, "anatomy": anatomy_page,
                  "mode": mode_page},
        "areas": areas, "branch_mains": branch_mains, "leaf_mains": leaf_mains,
        "stage_main": stage_main, "surfaces_main": surfaces_main, "root_main": root_main,
        "arrow_main": arrow_main, "root_slot": root_slot, "columns": columns,
        "map_block": map_block, "heading_map": heading_map, "footer": footer,
        "detail_screen": detail_screen, "detail_caption": detail_caption,
        "back_link": K.find(detail_screen, "Back to the map"),
        "detail_cards": K.find(detail_screen, "Cards"),
        "lane": lane, "surfaces_slot": surfaces_slot, "anatomy_block": anatomy_block,
        "heading_anatomy": heading_anatomy, "note": note,
        "heading_mode": heading_mode, "mode_frame": mode_frame,
        "anchors": anchors, "revision": revision, "date": date, "lines": lines, "specs": specs,
    }


# ---------------------------------------------------------------- assembly

def assemble(s, doc, nodes, root, branches, anatomy, detail):
    revision, lines, specs = nodes["revision"], nodes["lines"], nodes["specs"]

    # Measure wrapped text, then lay every raw frame out: the writer stores
    # gap + 0.5 so that setting the real gap forces FigCraft to lay out.
    relayout = json.loads(Path(str(doc_path) + ".relayout.json").read_text())
    wrapped = [n["id"] for n in doc.nodes.values()
               if n["type"] == "text" and n.get("auto_resize") == "height"]
    plan = ([{"command": "node.set", "params": {"ids": wrapped, "textAutoResize": "height"}}]
            + [{"command": c["command"],
                "params": dict(c["params"], gap=c["params"]["gap"] + 0.25)} for c in relayout]
            + relayout)
    for i in range(0, len(plan), 400):
        s.tool("batch", commands=plan[i:i + 400], label="relayout")

    def variant_set(mains, name, props, slots=()):
        for main in mains:
            s.cmd("component.create", ids=[main.id])
        set_id = (s.cmd("component.combineVariants", ids=[m.id for m in mains])["id"]
                  if len(mains) > 1 else mains[0].id)
        s.cmd("node.set", ids=[set_id], name=name)
        for prop, layer in props.items():
            s.cmd("component.addProperty", id=K.find(mains[0], layer).id, name=prop,
                  type="text", default=default_text(mains[0], layer))
            for main in mains[1:]:
                s.cmd("component.bindProperty", layer=K.find(main, layer).id,
                      property=prop, field="characters")
        for slot in slots:
            s.cmd("component.makeSlot", id=K.find(mains[0], slot).id, name=slot)
            for main in mains[1:]:
                s.cmd("component.bindProperty", layer=K.find(main, slot).id,
                      property=slot, field="slot")

    def single(main, name, props):
        s.cmd("component.create", ids=[main.id])
        s.cmd("node.set", ids=[main.id], name=name)
        for prop, layer in props.items():
            s.cmd("component.addProperty", id=K.find(main, layer).id, name=prop,
                  type="text", default=default_text(main, layer))

    variant_set([nodes["branch_mains"][a] for a in nodes["areas"]], "Branch",
                {"Name": "title", "Role": "role", "Source": "source"}, slots=("Code",))
    variant_set([nodes["leaf_mains"][a] for a in nodes["areas"]], "Leaf",
                {"Title": "title", "Fact": "fact", "Source": "source"})
    single(nodes["stage_main"], "Stage",
           {"Number": "number", "Name": "name", "Detail": "detail", "Source": "source"})
    single(nodes["surfaces_main"], "Surfaces", {"Body": "body", "Source": "source"})
    single(nodes["root_main"], "Root", {"Claim": "claim", "Facts": "facts", "Source": "source"})
    s.cmd("component.create", ids=[nodes["arrow_main"].id])
    s.cmd("node.set", ids=[nodes["arrow_main"].id], name="Arrow")

    def instance(component, props, parent=None):
        r = s.cmd("instance.create", component=component, parent=parent)
        s.cmd("instance.setProperties", ids=[r["id"]], properties=props)
        return r["id"]

    # Features: root, then the 12 areas in three columns of four
    instance(nodes["root_main"].id, {
        "Claim": root["claim"],
        "Facts": "\n".join([f"{sources.crate_count()} crates · {sources.app_count()} apps",
                            f"{specs} command specs",
                            "MIT OR Apache-2.0",
                            f"{sources.version()} · {revision}"]),
        "Source": root["where"],
    }, parent=nodes["root_slot"].id)

    components_area = None
    for index, branch in enumerate(branches):
        column = nodes["columns"][min(index // 4, 2)]
        bid = instance(nodes["branch_mains"][branch["area"]].id, {
            "Name": branch["title"], "Role": branch["role"], "Source": branch["where"],
        }, parent=column.id)
        if branch["area"] == detail["area"]:
            components_area = bid
        for leaf in branch["leaves"]:
            r = s.cmd("instance.slotInsert", id=bid, slot="Code",
                      component=nodes["leaf_mains"][branch["area"]].id)
            s.cmd("instance.setProperties", ids=[r["id"]], properties={
                "Title": leaf["title"], "Fact": leaf["fact"], "Source": leaf["where"]})

    # The zoom: the area card opens the detail screen; the link on it goes back.
    for card in detail["cards"]:
        instance(nodes["leaf_mains"][detail["area"]].id, {
            "Title": card["title"], "Fact": card["fact"], "Source": card["where"],
        }, parent=nodes["detail_cards"].id)
    if components_area:
        s.cmd("proto.interaction.add", id=components_area, dest=nodes["detail_screen"].id,
              transition={"type": "dissolve"}, durationMs=200, easing="easeOut")
        s.cmd("proto.interaction.add", id=nodes["back_link"].id, action={"type": "back"},
              transition={"type": "dissolve"}, durationMs=200, easing="easeOut")

    # Anatomy: the five stages, arrows between, then the seam card
    for i, stage in enumerate(anatomy["stages"]):
        if i:
            s.cmd("instance.create", component=nodes["arrow_main"].id, parent=nodes["lane"].id)
        instance(nodes["stage_main"].id, {
            "Number": stage["number"], "Name": stage["name"], "Detail": stage["detail"],
            "Source": stage["where"],
        }, parent=nodes["lane"].id)
    instance(nodes["surfaces_main"].id, {
        "Body": anatomy["surfaces"]["body"], "Source": anatomy["surfaces"]["where"],
    }, parent=nodes["surfaces_slot"].id)

    # Place the blocks now that every section has its measured height.
    for page_key, heading_key, block_key in (("features", "heading_map", "map_block"),
                                             ("anatomy", "heading_anatomy", "anatomy_block"),
                                             ("mode", "heading_mode", "mode_frame")):
        s.cmd("page.select", id=nodes["pages"][page_key])
        h = s.tool("get_node", id=nodes[heading_key].id, depth=0)["height"]
        s.cmd("node.set", ids=[nodes[block_key].id], x=64, y=56 + h + 72)
        if page_key == "features":
            s.cmd("node.set", ids=[nodes["detail_screen"].id], x=DETAIL_X, y=56 + h + 72)
            s.cmd("node.set", ids=[nodes["detail_caption"].id], x=DETAIL_X, y=56 + h + 36)

    s.cmd("page.select", id=nodes["pages"]["features"])


# ---------------------------------------------------------------- main

doc_path = OUT / "figcraft-feature-map.raw.figcraft"


def write_sidecar(nodes, branches, detail):
    meta = {
        "name": "FigCraft · feature map",
        "version": sources.version(),
        "revision": nodes["revision"],
        "date": nodes["date"],
        "anchors": nodes["anchors"],
        "areas": len(branches),
        "capabilities": sum(len(b["leaves"]) for b in branches),
        "detail": detail["area"],
        "pages": ["Components", "Features", "Anatomy", "Map mode"],
    }
    path = OUT / "figcraft-feature-map.json"
    path.write_text(json.dumps(meta, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
    return path


def main():
    global doc_path

    if "--check" in sys.argv:
        root, branches, anatomy, detail = resolved()
        total = anchor_count(root, branches, anatomy, detail)
        print(f"{total} anchors OK · {len(branches)} areas · "
              f"{sum(len(b['leaves']) for b in branches)} capabilities · "
              f"{len(detail['cards'])} detail cards · {sources.version()}")
        return 0

    publish = "--publish" in sys.argv
    force = "--force" in sys.argv
    name = sys.argv[sys.argv.index("--name") + 1] if "--name" in sys.argv else "FigCraft · feature map"

    if publish:
        existing = [meta.parent.name for meta in SERVER_DATA.glob("*/meta.json")
                    if json.loads(meta.read_text()).get("name") == name]
        if existing and not force:
            print(f"'{name}' is already on the server ({', '.join(existing)}). Publishing again "
                  f"would create a duplicate; pass --force for a new one.")
            return 1

    root, branches, anatomy, detail = resolved()
    doc, nodes = build_document(root, branches, anatomy, detail)

    OUT.mkdir(exist_ok=True)
    doc.save(str(doc_path))

    s = Session(bridge=publish)
    try:
        s.tool("open_file", path=str(doc_path))
        assemble(s, doc, nodes, root, branches, anatomy, detail)
        out = OUT / "figcraft-feature-map.figcraft"
        s.tool("save_file", path=str(out))
        print("wrote", out)
        print("sidecar", write_sidecar(nodes, branches, detail))

        if publish:
            result = s.tool("publish_project", name=name)
            print("publish:", json.dumps(result)[:400])
            for _ in range(30):
                time.sleep(0.5)
                peers = s.cmd("collab.peers")
                if isinstance(peers, dict) and peers.get("available"):
                    print("live:", json.dumps(peers)[:200])
                    break
            for meta in SERVER_DATA.glob("*/meta.json"):
                data = json.loads(meta.read_text())
                if data.get("name") == name:
                    print("server file:", meta.parent.name, "·", data.get("name"))
    finally:
        s.close()

    SHOTS.mkdir(exist_ok=True)
    for page, slug in (("Features", "features"), ("Anatomy", "anatomy"),
                       ("Map mode", "map-mode"), ("Components", "components")):
        target = SHOTS / f"{slug}.png"
        subprocess.run([str(CLI), "render", str(OUT / "figcraft-feature-map.figcraft"),
                        "-o", str(target), "--scale", "0.6", "--page", page],
                       check=False, capture_output=True)
        if target.exists():
            print("shot", target)
    return 0


if __name__ == "__main__":
    sys.exit(main())
