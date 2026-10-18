# design — how the map is built

Two layers: **content** (what the map says, and what proves it) and **assembly** (how it is drawn
and published). Prose changes touch only `map_content.py`; visual changes touch `map_kit.py`.

| File | Role |
|---|---|
| `map_content.py` | Areas, capabilities, the anatomy stages and the surfaces card. Every entry has a `(path, literal)` source. |
| `map_kit.py` | One ink per area, the card components (`Branch`, `Leaf`, `Stage`, `Surfaces`, `Root`), page chrome. |
| `fc.py` | Writes a `.figcraft` zip: frames with auto layout, text, rectangles. Stores `gap + 0.5` so a later `node.set gap` forces FigCraft to lay out. |
| `sources.py` | Resolves anchors, reads `Cargo.toml`, counts crates, apps, lines and `spec!` entries, reads the git revision. |
| `build_map.py` | Builds the raw document, then drives a headless MCP session to create components, place instances and publish. |
| `check_anchors.py` | Resolves and prints every anchor without building. |

## The pipeline

1. **Resolve.** Every `(path, literal)` becomes `path:line`. A literal must occur exactly once in
   its file. Missing or ambiguous stops the build, naming the card.
2. **Draw the raw document.** `fc.py` writes the component mains on a `Components` page and empty
   containers on the content pages. No instance exists yet, so nothing is duplicated.
3. **Open in FigCraft.** `build_map.py` starts `figcraft-cli mcp` (or the host's
   `mcp_bridge.py` when publishing) and opens the raw file. The first batch measures wrapped text
   and applies the real gaps.
4. **Make components.** Each set of mains becomes a FigCraft component set (`component.create`,
   `component.combineVariants`), text layers become text properties
   (`component.addProperty` + `bindProperty`), and the `Code` frame becomes a slot
   (`component.makeSlot`).
5. **Place instances.** `instance.create` appends into auto-layout columns; leaves go in through
   `instance.slotInsert`; `instance.setProperties` writes the copy and the `path:line`.
6. **Place the blocks.** The page blocks are positioned below their measured headings
   (`get_node` heights); page bounds come from frames, so footers live inside a block frame.
7. **Save and export.** `save_file` writes `build/figcraft-feature-map.figcraft`;
   `figcraft-cli render --page` writes `build/shots/*.png`.

## Adding an area or a capability

Both are data. An area is one dict in `BRANCHES`; a capability is one dict in its `leaves`. Give
the area an `area` key that already has an ink in `K.AREAS` (a new one needs a colour), and give
the card a source that is a single distinctive line. Then:

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

## Editing rules

- **One capability per leaf.** If a card needs "and", it is two leaves.
- **Cite behaviour, not names.** A literal from the file that makes the claim true; not a path
  alone, and not a line number (numbers move).
- **Never put an invented feature in a card.** If the code has no such command, flag or type, the
  card is describing something else.
- **Chrome is allowed to be free.** Headings, the reading key and the footer are plain frames;
  they carry no semantics and a future diff ignores them.

## Publishing

`build_map.py --publish` calls `publish_project`, which the host bridge turns into
`collab.publish` with the server key. The name defaults to `FigCraft · feature map` (`--name`).

Publishing twice under one name creates a **second** server file, so the build refuses when the
name is already hosted; pass `--force` to do it anyway. Replacing a hosted file in place needs the
server's admin connection (`Admin::Remove` / `Admin::Rename` in `crates/collab/src/message.rs`);
that is how the first, crate-first draft was removed after this map was published. It is not wired
into this script yet.

## Limits

- The engine is the **v0.5.1 CLI** installed in `figcraft-host`, while the map describes the
  **0.7.0** source. Commands used here exist in both; do not reach for newer commands without
  upgrading the host.
- Fonts: Charter (titles), Inter (body), JetBrains Mono (sources). FigCraft embeds Inter and falls
  back gracefully, but the desktop app and the headless CLI can measure text slightly differently.
  Auto layout absorbs the difference; fixed frames would not.
- Page bounds are the union of **frames**. A loose text node outside a frame is clipped by
  `render --page`, so keep footers inside the block.
