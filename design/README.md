# design — how the map is built

Two layers: **content** (what the map says, and what proves it) and **assembly** (how it is drawn,
linked and updated). Prose changes touch only `map_content.py`; visual changes touch `map_kit.py`.

| File | Role |
|---|---|
| `map_content.py` | Areas, capabilities, the anatomy stages, the detail screen and the map-mode sketch. Every code claim has a `(path, literal)` source. |
| `map_kit.py` | One ink per area, the card components (`Branch`, `Leaf`, `Stage`, `Surfaces`, `Root`), the detail screen, the map-mode sketch, page chrome. |
| `fc.py` | Writes a `.figcraft` zip: frames with auto layout, text, rectangles. Stores `gap + 0.5` so a later `node.set gap` forces FigCraft to lay out. |
| `sources.py` | Resolves anchors, reads `Cargo.toml`, counts crates, apps, lines and `spec!` entries, reads the git revision. |
| `build_map.py` | Builds the raw document, drives a headless MCP session to create components and place instances, links the detail screen, writes the sidecar and publishes. |
| `context.py` | The context bundle for a selected layer: page, trail of cards, sources, crop, map version. |
| `check_anchors.py` | Resolves and prints every anchor without building. |

## The pipeline

1. **Resolve.** Every `(path, literal)` becomes `path:line`. A literal must occur exactly once in
   its file. Missing or ambiguous stops the build, naming the card.
2. **Draw the raw document.** `fc.py` writes the component mains on a `Components` page, the page
   blocks, the detail screen and the map-mode sketch. No instance exists yet.
3. **Open in FigCraft.** `build_map.py` starts `figcraft-cli mcp` (or the host's
   `mcp_bridge.py` when publishing) and opens the raw file. The first batch measures wrapped text
   and applies the real gaps.
4. **Make components.** Each set of mains becomes a FigCraft component set (`component.create`,
   `component.combineVariants`), text layers become text properties
   (`component.addProperty` + `bindProperty`), and the `Code` frame becomes a slot
   (`component.makeSlot`).
5. **Place instances.** `instance.create` appends into auto-layout columns; leaves go in through
   `instance.slotInsert`; `instance.setProperties` writes the copy and the `path:line`.
6. **Link the zoom.** The Components area instance gets a `proto.interaction.add` navigate to the
   detail screen's frame; its *Back to the map* link gets an `action: back` interaction.
7. **Place the blocks.** The page blocks are positioned below their measured headings
   (`get_node` heights); the detail screen is placed to their right. Page bounds come from frames,
   so footers and captions live inside or below a frame.
8. **Save, describe, export.** `save_file` writes `build/figcraft-feature-map.figcraft`;
   the sidecar `build/figcraft-feature-map.json` records version, revision, counts and pages, for
   `context.py`; `figcraft-cli render --page` writes `build/shots/*.png`.

## Adding an area, a capability or a detail card

All data. An area is one dict in `BRANCHES`; a capability is one dict in its `leaves`; a detail
card is one dict in `DETAIL["cards"]`. Give the area an `area` key that already has an ink in
`K.AREAS` (a new one needs a colour), and give the card a source that is a single distinctive
line. Then:

```sh
python3 check_anchors.py    # the new source resolves once
python3 build_map.py        # the card appears, the counts update themselves
```

The page statistics (area count, capability count, command count, anchor count, version,
revision) are computed, not typed. A new variant — another area — is just another main in
`map_kit.py`; the build creates the set from the mains it is given.

## Component model

| Main | Variants | Text properties | Slot |
|---|---|---|---|
| `Branch` | one per area (12) | Name, Role, Source | `Code` holds its leaves |
| `Leaf` | one per area (12) | Title, Fact, Source | — |
| `Stage` | one | Number, Name, Detail, Source | — |
| `Surfaces` | one | Body, Source | — |
| `Root` | one | Claim, Facts, Source | — |
| `Arrow` | one | — | — |

The detail screen reuses `Leaf` for its cards; the map-mode sketch is chrome, drawn with positioned
frames and text, and is labelled as a proposal so it is not mistaken for a code claim.

## The context bundle

`context.py` joins or opens a document, finds the selection (or `--node`), and walks up to the
nearest component instances. It reads their `componentProperties`, so the bundle carries the
semantic trail (`Features › Components and instances › Variants and properties`), the sources and
a PNG crop, not just a layer id. Two modes:

```sh
python3 context.py --file build/figcraft-feature-map.figcraft --node 322
python3 context.py --link "figcraft://join?…" --node 17179869441
```

The live mode is the one that matters: an agent attached to the hosted copy can answer from a
click. The selection is per client — a headless join has none, so pass the id the app reports.

## Updating the hosted copy

The hosted file keeps its id and links. The update path is join → delete the old pages →
`file.import` the rebuilt document → delete the leftover page. Import copies pages, components and
prototype interactions (verified on 2026-10-18: 4 pages, 30 components, the Components → detail
navigate link).

`build_map.py --publish` is for the first copy only: publishing twice under one name creates a
**second** server file, so the build refuses when the name is already hosted.

## Limits

- The engine is the **v0.5.1 CLI** installed in `figcraft-host`, while the map describes the
  **0.7.0** source. Commands used here exist in both; do not reach for newer commands without
  upgrading the host.
- Fonts: Charter (titles), Inter (body), JetBrains Mono (sources). FigCraft embeds Inter and falls
  back gracefully, but the desktop app and the headless CLI can measure text slightly differently.
  Auto layout absorbs the difference; fixed frames would not.
- Page bounds are the union of **frames**. A loose text node outside a frame is clipped by
  `render --page`, so keep footers inside the block.
- The map-mode sketch is a drawing, not a prototype: it has no interactions and no anchors. It is
  there to be discussed and then built for real.
