#!/usr/bin/env python3
"""Draw the mindmap system (client, server, private Herdr) and publish it to the 0.7.0 fork."""
from __future__ import annotations

import json
import subprocess
import sys
import time
from datetime import date
from pathlib import Path

from fc import Doc, F, H, Vt, R, T
import map_kit as K
import build_map

PROJECT = Path(__file__).resolve().parent.parent
SERVER = PROJECT / "server"
OUT = PROJECT / "build"
CLI = SERVER / "figcraft/target/release/figcraft-cli"
KEY = (SERVER / "data/server.key").read_text().strip()
ADDRESS = "ws://100.104.9.15:47910"


def _t(text, size, font=K.SANS, style="Regular", color=K.FG, w=None, name=None, lh=None, ls=None,
       x=0, y=0):
    return T(text, size, font=font, style=style, color=color, w=w, name=name, lh=lh, ls=ls,
             x=x, y=y)


def label(text):
    return _t(text, 10, K.MONO, "Medium", K.MUTED, name=text, ls=6)


def card(title, eyebrow, body, facts, w, ink=K.FG, fill=K.SURF):
    return Vt(
        label(eyebrow),
        _t(title, 30, K.DISP, "Regular", ink, w=w - 48, name=title, lh=34),
        _t(body, 14, K.SANS, "Regular", K.MUTED, w=w - 48, name="body", lh=20),
        _t(facts, 11, K.MONO, "Regular", K.FG, w=w - 48, name="facts", lh=18),
        gap=8, pad=24, w=w, fill=fill, stroke=K.BORDER, radius=12, name=title)


def lane(title, body, w, ink):
    return Vt(
        _t(title, 10, K.MONO, "Medium", ink, name="lane", ls=6),
        _t(body, 13, K.SANS, "Regular", K.FG, w=w - 32, name="body", lh=19),
        gap=6, pad=16, w=w, fill="#F7F4EE", stroke=K.BORDER, radius=8, name=title)


def build():
    doc = Doc("Mindmap · system")
    page = doc.page("System", bg=K.BG)

    doc.place(page, K.heading(
        "The system, end to end",
        "One service on the host, one client on the Mac. The canvas is the document plane; the "
        "agents are the execution plane; Herdr is the part nobody sees.", w=1700), 64, 56)

    mac = card(
        "MacBook · FigCraft",
        "CLIENT · 0.7.0",
        "Opens the project's document over Tailscale. Read, select, comment, ask — no drawing "
        "tools for the human.",
        "~/code/figcraft/target/debug/figcraft\ncontrol channel 127.0.0.1:49123",
        980, ink="#2F6F6A")

    transport = Vt(
        _t("↓", 26, K.SANS, "Regular", K.ACCENT, name="arrow"),
        _t("TAILSCALE", 10, K.MONO, "Medium", K.ACCENT, name="name", ls=6),
        _t("100.107.22.29  →  100.104.9.15", 12, K.MONO, "Regular", K.MUTED, name="path"),
        gap=4, w=980, cross="center", name="Transport")

    doc_lane = lane(
        "DOCUMENT PLANE",
        "Files, edit and view links, live sessions, persistence as snapshot plus log. "
        f"{ADDRESS} · data in server/data.",
        440, "#3E5A78")
    run_lane = lane(
        "EXECUTION PLANE",
        "A private Herdr session with its own socket under data/agents: a workspace per project, "
        "a pane per job, worktrees for parallel designs. Invisible to the user.",
        440, K.ACCENT)

    host = Vt(
        label("HOST · BEFREE"),
        _t("One service", 30, K.DISP, "Regular", K.FG, w=940 - 48, name="title", lh=34),
        _t("mindmap-server 0.7.0 — the document plane and the agent host in the same process.",
           14, K.SANS, "Regular", K.MUTED, w=940 - 48, name="body", lh=20),
        H(doc_lane, run_lane, gap=16, name="Lanes"),
        _t("unit: server/systemd/mindmap-server.service · today the document plane is "
           "containerised and the host unit adds the agents", 11, K.MONO, "Regular", K.FG,
           w=940 - 48, name="facts", lh=18),
        gap=10, pad=24, w=940, fill=K.SURF, stroke=K.BORDER, radius=14, name="Host")

    doc.place(page, Vt(mac, transport, host, gap=14, w=980, name="Stack"), 64, 240)

    note = Vt(
        label("WHAT A USER INSTALLS"),
        _t("One service. Not FigCraft plus Herdr.", 20, K.DISP, "Regular", K.FG, w=560, name="one", lh=24),
        label("WHAT THEY NEVER SEE"),
        _t("Herdr, its socket, the links, the tokens, the queue.", 16, K.SANS, "Regular", K.MUTED,
           w=560, name="two", lh=22),
        label("WHERE THIS IS TODAY"),
        _t("0.7.0 fork live on 47910; Mac publishes and joins; private Herdr proven beside the "
           "user's own daemon; the host unit is written.", 13, K.SANS, "Regular", K.FG, w=560,
           name="today", lh=19),
        gap=8, pad=26, w=620, fill="#F7F4EE", stroke=K.BORDER, radius=12, name="Notes")
    doc.place(page, note, 1120, 240)

    footer = _t(f"Designed in FigCraft · {date.today().isoformat()} · a map of the system, not of the code",
                10, K.MONO, "Regular", K.MUTED, w=900, name="Footer")
    doc.place(page, footer, 64, 780)
    build_screens(doc)
    return doc, page


def main():
    doc, page = build()
    OUT.mkdir(exist_ok=True)
    raw = OUT / "mindmap-system.raw.figcraft"
    doc.save(str(raw))

    build_map.CLI = CLI
    s = build_map.Session(bridge=False)
    try:
        s.tool("open_file", path=str(raw))
        relayout = json.loads(Path(str(raw) + ".relayout.json").read_text())
        wrapped = [n["id"] for n in doc.nodes.values()
                   if n["type"] == "text" and n.get("auto_resize") == "height"]
        plan = ([{"command": "node.set", "params": {"ids": wrapped, "textAutoResize": "height"}}]
                + [{"command": c["command"],
                    "params": dict(c["params"], gap=c["params"]["gap"] + 0.25)} for c in relayout]
                + relayout)
        for i in range(0, len(plan), 400):
            s.tool("batch", commands=plan[i:i + 400], label="relayout")
        # place below the measured heading
        h = s.tool("get_node", id=page, depth=0)  # noqa: F841 (page exists)
        out = OUT / "mindmap-system.figcraft"
        s.tool("save_file", path=str(out))
        print("wrote", out)

        if "--no-publish" in sys.argv:
            return
        s.cmd("collab.publish", server=f"{ADDRESS}/?key={KEY}", name="Mindmap · system")
        for _ in range(30):
            time.sleep(0.5)
            peers = s.cmd("collab.peers")
            if peers.get("status") == "live":
                break
        print("live:", peers.get("status"), peers.get("fileName"))
    finally:
        s.close()




# --- screens page ---------------------------------------------------------------

def _mock(w=820, h=600, name="Screen"):
    return F(name, w=w, h=h, fill="#EDEBE6", stroke=K.BORDER, radius=14, clip=True)


def _topbar(w=820):
    return F("Top bar",
             _t("Mindmap", 15, K.SANS, "Medium", K.FG, name="wordmark", x=20, y=17),
             _t("FigCraft  ›  Components", 11, K.MONO, "Regular", K.MUTED, name="crumb", x=132, y=20),
             F("Share", _t("Share", 11, K.SANS, "Medium", "#FFFFFF", name="label", x=14, y=7),
               x=w - 96, y=13, w=72, h=30, fill=K.ACCENT, radius=6),
             x=0, y=0, w=w, h=56, fill=K.SURF, clip=True)


def _card(x, y, title, lines, w=360, h=180):
    return F("Card " + title,
             _t(title, 15, K.SANS, "Medium", K.FG, name="title", x=14, y=12),
             _t(lines, 11, K.SANS, "Regular", K.MUTED, w=w - 28, name="body", lh=16, x=14, y=38),
             x=x, y=y, w=w, h=h, fill=K.SURF, stroke=K.BORDER, radius=8, clip=True)


def _composer(x, y, w, question, hint="⏎"):
    return F("Composer",
             _t(question, 13, K.SANS, "Regular", "#F2EFE8", w=w - 70, name="question", x=16, y=14),
             _t(hint, 11, K.MONO, "Regular", K.DISABLED, name="hint", x=w - 34, y=15),
             x=x, y=y, w=w, h=46, fill=K.FG, radius=9, clip=True)


def build_screens(doc):
    page = doc.page("Screens", bg=K.BG)
    doc.place(page, K.heading(
        "Client screens — a proposal",
        "The map client has no tools: a canvas, a top bar, a question bar, and the agent's work "
        "shown where it happened. Two screens, drawn at the size of the Mac window.", w=1800),
        64, 56)

    project = _mock(name="Project")
    project.add(
        _topbar(),
        _card(24, 80, "Canvas and layers", "Frames, groups, sections, shapes and text are one node tree. The canvas is how you edit it — or, here, how you read it."),
        _card(420, 80, "Components", "A component is a subtree with identity; an instance remembers it and what changed."),
        _card(24, 280, "Auto layout", "Frames size themselves and children follow rules instead of coordinates."),
        _composer(24, 526, 772, "Ask or describe a change…", "⌘K"))
    doc.place(page, project, 64, 250)

    ask = _mock(name="Ask")
    ask.add(
        _topbar(),
        _card(24, 80, "Canvas and layers", "Frames, groups, sections, shapes and text are one node tree. The canvas is how you edit it — or, here, how you read it.", h=180),
        F("Selection", x=18, y=74, w=372, h=192, fill=None, stroke=K.ACCENT, sw=2, radius=10),
        F("Context",
          _t("SELECTED · Components and instances", 9, K.MONO, "Medium", K.ACCENT, name="label", ls=4, x=12, y=6),
          _t("components.rs:1", 10, K.MONO, "Regular", K.MUTED, name="src", x=250, y=6),
          x=24, y=470, w=520, h=26, fill="#F7F4EE", stroke=K.BORDER, radius=6, clip=True),
        F("Reply",
          _t("AGENT", 9, K.MONO, "Medium", K.ACCENT, name="who", ls=4, x=14, y=10),
          _t("Every instance is rebuilt from its main, keeping layer ids and re-applying overrides. That is why a component edit reaches every screen.", 11, K.SANS, "Regular", K.FG, w=330, name="text", lh=16, x=14, y=28),
          x=420, y=280, w=372, h=150, fill=K.SURF, stroke=K.BORDER, radius=8, clip=True),
        _composer(24, 526, 772, "Why does editing a main change every screen?", "⏎"))
    doc.place(page, ask, 960, 250)

    doc.place(page, _t("PROJECT · no panels, no tools", 10, K.MONO, "Medium", K.MUTED, name="cap1", ls=4), 64, 870)
    doc.place(page, _t("ASK · the selection carries its sources", 10, K.MONO, "Medium", K.MUTED, name="cap2", ls=4), 960, 870)
    return page


if __name__ == "__main__":
    main()
