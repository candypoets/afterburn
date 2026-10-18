# mindmap

A feature map of **FigCraft** — the ArtCraft, pure-Rust Figma clone — drawn in FigCraft itself,
and the generator that keeps it true to the code.

The map is published on the team server as **"FigCraft · feature map"**: `http://befree:7480` on
the LAN, or the Tailscale address when you are away.

## What it shows

**Features** — the whole surface, area by area. Twelve branches: Canvas and layers, Auto layout
and constraints, Components and instances, Styles and variables, Vector and shapes, Paint,
strokes and effects, Text, Prototyping, Handoff, Files and formats, Multiplayer and server,
Extend and automate. Forty-five leaves say what you can do and, in a sentence, how the code
makes it work.

**Anatomy** — how a feature is built, at a high level: a panel calls a command, the command edits
the document, layout and text give it geometry, the renderer gives it pixels, and the file or the
wire carries it. Plus the seam that makes the tool agent-friendly: the menu, a shortcut, the
Actions palette, the CLI, the MCP server, the control channel and a plugin all dispatch the same
command ids.

**Components** — every main: `Branch`, `Leaf`, `Stage`, `Surfaces`, `Root`, `Arrow`. Edit a main
and every page follows.

Every card is an instance of a component, and a leaf lives in its area's `Code` slot, so the
containment on the canvas is the containment in the model.

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
python3 design/build_map.py           # build/figcraft-feature-map.figcraft + build/shots/*.png
python3 design/build_map.py --publish # ...and publish it to the team server
```

It uses `figcraft-cli` at `/root/figcraft-host/vendor/figcraft-cli` and, for publishing, the host
bridge at `/root/figcraft-host/app/mcp_bridge.py` — that is where the server key lives. Override
the paths with `FIGCRAFT_CLI`, `FIGCRAFT_BRIDGE` and `FIGCRAFT_SERVER_DATA`.

Outputs go to `build/`, which is git-ignored. The published document is the shareable artifact;
the repository holds the content and the proof.

## Publishing, honestly

`--publish` creates a **new** server file. The server has no "replace this file" operation exposed
to agents, and publishing twice under one name would leave two copies, so the build refuses when
a file with that name already exists.

The document that was here first ("FigCraft · code map") was a crate-first draft and has been
removed from the server; nothing of it is left to mistake for this one.

## Why this exists

The map documents FigCraft feature-wide, as a reference a person and an agent can share. It is
also the shape the next step needs: a document whose cards are claims about code. Turning FigCraft
into an AI code-map editor means giving it what this map currently does outside the canvas:

1. **A repository binding** — the file knows its remote, ref and commit.
2. **Anchors in the model** — a span on a card, checked when the file opens and on rebuild.
3. **Map-level commands** — add area, revise capability, link an anchor, not hundreds of node edits.
4. **Git** — a commit per accepted map change, and a map branch opened from a commit.
5. **Diff by meaning** — compare area, capability and anchor keys, never layer ids.

FigCraft already has the expensive half: one command surface, a persistent document, components
with slots, live sessions and branches.

## Layout

```
design/
  map_content.py   the copy, and every (path, literal) anchor
  map_kit.py       the visual kit: one ink per area, the card components, page chrome
  fc.py            the minimal .figcraft writer (frames, auto layout, text)
  sources.py       anchor resolution, repo statistics, revision
  build_map.py     assemble pages, create components, place instances, publish
  check_anchors.py resolve and print every anchor
```

See [`design/README.md`](design/README.md) for the mechanics.
