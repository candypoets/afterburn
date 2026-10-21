# mindmap

A feature map of **FigCraft** — the ArtCraft, pure-Rust Figma clone — drawn in FigCraft itself,
and the generator that keeps it true to the code.

The map is published on the team server as **"FigCraft · feature map"**: `http://befree:7480` on
the LAN, or the Tailscale address when you are away.

It is a visual layer between a person, an agent and the code. The person reads and points; the
agent maintains the map and answers; the code stays the source of truth behind every card.

## What it shows

**Features** — the whole surface, area by area. Twelve branches: Canvas and layers, Auto layout
and constraints, Components and instances, Styles and variables, Vector and shapes, Paint,
strokes and effects, Text, Prototyping, Handoff, Files and formats, Multiplayer and server,
Extend and automate. Forty-five leaves say what you can do and, in a sentence, how the code
makes it work.

**The detail screen** — the first "zoom in". Clicking **Components and instances** in the map
opens *How an instance stays a copy*: five deeper claims (rebuilds, override groups, variant
switches, slots, libraries), each with its own source. The back link returns to the map. It sits to
the right of the map on the Features page and is a prototype link, so it also plays in presentation.

**Anatomy** — how a feature is built: a panel calls a command, the command edits the document,
layout and text give it geometry, the renderer gives it pixels, and the file or the wire carries
it. Plus the seam that makes the tool agent-friendly: the menu, a shortcut, the Actions palette,
the CLI, the MCP server, the control channel and a plugin all dispatch the same command ids.

**Map mode** — a drawn proposal, not a claim about the code: the same document with the drawing
tools gone, a selected card, a question, and the map's own change history. It is here to be
looked at and argued with.

**Components** — every main: `Branch`, `Leaf`, `Stage`, `Surfaces`, `Root`, `Arrow`. Edit a main
and every page follows.

## The system design

A second, single-page design on the same 0.7.0 server: **"Mindmap · system"**. It draws the shape we
are building — the Mac client over Tailscale, one host service, the document plane and the
execution plane (the private Herdr) in the same process, and what a user installs versus what they
never see. `design/build_system.py` builds and publishes it; it can be opened on the Mac through
the app's control channel and `ui.render`.

## The client

**Mindmap.app** is the Mac client: FigCraft's engine and canvas with the editor taken off the
human's screen. `--map-mode` hides the layers and design panels, the toolbar, the tab bar and the
frame labels, and adds a top bar with one tab per page and the live count. The app is renamed and
carries its own icon; eframe's runtime Dock-icon override is skipped in map mode so the bundle's
icon is the one shown. The patch, the icon and `build-mac.sh`
live in [`client/`](client/README.md). It is a UI gate, not yet a permission model: editing
shortcuts and the writable document remain, and page navigation is done by the control channel
until the map has in-canvas links.

The installed FigCraft.app (0.5.1) stays untouched; Mindmap is a separate 0.7.0 bundle talking to
our 0.7.0 server.

## Click, then ask

The part that makes this more than a document is `design/context.py`. Given a selected layer — in
the built file or in the live hosted copy — it walks up to the component instances that carry the
meaning and prints one JSON bundle: the page, the trail of cards from the area to the leaf, their
text and `path:line` sources, a crop of the map around the selection, and the map's version.

```sh
python3 design/context.py --node 322
python3 design/context.py --link "figcraft://join?server=…&file=…&token=…" --node 17179869441
```

That bundle is what an agent needs to answer *"what does this mean, and where does it live?"*
without the person retyping the context. Clicking a card is the fastest way to ask about it.

## Anchors

A card's source is `(path, literal)` in [`design/map_content.py`](design/map_content.py). The
build finds that literal in that file, requires it exactly once, and shows `path:line` on the
card. Move or rename the line and the build stops, naming the card. The map cannot invent a
feature, a flag or a type the code does not have.

## Build

The scripts use only the Python standard library; the engine is the FigCraft CLI.

```sh
git clone --depth 1 https://github.com/storytold/figcraft .figcraft-src

python3 design/check_anchors.py       # resolve every source; print what moved
python3 design/build_map.py --check   # the same, as the build sees it
python3 design/build_map.py           # build/figcraft-feature-map.figcraft + shots/
python3 design/build_map.py --publish # first publish only: creates a new server file
```

It uses `figcraft-cli` at `/root/figcraft-host/vendor/figcraft-cli` and, for publishing, the host
bridge at `/root/figcraft-host/app/mcp_bridge.py` — that is where the server key lives. Override
the paths with `FIGCRAFT_CLI`, `FIGCRAFT_BRIDGE` and `FIGCRAFT_SERVER_DATA`.

## Updating the hosted copy

The hosted file keeps its id and its links. An update joins it, deletes its pages and imports the
rebuilt document (`file.import` copies pages, components and interactions), so the team's copy
becomes the new build in place:

```text
join the hosted file → page.delete (old pages) → file.import build/figcraft-feature-map.figcraft
→ page.delete (the leftover) → verify pages, components, and one prototype link
```

Publishing (`--publish`) is only for the first copy: it creates a new server file, which is why the
build refuses to publish when the name is already hosted.

## The FigCraft checkout

`.figcraft-src` is the pinned reference (0.7.0, `99e5db3`) the anchors resolve against. When the map
work moves into FigCraft itself, edit `figcraft-work/`, a worktree of the same clone on the
`map-mode` branch:

```sh
git -C .figcraft-src worktree list
cargo build -p figcraft --manifest-path figcraft-work/Cargo.toml
```

Both are git-ignored: this repository holds the map and the generator, not a vendored FigCraft.

## Why this exists

A chat window is a poor interface to a large system: too little context, too much scrolling, and
the words drift between people and agents. A maintained visual map fixes the terms and the place.
The person commands the altitude — zoom in on this, rename that, show the failure paths — and the
agent keeps the drawing true enough to be useful, with its history as the record.

The next steps toward that editor are the ones this map already leans on: a repository/commit
binding for the document, anchors that survive a rebuild, prototype navigation for every area, and
a selection that always carries its context bundle.

## Layout

```
design/
  map_content.py   the copy, and every (path, literal) anchor
  map_kit.py       the visual kit: one ink per area, cards, the detail screen, the map-mode sketch
  fc.py            the minimal .figcraft writer (frames, auto layout, text)
  sources.py       anchor resolution, repo statistics, revision
  build_map.py     assemble pages, create components, place instances, publish
  build_system.py  build and publish the system design and its screens page
  context.py       the context bundle for a selected layer
  check_anchors.py resolve and print every anchor
```

See [`design/README.md`](design/README.md) for the mechanics.
