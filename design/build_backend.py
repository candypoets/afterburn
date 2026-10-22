#!/usr/bin/env python3
"""A backend-code map page: the service and document engine, without frontend/UI code."""
from pathlib import Path

from fc import Doc, F, R, T, shadow

BG = "#F4F4F2"
PANEL = "#FFFFFF"
SOFT = "#F8F6F3"
BORDER = "#E3E2DF"
TEXT = "#1A1A1A"
MUTED = "#6E6C68"
FAINT = "#9A9791"
ACCENT = "#B5452F"
TEAL = "#2F6F6A"
SANS = "Inter"
MONO = "JetBrains Mono"
DISP = "Charter"

W = 2000
CARD_W = 888
CARD_H = 372
X1, X2 = 64, 1048
Y = [236, 646, 1056]


def t(text, size, font=SANS, style="Regular", color=TEXT, x=0, y=0, w=None, lh=None, name=None):
    return T(text, size, font=font, style=style, color=color, x=x, y=y, w=w, lh=lh, name=name)


def card(x, y, eyebrow, title, body, rows):
    kids = [
        t(eyebrow, 9, MONO, "Medium", ACCENT, x=24, y=18, name="eyebrow"),
        t(title, 21, SANS, "Medium", TEXT, x=24, y=38, name="title"),
        t(body, 11.5, SANS, "Regular", MUTED, x=24, y=74, w=CARD_W - 48, lh=16, name="body"),
    ]
    for i, (path, desc) in enumerate(rows):
        yy = 126 + i * 50
        kids.append(F("divider", x=24, y=yy - 9, w=CARD_W - 48, h=1, fill=BORDER))
        kids.append(t(path, 9.5, MONO, "Medium", ACCENT, x=24, y=yy, name="path " + path))
        kids.append(t(desc, 10.2, SANS, "Regular", MUTED, x=24, y=yy + 14, w=CARD_W - 48, lh=13,
                      name="desc " + path))
    return F("Card " + title, *kids, x=x, y=y, w=CARD_W, h=CARD_H, fill=PANEL, stroke=BORDER,
             radius=10, effects=[shadow(y=3, blur=10, color="#00000010")])


def build(path: Path):
    doc = Doc("backend code")
    page = doc.page("backend code", bg=BG)
    doc.nodes[str(page)]["width"] = W
    doc.nodes[str(page)]["height"] = 1500
    doc.place(page, R(W, 1500, fill=BG, name="Page bounds"), 0, 0)
    doc.place(page, t("Backend code", 36, DISP, "Regular", TEXT, x=0, y=0, name="title"), 64, 48)
    doc.place(page, t("The Rust service and document engine behind Mindmap. Frontend, UI and the "
                      "Python authoring tooling are deliberately out of scope.", 13, SANS, "Regular",
                      MUTED, x=0, y=0, w=1760, lh=19, name="lede"), 64, 100)
    doc.place(page, F("Boundary",
                      t("IN SCOPE", 8.5, MONO, "Medium", TEAL, x=20, y=18, name="in"),
                      t("service/src  ·  server/figcraft/crates/{doc,engine,collab,collab-net,format,render,svg,pdf,geom,pathops,color,text,layout,mcp,plugin}  ·  apps/figcraft-server",
                        9.5, MONO, "Regular", TEXT, x=108, y=17, name="in paths"),
                      t("OUT OF SCOPE", 8.5, MONO, "Medium", ACCENT, x=20, y=38, name="out"),
                      t("client-src (incl. ui-egui)  ·  apps/figcraft  ·  apps/figcraft-web  ·  design/ Python  ·  tools/dsh-tui-eur  ·  apps/figcraft-cli is a client tool",
                        9.5, MONO, "Regular", MUTED, x=108, y=37, name="out paths"),
                      x=0, y=0, w=W - 128, h=62, fill=PANEL, stroke=BORDER, radius=8),
               64, 150)
    doc.place(page, card(X1, Y[0], "01 · SERVICE API", "One TCP API",
                         "The app speaks newline JSON to one Rust process. Methods validate, resolve a workspace, then call the rest of the service.",
                         [("src/server.rs", "Request dispatch and validation: health, workspaces, open, init, adopt, send, diff, sync, sync_status."),
                          ("src/main.rs", "CLI entry points and `serve`; argument parsing and startup."),
                          ("src/config.rs", "Repository paths, document server/CLI/Herdr, environment overrides."),
                          ("src/link.rs", "Join-link parsing, host rewriting and percent encoding."),
                          ("src/error.rs · src/time.rs", "Shared error type, timestamps and small transport helpers.")]), 64, Y[0])
    doc.place(page, card(X2, Y[0], "02 · PROJECTS & AGENTS", "Herdr is the inventory",
                         "One workspace is one Herdr project. Open publishes or reuses its FigCraft document; Init asks an agent to draw the first map.",
                         [("src/project.rs", "Inventory, document publish/reuse, Init/adopt, send and sync orchestration."),
                          ("src/herdr.rs", "Typed wrapper over Herdr workspaces, tabs, panes and agents."),
                          ("src/agent.rs", "Target choice, prompt submission, trust handling, dsh-tui pane fallback, installed CLIs."),
                          ("src/records.rs", "Project store: file/edit links, built flag and last successful sync."),
                          ("src/mcp.rs", "FigCraft CLI MCP session: publish, save, inspect and join links.")]), X2, Y[0])
    doc.place(page, card(X1, Y[1], "03 · COMMENTS & SYNC", "Two honest diffs",
                         "Comments become one prompt bundle. Sync compares the map snapshot with the last one and git with the last synced commit; only a finished agent writes the baseline.",
                         [("src/comments.rs", "Page/node/text bundle, sanitisation, prompt formatting and limits."),
                          ("src/sync.rs", "Snapshot validation, card-by-id diff, git drift, prompt builder, job transitions."),
                          ("src/agent.rs", "Submits the sync prompt and reports in-flight, done or failed."),
                          ("src/records.rs", "`last_sync`: commit + map snapshot, written only after success.")]), X1, Y[1])
    doc.place(page, card(X2, Y[1], "04 · DOCUMENT ENGINE", "The node tree and commands",
                         "The FigCraft fork underneath the map. Documents are page trees; every edit is a command with history.",
                         [("crates/doc", "Pages and nodes, IDs, transforms, paints, components/variants, text and the document model."),
                          ("crates/engine", "Command specs, history/undo, session and live state, selection, components, layout."),
                          ("crates/format", "`.figcraft` zip and REST file formats, migrations, import/export plumbing."),
                          ("crates/mcp · crates/plugin", "MCP bridge and the plugin host.")]), X2, Y[1])
    doc.place(page, card(X1, Y[2], "05 · COLLAB & SERVER", "Live documents and storage",
                         "The document server keeps edits live and stores files. It is a backend binary, not the desktop client.",
                         [("crates/collab", "Live document state, peers, cursors and operations."),
                          ("crates/collab-net", "Wire protocol and transport for live sessions."),
                          ("apps/figcraft-server", "Server binary: entry point and public API."),
                          ("src/{config,store,disk,directory,codes,abuse}.rs", "Configuration, document store, disk layout, access directory/codes and abuse limits."),
                          ("src/{main,lib,cli}.rs", "Process entry, server library and CLI surface.")]), X1, Y[2])
    doc.place(page, card(X2, Y[2], "06 · RENDER & PRIMITIVES", "Drawing and math",
                         "Shared Rust crates used by both the engine and the server. Rendering is backend; the egui editor is not.",
                         [("crates/render", "Raster/vector rendering pipeline."),
                          ("crates/svg · crates/pdf", "SVG and PDF exporters."),
                          ("crates/geom · crates/pathops", "Affine geometry, rectangles, paths and boolean operations."),
                          ("crates/color · crates/text · crates/layout", "Color, text shaping and auto-layout."),
                          ("crates/ui-egui · client-src · apps/figcraft-web", "Excluded frontend/UI; listed here only to mark the boundary.")]), X2, Y[2])
    doc.place(page, t("Every module above is backend or shared runtime code. The client's read/comment/sync UI, "
                      "the egui editor and the Python design pipeline never ship in the service.", 9.5, MONO,
                      "Regular", FAINT, x=0, y=0, w=W - 128, name="foot"), 64, Y[2] + CARD_H + 22)
    doc.save(str(path))
    print(f"wrote {path}")


if __name__ == "__main__":
    build(Path(__file__).resolve().parent.parent / "build" / "backend-code.figcraft")
