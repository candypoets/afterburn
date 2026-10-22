#!/usr/bin/env python3
"""Draw the mindmap system (client, server, private Herdr) and publish it to the 0.7.0 fork."""
from __future__ import annotations

import json
import subprocess
import sys
import time
from datetime import date
from pathlib import Path

from fc import Doc, F, H, Vt, R, T, shadow
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
    build_product(doc)
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




# --- product screens ------------------------------------------------------------

def _appbar(w, workspace, tabs=None, active=None, people=("JL", "MK", "TS"), live=3):
    tab_nodes = []
    x = 460
    for name in tabs or []:
        tab_nodes.append(_ui(name, 12, K.SANS, "Medium", UI_ACCENT if name == active else UI_MUTED,
                             name="tab " + name, x=x, y=21))
        if name == active:
            tab_nodes.append(F("underline", x=x, y=52, w=len(name) * 6.4, h=2, fill=UI_ACCENT))
        x += len(name) * 7 + 26
    avatars = []
    x = w - 250
    for initials in people:
        avatars.append(F("Avatar " + initials,
                         _ui(initials, 9, K.MONO, "Medium", "#FFFFFF", name="initials", x=6, y=5),
                         x=x, y=16, w=22, h=22, fill="#6B4E8C", radius=11))
        x += 18
    return F(
        "App bar",
        _ui("Mindmap", 15, K.SANS, "Medium", UI_TEXT, name="wordmark", x=24, y=19),
        _ui(f"Herdr  /  {workspace}", 11, K.MONO, "Regular", UI_MUTED, name="crumb", x=112, y=22),
        *tab_nodes,
        _ui("befree", 10, K.MONO, "Regular", UI_MUTED, name="machine", x=w - 330, y=22),
        *avatars,
        *([_ui(f"{live} live", 10, K.MONO, "Medium", "#2F6F6A", name="live", x=w - 160, y=22)] if live else []),
        F("Share", _ui("Share", 11, K.SANS, "Medium", "#FFFFFF", name="label", x=14, y=7),
          x=w - 96, y=14, w=72, h=28, fill=UI_TEXT, radius=6),
        F("line", x=0, y=55, w=w, h=1, fill=UI_BORDER),
        x=0, y=0, w=w, h=56, fill=UI_PANEL, effects=[shadow(y=2, blur=10, color="#00000010")])


def _ws_row(y, name, path, agents, map_state, selected=False):
    return F(
        "Workspace " + name,
        _ui(name, 13, K.SANS, "Medium", UI_TEXT, name="name", x=16, y=10),
        _ui(path, 9, K.MONO, "Regular", UI_MUTED, w=190, name="path", x=16, y=28),
        _ui(f"{agents} agent{'s' if agents != 1 else ''}" if agents else "no agent", 9, K.MONO,
            "Regular", UI_MUTED, name="agents", x=16, y=44),
        _ui(map_state, 9, K.MONO, "Medium", UI_ACCENT if map_state == "INIT" else "#2F6F6A",
            name="map", x=236, y=10),
        x=0, y=y, w=300, h=64, fill=UI_SOFT if selected else UI_PANEL)


def _screen_workspaces():
    f = F("Workspaces", w=980, h=620, fill=UI_BG, stroke=UI_BORDER, radius=12, clip=True)
    f.add(
        _appbar(980, "Workspaces", live=0, people=()),
        F("Sidebar", x=0, y=56, w=300, h=564, fill=UI_PANEL, stroke=UI_BORDER),
        _ui("WORKSPACES · BEFREE", 9, K.MONO, "Medium", UI_MUTED, name="label", ls=3, x=16, y=72),
        _ws_row(96, "mindmap", "/root/code/mindmap", 2, "MAP", selected=True),
        _ws_row(162, "eisledger", "/root/code/eisledger", 1, "INIT"),
        _ws_row(228, "nuts-cash", "/root/code/nuts-cash", 0, "MAP"),
        _ws_row(294, "worktree-jev", "~/.herdr/worktrees/…/worktree-jev", 3, "MAP"),
        _ui("SELECTED WORKSPACE", 9, K.MONO, "Medium", UI_MUTED, name="eyebrow", ls=3, x=328, y=84),
        _ui("mindmap", 26, K.SANS, "Medium", UI_TEXT, name="title", x=328, y=102),
        _ui("/root/code/mindmap  ·  main  ·  2 agents working", 11, K.MONO, "Regular", UI_MUTED,
            name="meta", x=328, y=140),
        _ui("The map is attached to the workspace, not to a link. Open it, or ask the agents "
           "already in the workspace.", 12.5, K.SANS, "Regular", UI_MUTED, w=600, name="body",
            lh=18, x=328, y=170),
        F("Open map", _ui("Open map", 11, K.SANS, "Medium", "#FFFFFF", name="label", x=18, y=8),
          x=328, y=214, w=110, h=32, fill=UI_TEXT, radius=6),
        F("Agents", _ui("2 agents · working", 11, K.SANS, "Medium", UI_TEXT, name="label", x=16, y=8),
          x=448, y=214, w=150, h=32, fill=UI_PANEL, stroke=UI_BORDER, radius=6),
        F("Hint", _ui("No map yet? The row says INIT — open it and press Init map.",
                      11, K.SANS, "Regular", UI_MUTED, w=540, name="hint", x=16, y=12),
          x=328, y=300, w=600, h=54, fill=UI_SOFT, radius=8))
    return f


def _screen_empty():
    f = F("Empty map", w=980, h=620, fill=UI_BG, stroke=UI_BORDER, radius=12, clip=True)
    f.add(
        _appbar(980, "eisledger", live=1, people=("JL",)),
        F("Empty state",
          _ui("NO MAP YET", 9, K.MONO, "Medium", UI_ACCENT, name="eyebrow", ls=4, x=28, y=24),
          _ui("Init the map for eisledger", 24, K.SANS, "Medium", UI_TEXT, name="title", x=28, y=48),
          _ui("/root/code/eisledger · main · 2 agents in the workspace", 11, K.MONO, "Regular",
              UI_MUTED, name="meta", x=28, y=86),
          _ui("Init asks one agent to read the repository, name the areas and draw the first "
             "overview. Pick who: the agent already here, or a new CLI.", 12.5, K.SANS, "Regular",
              UI_MUTED, w=456, name="body", lh=18, x=28, y=112),
          F("Init with kimi", _ui("Init with kimi", 12, K.SANS, "Medium", "#FFFFFF", name="label", x=16, y=9),
            x=28, y=168, w=136, h=34, fill=UI_ACCENT, radius=6),
          F("Chevron", _ui("▾", 12, K.SANS, "Medium", "#FFFFFF", name="glyph", x=12, y=8),
            x=164, y=168, w=34, h=34, fill="#8F3524", radius=6),
          _ui("kimi is already here · no new process", 10, K.MONO, "Regular", UI_MUTED,
              name="hint", x=28, y=210),
          F("Menu",
            _ui("AGENTS IN THIS WORKSPACE", 9, K.MONO, "Medium", UI_MUTED, name="agents", ls=3, x=14, y=12),
            F("kimi row",
              _ui("kimi", 11.5, K.SANS, "Medium", UI_TEXT, name="name", x=10, y=7),
              _ui("idle", 10, K.MONO, "Regular", UI_MUTED, name="status", x=380, y=8),
              _ui("✓", 11, K.SANS, "Medium", UI_ACCENT, name="check", x=436, y=7),
              x=14, y=32, w=452, h=30, fill=UI_SOFT, radius=5),
            F("claude row",
              _ui("claude", 11.5, K.SANS, "Regular", UI_TEXT, name="name", x=10, y=7),
              _ui("done", 10, K.MONO, "Regular", UI_MUTED, name="status", x=380, y=8),
              x=14, y=64, w=452, h=30, fill=None, radius=5),
            F("divider", x=14, y=104, w=452, h=1, fill=UI_BORDER),
            _ui("START A NEW AGENT", 9, K.MONO, "Medium", UI_MUTED, name="new", ls=3, x=14, y=114),
            F("chips",
              F("codex", _ui("codex", 10.5, K.MONO, "Regular", UI_TEXT, name="t", x=10, y=6),
                x=0, y=0, w=86, h=26, fill=UI_PANEL, stroke=UI_BORDER, radius=5),
              F("claude", _ui("claude", 10.5, K.MONO, "Regular", UI_TEXT, name="t", x=10, y=6),
                x=94, y=0, w=86, h=26, fill=UI_PANEL, stroke=UI_BORDER, radius=5),
              F("kimi", _ui("kimi", 10.5, K.MONO, "Regular", UI_TEXT, name="t", x=10, y=6),
                x=188, y=0, w=86, h=26, fill=UI_PANEL, stroke=UI_BORDER, radius=5),
              x=14, y=136, w=280, h=26),
            _ui("The agent reads the repo and draws; 6–12 areas, every card sourced.", 9.5, K.MONO,
                "Regular", UI_MUTED, w=430, name="note", x=14, y=174),
            x=28, y=240, w=480, h=196, fill=UI_PANEL, stroke=UI_BORDER, radius=8,
            effects=[shadow(y=6, blur=18, color="#00000014")]),
          x=230, y=110, w=536, h=470, fill=UI_PANEL, stroke=UI_BORDER, radius=10,
          effects=[shadow(y=6, blur=18, color="#00000014")]))
    return f



def _screen_init():
    f = F("Init running", w=980, h=620, fill=UI_BG, stroke=UI_BORDER, radius=12, clip=True)
    f.add(
        _appbar(980, "eisledger", tabs=["Overview"], active="Overview"),
        _ui("Agent working · codex · reading crates/engine/src", 11, K.MONO, "Regular", UI_TEXT,
            name="status", x=28, y=84),
        F("Ghost 1", _ui("Reading the repository…", 12, K.SANS, "Regular", UI_MUTED, x=16, y=14),
          F("b1", x=16, y=46, w=190, h=8, fill="#E7E5E1", radius=3),
          F("b2", x=16, y=62, w=150, h=8, fill="#E7E5E1", radius=3),
          F("b3", x=16, y=78, w=100, h=8, fill="#E7E5E1", radius=3),
          x=28, y=116, w=300, h=120, fill=UI_SOFT, stroke=UI_BORDER, radius=8),
        F("Ghost 2", _ui("Naming areas…", 12, K.SANS, "Regular", UI_MUTED, x=16, y=14),
          F("b1", x=16, y=46, w=170, h=8, fill="#E7E5E1", radius=3),
          F("b2", x=16, y=62, w=130, h=8, fill="#E7E5E1", radius=3),
          F("b3", x=16, y=78, w=80, h=8, fill="#E7E5E1", radius=3),
          x=344, y=116, w=300, h=120, fill=UI_SOFT, stroke=UI_BORDER, radius=8),
        F("Progress",
          _ui("INIT · 01 / 03", 9, K.MONO, "Medium", UI_ACCENT, name="step", ls=4, x=16, y=12),
          _ui("Read the repository and plan the overview. Pages appear as they render.",
             11.5, K.SANS, "Regular", UI_TEXT, w=740, name="text", lh=16, x=16, y=30),
          x=28, y=500, w=924, h=64, fill=UI_PANEL, stroke=UI_BORDER, radius=10,
          effects=[shadow(y=4, blur=14, color="#00000010")]),
        F("Stop", _ui("Stop", 11, K.SANS, "Medium", UI_TEXT, name="label", x=16, y=8),
          x=852, y=514, w=78, h=30, fill=UI_PANEL, stroke=UI_BORDER, radius=6))
    return f


def _pin(x, y, n):
    return F("Comment pin " + str(n),
             _ui(str(n), 9, K.MONO, "Medium", "#FFFFFF", name="n", x=5, y=3),
             x=x, y=y, w=18, h=18, fill=UI_ACCENT, radius=9,
             effects=[shadow(y=1, blur=4, color="#00000020")])


def _draft(x, y, text, w):
    return F("Draft",
             _ui(text, 10.5, K.SANS, "Regular", UI_TEXT, w=w - 34, name="text", x=12, y=7),
             _ui("×", 11, K.SANS, "Medium", UI_MUTED, name="x", x=w - 20, y=6),
             x=x, y=y, w=w, h=28, fill=UI_PANEL, stroke=UI_BORDER, radius=6)


def _composer(x, y, w, context, placeholder, drafts, target="kimi", send=False, menu=False):
    kids = [
        _ui("COMMENTING ON  ·  " + context, 8.5, K.MONO, "Medium", UI_ACCENT, name="context", ls=2,
            x=16, y=12),
        _ui(placeholder, 12.5, K.SANS, "Regular", "#9A9791" if not send else UI_TEXT, w=w - 250,
            name="input", x=16, y=34),
        F("Target", _ui(target + "  ▾", 10.5, K.SANS, "Medium", UI_TEXT, name="name", x=12, y=6),
          x=w - 188, y=26, w=86, h=30, fill=UI_PANEL, stroke=UI_BORDER, radius=6),
        F("Send", _ui("↑", 13, K.SANS, "Medium", "#FFFFFF", name="glyph", x=10, y=5),
          x=w - 92, y=26, w=30, h=30, fill=UI_ACCENT, radius=15),
        _ui("⏎ comment   ·   ⌘⏎ send the prompt   ·   " + (f"{drafts} attached" if drafts else "no drafts yet"),
            8.5, K.MONO, "Regular", UI_MUTED, name="hint", x=16, y=64),
    ]
    if menu:
        kids.append(F("Agents",
          _ui("AGENTS IN THIS WORKSPACE", 8.5, K.MONO, "Medium", UI_MUTED, name="agents", ls=2, x=12, y=10),
          F("kimi", _ui("kimi", 10.5, K.SANS, "Medium", UI_TEXT, name="n", x=8, y=5),
            _ui("idle", 9, K.MONO, "Regular", UI_MUTED, name="s", x=118, y=6),
            _ui("✓", 10, K.SANS, "Medium", UI_ACCENT, name="c", x=150, y=5),
            x=10, y=30, w=166, h=24, fill=UI_SOFT, radius=4),
          F("claude", _ui("claude", 10.5, K.SANS, "Regular", UI_TEXT, name="n", x=8, y=5),
            _ui("done", 9, K.MONO, "Regular", UI_MUTED, name="s", x=118, y=6),
            x=10, y=56, w=166, h=24, fill=None, radius=4),
          _ui("START A NEW AGENT", 8.5, K.MONO, "Medium", UI_MUTED, name="new", ls=2, x=12, y=90),
          _ui("codex", 9.5, K.MONO, "Regular", UI_TEXT, name="a", x=12, y=106),
          _ui("claude", 9.5, K.MONO, "Regular", UI_TEXT, name="b", x=62, y=106),
          _ui("kimi", 9.5, K.MONO, "Regular", UI_TEXT, name="c", x=112, y=106),
          x=w - 210, y=-138, w=188, h=124, fill=UI_PANEL, stroke=UI_BORDER, radius=8,
          effects=[shadow(y=6, blur=18, color="#00000018")]))
    return F("Composer", *kids, x=x, y=y, w=w, h=86, fill=UI_PANEL, stroke=UI_BORDER,
             radius=10, effects=[shadow(y=4, blur=16, color="#00000018")])


def _screen_map():
    f = F("Map and composer", w=980, h=620, fill=UI_BG, stroke=UI_BORDER, radius=12, clip=True)
    f.add(
        _appbar(980, "mindmap", tabs=["Overview", "Components", "Handoff"], active="Overview", live=3),
        _canvas_card(24, 84, 1, "Canvas and layers", "One node tree holds the design; the canvas is how you read it.", "Overview", "doc/node.rs:3"),
        _canvas_card(336, 84, 2, "Components", "A subtree with identity; an instance remembers what changed.", "Components", "components.rs:1"),
        _canvas_card(24, 232, 3, "Auto layout", "Frames size themselves; children follow rules.", "Layout", "layout/lib.rs:3"),
        _canvas_card(336, 232, 4, "Text", "Fonts, shaping, bidi and line layout.", "Text", "text/lib.rs:3"),
        F("Selection", x=330, y=78, w=312, h=152, fill=None, stroke=UI_ACCENT, sw=2, radius=10),
        F("Text selection", x=352, y=142, w=156, h=15, fill="#B5452F26", radius=2),
        _ui("⌘C", 8, K.MONO, "Medium", UI_ACCENT, name="copy", x=650, y=142),
        _pin(614, 88, 1),
        _pin(302, 88, 2),
        _pin(614, 236, 3),
        _ui("DRAFTS · 1", 8.5, K.MONO, "Medium", UI_MUTED, name="drafts", ls=2, x=24, y=400),
        _draft(24, 416, "Can the rebuild reuse the instance's layer ids?", 340),
        _composer(24, 486, 932, "Components · components.rs:1", "Comment on this card…", 1),
        _ui("Select another card: the input clears, the draft stays.", 9, K.MONO, "Regular", UI_MUTED,
            name="note", x=24, y=582))
    return f


def _screen_send():
    f = F("Send", w=980, h=620, fill=UI_BG, stroke=UI_BORDER, radius=12, clip=True)
    f.add(
        _appbar(980, "mindmap", tabs=["Overview", "Components", "Handoff"], active="Components", live=2),
        _canvas_card(24, 84, 1, "Canvas and layers", "The selected card is what the prompt is about.", "Overview", "doc/node.rs:3"),
        _canvas_card(336, 84, 2, "Components", "Two drafts and one prompt, in one send.", "Components", "components.rs:1"),
        F("Selection", x=330, y=78, w=312, h=152, fill=None, stroke=UI_ACCENT, sw=2, radius=10),
        _pin(302, 88, 1),
        _pin(614, 88, 2),
        _pin(302, 236, 3),
        _ui("DRAFTS · 2", 8.5, K.MONO, "Medium", UI_MUTED, name="drafts", ls=2, x=24, y=372),
        _draft(24, 388, "Can the rebuild reuse the instance's layer ids?", 340),
        _draft(374, 388, "This should say where overrides reset.", 300),
        _ui("The same input, now the prompt: it sends with both drafts attached.", 9, K.MONO,
            "Regular", UI_MUTED, name="note", x=24, y=424),
        _composer(24, 470, 932, "Components · components.rs:1",
                  "Why does editing a main change every screen?", 2, send=True, menu=True)
    )
    return f


def _screen_notes():
    f = F("Comment model", w=980, h=620, fill=UI_PANEL, stroke=UI_BORDER, radius=12, clip=True)
    f.add(
        _ui("THE COMMENT MODEL", 9, K.MONO, "Medium", UI_ACCENT, name="eyebrow", ls=4, x=48, y=44),
        _ui("One input, two verbs", 30, K.SANS, "Medium", UI_TEXT, name="title", x=48, y=68),
        _ui("The bar at the bottom is the comment box and the prompt box. The selection tells it "
           "what you are talking about.", 12.5, K.SANS, "Regular", UI_MUTED, w=620, name="lede",
            lh=18, x=48, y=116),
        _ui("1", 12, K.MONO, "Medium", UI_ACCENT, name="n1", x=48, y=180),
        _ui("Select.", 13, K.SANS, "Medium", UI_TEXT, name="t1", x=72, y=178),
        _ui("The chip shows the page, card and source. Text selects; ⌘C copies. Right-click is Copy "
           "or Comment, nothing else.", 11.5, K.SANS, "Regular", UI_MUTED, w=330, name="b1", lh=16,
            x=72, y=200),
        _ui("2", 12, K.MONO, "Medium", UI_ACCENT, name="n2", x=520, y=180),
        _ui("Comment (⏎).", 13, K.SANS, "Medium", UI_TEXT, name="t2", x=544, y=178),
        _ui("The input becomes a draft on that selection. Choosing another card clears the input; "
           "the drafts stay above it.", 11.5, K.SANS, "Regular", UI_MUTED, w=340, name="b2", lh=16,
            x=544, y=200),
        _ui("3", 12, K.MONO, "Medium", UI_ACCENT, name="n3", x=48, y=272),
        _ui("Send (⌘⏎).", 13, K.SANS, "Medium", UI_TEXT, name="t3", x=72, y=270),
        _ui("The same input becomes the prompt. Every draft attaches; the target chip picks the "
           "agent, or a new CLI.", 11.5, K.SANS, "Regular", UI_MUTED, w=330, name="b3", lh=16,
            x=72, y=292),
        _ui("4", 12, K.MONO, "Medium", UI_ACCENT, name="n4", x=520, y=272),
        _ui("Nothing else.", 13, K.SANS, "Medium", UI_TEXT, name="t4", x=544, y=270),
        _ui("No layers panel, no design tools, no auto-send. A draft reaches an agent only when "
           "you send it.", 11.5, K.SANS, "Regular", UI_MUTED, w=340, name="b4", lh=16, x=544, y=292),
        F("Flow",
          _ui("Select", 11, K.SANS, "Medium", UI_TEXT, name="s1", x=20, y=9),
          _ui("→", 11, K.SANS, "Regular", UI_MUTED, name="a1", x=78, y=9),
          _ui("⏎ comment", 11, K.SANS, "Medium", UI_TEXT, name="s2", x=102, y=9),
          _ui("→", 11, K.SANS, "Regular", UI_MUTED, name="a2", x=196, y=9),
          _ui("2 drafts", 11, K.SANS, "Medium", UI_ACCENT, name="s3", x=220, y=9),
          _ui("→", 11, K.SANS, "Regular", UI_MUTED, name="a3", x=292, y=9),
          _ui("⌘⏎ send to kimi", 11, K.SANS, "Medium", UI_TEXT, name="s4", x=316, y=9),
          _ui("→", 11, K.SANS, "Regular", UI_MUTED, name="a4", x=440, y=9),
          _ui("reply", 11, K.SANS, "Medium", "#2F6F6A", name="s5", x=464, y=9),
          x=48, y=390, w=520, h=30, fill=UI_SOFT, radius=15),
        F("Right click",
          _ui("Copy", 11, K.SANS, "Regular", UI_TEXT, name="c1", x=14, y=8),
          _ui("Comment", 11, K.SANS, "Medium", UI_ACCENT, name="c2", x=14, y=30),
          _ui("nothing else", 9, K.MONO, "Regular", UI_MUTED, name="c3", x=14, y=52),
          x=712, y=386, w=120, h=74, fill=UI_PANEL, stroke=UI_BORDER, radius=8,
          effects=[shadow(y=4, blur=12, color="#00000014")]),
        _ui("One selection, one input. Comment as often as you like; the drafts wait above the "
           "bar until you send them.", 10, K.MONO, "Regular", UI_MUTED, w=760, name="foot",
            x=48, y=474))
    return f



def build_product(doc):
    page = doc.page("Product", bg=K.BG)
    doc.place(page, K.heading(
        "The workspace client",
        "Herdr is the list of projects. A map is optional, Init gives it an agent, and comments "
        "are drafts you send.", w=2000), 64, 56)
    doc.place(page, _ui("01 · WORKSPACES — every Herdr workspace, with or without a map", 10, K.MONO, "Medium", K.MUTED, name="c1", ls=3), 64, 222)
    doc.place(page, _ui("02 · EMPTY — Init asks which agent, existing or new", 10, K.MONO, "Medium", K.MUTED, name="c2", ls=3), 1092, 222)
    doc.place(page, _ui("03 · INIT — pages appear as the agent renders them", 10, K.MONO, "Medium", K.MUTED, name="c3", ls=3), 64, 902)
    doc.place(page, _ui("04 · COMMENT — one input at the bottom; a draft on the selection", 10, K.MONO, "Medium", K.MUTED, name="c4", ls=3), 1092, 902)
    doc.place(page, _ui("05 · SEND — the same input becomes the prompt; drafts attach", 10, K.MONO, "Medium", K.MUTED, name="c5", ls=3), 64, 1582)
    doc.place(page, _ui("06 · MODEL — select, comment, send; nothing else", 10, K.MONO, "Medium", K.MUTED, name="c6", ls=3), 1092, 1582)
    doc.place(page, _screen_workspaces(), 64, 250)
    doc.place(page, _screen_empty(), 1092, 250)
    doc.place(page, _screen_init(), 64, 930)
    doc.place(page, _screen_map(), 1092, 930)
    doc.place(page, _screen_send(), 64, 1610)
    doc.place(page, _screen_notes(), 1092, 1610)

    rules = Vt(
        _ui("FIVE RULES", 9, K.MONO, "Medium", UI_MUTED, name="label", ls=4),
        _ui("1  Herdr workspaces are the projects; a map is optional.   2  No links: the service "
            "joins the client.   3  Humans select, copy, comment, ask — never edit.", 12, K.SANS,
            "Regular", UI_TEXT, w=1900, name="one", lh=17),
        _ui("4  A comment is a draft until you send it to an agent.   5  Agents write a model; the "
            "service renders components, pages and sources.", 12, K.SANS, "Regular", UI_TEXT,
            w=1900, name="two", lh=17),
        gap=6, pad=22, w=2008, fill=UI_PANEL, stroke=UI_BORDER, radius=10, name="Rules")
    doc.place(page, rules, 64, 2290)
    doc.place(page, F("spacer", x=0, y=0, w=1, h=40, fill=None), 64, 2400)
    return page


# --- screens page ---------------------------------------------------------------

UI_BG = "#F4F4F2"
UI_PANEL = "#FFFFFF"
UI_BORDER = "#E3E2DF"
UI_TEXT = "#1A1A1A"
UI_MUTED = "#6E6C68"
UI_ACCENT = "#B5452F"
UI_SOFT = "#F1F0ED"


def _ui(text, size, font=K.SANS, style="Regular", color=UI_TEXT, w=None, name=None, lh=None, ls=None, x=0, y=0):
    return _t(text, size, font, style, color, w=w, name=name, lh=lh, ls=ls, x=x, y=y)


def _shadow():
    return F("shadow", x=0, y=0, w=1, h=1, fill=None)


def _topbar(w, active="Overview", crumb="FigCraft  /  Components"):
    tabs = ["Overview", "Components", "Handoff"]
    tab_nodes = []
    x = 460
    for name in tabs:
        tab_nodes.append(_ui(name, 12, K.SANS, "Medium", UI_ACCENT if name == active else UI_MUTED,
                             name="tab " + name, x=x, y=21))
        if name == active:
            tab_nodes.append(F("underline", x=x, y=52, w=len(name) * 6.4, h=2, fill=UI_ACCENT))
        x += len(name) * 7 + 26
    people = [("JL", "#6B4E8C"), ("MK", "#2F6F6A"), ("TS", "#8A6D2B")]
    avatars = []
    x = w - 280
    for initials, color in people:
        avatars.append(F("Avatar " + initials,
                         _ui(initials, 9, K.MONO, "Medium", "#FFFFFF", name="initials", x=6, y=5),
                         x=x, y=16, w=22, h=22, fill=color, radius=11))
        x += 18
    return F(
        "Top bar",
        _ui("Mindmap", 15, K.SANS, "Medium", UI_TEXT, name="wordmark", x=24, y=19),
        _ui(crumb, 11, K.MONO, "Regular", UI_MUTED, name="crumb", x=112, y=22),
        *tab_nodes,
        *avatars,
        _ui("3 live", 10, K.MONO, "Medium", "#2F6F6A", name="live", x=w - 160, y=22),
        F("Share", _ui("Share", 11, K.SANS, "Medium", "#FFFFFF", name="label", x=14, y=7),
          x=w - 96, y=14, w=72, h=28, fill=UI_TEXT, radius=6),
        F("line", x=0, y=55, w=w, h=1, fill=UI_BORDER),
        x=0, y=0, w=w, h=56, fill=UI_PANEL, effects=[shadow(y=2, blur=10, color="#00000010")])


def _canvas_card(x, y, index, title, body, link, source="", w=300, h=152):
    return F(
        "Card " + title,
        _ui(f"{index:02d}  ·  {link.upper()}", 9, K.MONO, "Medium", UI_MUTED, name="eyebrow", ls=3, x=16, y=14),
        _ui(title, 15, K.SANS, "Medium", UI_TEXT, name="title", x=16, y=34),
        _ui(body, 11.5, K.SANS, "Regular", UI_MUTED, w=w - 32, name="body", lh=16, x=16, y=58),
        _ui(source, 9, K.MONO, "Regular", "#9A9791", name="source", x=16, y=h - 44),
        _ui("Open  →", 11, K.SANS, "Medium", UI_ACCENT, name="link", x=16, y=h - 26),
        x=x, y=y, w=w, h=h, fill=UI_PANEL, stroke=UI_BORDER, radius=8,
        effects=[shadow(y=2, blur=8, color="#00000010")])


def _ask(w, text, context=None):
    kids = []
    x_text = 20
    if context:
        kids.append(F("Context",
                      _ui(context, 9, K.MONO, "Regular", UI_MUTED, w=272, name="context", x=10, y=7),
                      x=14, y=13, w=292, h=26, fill=UI_SOFT, radius=5))
        x_text = 320
    kids.append(_ui(text, 12.5, K.SANS, "Regular", UI_TEXT, name="question", x=x_text, y=18))
    kids.append(F("Send", _ui("↑", 13, K.SANS, "Medium", "#FFFFFF", name="glyph", x=9, y=5),
                  x=w - 56 - 44, y=11, w=28, h=28, fill=UI_ACCENT, radius=14))
    return F("Ask bar", *kids, x=28, y=524, w=w - 56, h=50, fill=UI_PANEL, stroke=UI_BORDER,
             radius=10, effects=[shadow(y=4, blur=16, color="#00000018")])


def _reply(x, y, w=304, h=170):
    return F("Agent reply",
             _ui("AGENT", 9, K.MONO, "Medium", UI_ACCENT, name="who", ls=4, x=16, y=14),
             _ui("Every instance is rebuilt from its main, keeping layer ids and re-applying "
                 "overrides. That is why a component edit reaches every screen.", 11.5, K.SANS,
                 "Regular", UI_TEXT, w=w - 32, name="text", lh=17, x=16, y=34),
             _ui("crates/engine/src/components.rs:5", 9, K.MONO, "Regular", UI_MUTED,
                 name="source", x=16, y=h - 26),
             x=x, y=y, w=w, h=h, fill=UI_PANEL, stroke=UI_BORDER, radius=10,
             effects=[shadow(y=4, blur=14, color="#00000014")])


def _screens_page(doc):
    page = doc.page("Screens", bg=K.BG)
    doc.place(page, K.heading(
        "The canvas client",
        "No tools, no panels. Pages and people in one thin bar, cards that link to the next "
        "altitude, and a question bar that carries the selection with it.", w=1900), 64, 56)

    project = F("Project screen", w=980, h=620, fill=UI_BG, stroke=UI_BORDER, radius=12, clip=True)
    project.add(
        _topbar(980, active="Overview"),
        _canvas_card(28, 84, 1, "Canvas and layers", "Frames, groups, sections and text are one node tree. The canvas is how you read it now.", "Overview", "doc/node.rs:3"),
        _canvas_card(344, 84, 2, "Components", "A component is a subtree with identity; an instance remembers it and what changed.", "Components", "components.rs:1"),
        _canvas_card(660, 84, 3, "Handoff", "The same file becomes the spec: measurements, annotations, code.", "Handoff", "devinspect.rs:1"),
        _canvas_card(28, 252, 4, "Auto layout", "Frames size themselves and children follow rules instead of coordinates.", "Layout", "layout/lib.rs:3"),
        _canvas_card(344, 252, 5, "Styles and variables", "Colour, number, string and boolean values, with modes and bindings.", "Styles", "doc/lib.rs:170"),
        _canvas_card(660, 252, 6, "Text", "Fonts, shaping, bidi and line layout — a stack of its own.", "Text", "text/lib.rs:3"),
        _ask(980, "Ask about a card, or describe the change…"))
    doc.place(page, project, 64, 250)

    detail = F("Detail screen", w=980, h=620, fill=UI_BG, stroke=UI_BORDER, radius=12, clip=True)
    detail.add(
        _topbar(980, active="Components", crumb="FigCraft  /  Components  /  Variants"),
        _ui("←  Overview", 11, K.SANS, "Medium", UI_ACCENT, name="back", x=28, y=74),
        _canvas_card(28, 100, 1, "A main changes, every instance rebuilds", "The engine rebuilds each instance, keeps layer ids and re-applies the overrides.", "Detail", "components.rs:5", w=420),
        _canvas_card(28, 268, 2, "Overrides are groups", "Fills, strokes, effects, opacity, characters, text style and size reset on their own.", "Detail", "components.rs:11", w=420),
        _reply(500, 100),
        _ask(980, "Why does editing a main change every screen?", context="Components and instances  ·  components.rs:1"))
    doc.place(page, detail, 1092, 250)

    doc.place(page, _ui("01 · PROJECT — cards link down, pages and people stay in the bar", 10, K.MONO, "Medium", K.MUTED, name="cap1", ls=3), 64, 222)
    doc.place(page, _ui("02 · DETAIL — a link moves the canvas; the ask carries its sources", 10, K.MONO, "Medium", K.MUTED, name="cap2", ls=3), 1092, 222)
    doc.place(page, F("spacer", x=0, y=0, w=1, h=64, fill=None), 64, 870)
    return page


def build_screens(doc):
    return _screens_page(doc)


if __name__ == "__main__":
    main()
