---
name: mindmap
description: Build and maintain a feature map of a repository as a model, rendered into FigCraft. Use when asked to initialise, update or extend the map of a repo, or when an "Init map" job arrives.
---

# Mindmap

You maintain a **map of what a repository does**, not a drawing. You write a model; the service
renders it into FigCraft with the shared components. The map is read by a human who will never
edit it, and asked questions about it.

## Output

Write `.mindmap/model.json` in the repository and render it:

```sh
mindmap render .mindmap/model.json --project <id>
```

Then look at the rendered PNG the command returns. Iterate on the model, not on the canvas.

## Live comments and Sync

The service also sends comments and new-page prompts against an **existing live FigCraft
document**. For those jobs:

- Do **not** write `.mindmap/model.json` or re-render the whole map.
- The context packet names the document, page id, card id, path, text and the owner edit link.
- Run the FigCraft CLI with `mcp`, `collab.join` the edit link, `get_node` the page/card to read
  it, then `run_command` to change or add the named cards.
- Work only on the named page and card; keep card names stable; source every claim.

## The model

```json
{
  "project": "name",
  "pages": [
    {
      "title": "Overview",
      "areas": [
        {
          "name": "Canvas and layers",
          "role": "one sentence: what this part is for",
          "source": ["crates/doc/src/node.rs", "one struct with the properties every node can have"],
          "leaves": [
            {
              "title": "One node tree",
              "fact": "one or two sentences: what it does, and how it is built",
              "source": ["crates/doc/src/node.rs", "//! Nodes:"]
            }
          ]
        }
      ]
    }
  ]
}
```

- **6–12 areas** on the overview. Each is a part of the system a person would name.
- **3–5 leaves** per area. One claim per leaf; if it needs "and", split it.
- `source` is a `(path, literal)` pair: the literal must occur **exactly once** in that file. It is
  the proof of the card. Never cite a line number, and never invent a symbol.
- A detail page (a second `pages` entry) only when the overview is too thin: the same areas at
  one altitude lower, still sourced.

## How to read a repo

1. README, manifests (`Cargo.toml`, `package.json`, `pyproject.toml`), entry points, top-level
   directories, `docs/`.
2. Name the **areas** from how the system is built and used — not from the directory tree alone.
3. For each leaf, find the single line that proves it: a module doc, a command registration, a type
   definition, a route, a job.
4. Prefer the code that is true today; mark proposals as proposals, never as facts.

## Rules

- No free shapes, no styling, no coordinates: the renderer owns layout and components.
- No wall of text. A leaf is a claim, not a paragraph.
- Every card sourced. If you cannot find the line, the claim is not ready — leave it out.
- Keep names stable across updates: the map's history is diffed by name.
- On a code change, re-read only what changed, update the affected cards, render, and report what
  moved.

## When the map is empty

Start with the overview only. Do not open detail pages until the overview has 6–12 areas with
sources. The Init job is done when the overview renders and every card has a source.
