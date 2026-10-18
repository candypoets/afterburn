"""The content of "FigCraft · feature map".

Feature first, code second. A branch is an area of the tool a person would name
("components", "prototyping", "handoff"); a leaf is one thing you can do there
and, in a sentence, how the code makes it work. Every leaf cites one line of the
repository — the build resolves it to path:line and stops if it moved.

What is deliberately absent: a tour of the crates. The crates are the answer to
"where", not the question. The anatomy page traces one path through them.
"""

ROOT = {
    "eyebrow": "FEATURE MAP",
    "name": "FigCraft",
    "claim": "A Figma-class design tool in pure Rust: frames, layout, components, vector "
             "networks, text, prototypes, handoff, multiplayer — and every feature is a "
             "command an agent can call.",
    "source": ("README.md", "clean-room reimplementation of Figma"),
}

KEY = {
    "title": "How to read this map",
    "lede": "A branch is an area of the tool. Its leaves are the things you can do there, "
            "and the line under each one is the code that makes it work.",
    "rows": [
        ("AREA", "one part of the feature surface, one ink"),
        ("LEAF", "a capability and the shape of its implementation"),
        ("SLOT", "leaves live inside their area — the containment is the model"),
        ("SOURCE", "path:line, resolved from the repo at build time"),
    ],
    "note": "If a cited line moves, the build stops.",
}

# area: the ink key and the branch's variant value.
# title: the name a person would use.
# role: the idea of the area in one sentence.
# leaves: title, fact (what it does, and how it is built), source.

BRANCHES = [
    {
        "area": "CANVAS",
        "title": "Canvas and layers",
        "role": "The document is one tree. The canvas is how you edit it, and every gesture is "
                "an edit to that tree.",
        "source": ("crates/doc/src/node.rs", "one struct with the properties every node can have"),
        "leaves": [
            {
                "title": "One node tree",
                "fact": "Frames, groups, sections, shapes, text, vectors, components, instances "
                        "and slices are one node struct with per-kind data. The same tree is laid "
                        "out, drawn, saved and synced.",
                "source": ("crates/doc/src/node.rs", "//! Nodes:"),
            },
            {
                "title": "Pointer tools",
                "fact": "Move, resize, rotate, marquee and vector edit turn gestures into "
                        "document edits through interactions; one gesture is one undo step.",
                "source": ("crates/engine/src/tools.rs", "//! Pointer tools."),
            },
            {
                "title": "Snapping and measuring",
                "fact": "Edges and centres snap to nearby layers, the parent frame, layout grid "
                        "columns and ruler guides; Alt measures distances and padding.",
                "source": ("crates/engine/src/snap.rs", "//! Snapping and smart guides:"),
            },
            {
                "title": "The layers panel",
                "fact": "Pages, the layer tree, rename, visibility, locking and drag, built to "
                        "stay usable in files with tens of thousands of layers.",
                "source": ("crates/ui-egui/src/layers.rs", "//! The left panel:"),
            },
        ],
    },
    {
        "area": "LAYOUT",
        "title": "Auto layout and constraints",
        "role": "Frames size themselves and their children follow rules instead of coordinates.",
        "source": ("crates/layout/src/lib.rs", "Figma-style auto layout"),
        "leaves": [
            {
                "title": "Direction, wrap and grid",
                "fact": "Horizontal and vertical flows, wrapping, and grids with fixed, hug and "
                        "fr tracks and cells that span — computed as a pure function of the tree.",
                "source": ("crates/layout/src/lib.rs", "Figma-style auto layout"),
            },
            {
                "title": "Hug, fill, fixed",
                "fact": "Each child chooses its size, grows to fill the space, or stays fixed; "
                        "min and max bound it, and absolute children opt out of the flow.",
                "source": ("crates/ui-egui/src/layout_panel.rs", "//! The Design panel's layout section:"),
            },
            {
                "title": "Constraints",
                "fact": "Children of a regular frame pin or scale against its edges when the "
                        "frame resizes, so a screen survives a different device size.",
                "source": ("crates/layout/src/lib.rs", "children of a regular frame follow their constraints"),
            },
        ],
    },
    {
        "area": "COMPONENTS",
        "title": "Components and instances",
        "role": "A component is a subtree with identity; an instance is a copy that remembers it "
                "and what changed.",
        "source": ("crates/engine/src/components.rs", "Components and instances."),
        "leaves": [
            {
                "title": "Mains and instances",
                "fact": "An instance's layers are real nodes copied from its main, each recording "
                        "the node it mirrors, so overrides survive edits to the main.",
                "source": ("crates/engine/src/components.rs", "An instance's layers are real nodes"),
            },
            {
                "title": "Variants and properties",
                "fact": "Component sets switch by variant; variant, boolean, text, instance-swap "
                        "and slot properties drive the copies inside them.",
                "source": ("crates/engine/src/components.rs", "Variants are components inside a component set"),
            },
            {
                "title": "The Assets panel",
                "fact": "Components as live thumbnails in folders, searched, clicked to place or "
                        "dragged onto the canvas and into an auto layout frame.",
                "source": ("crates/ui-egui/src/assets_panel.rs", "//! The Assets tab:"),
            },
            {
                "title": "Libraries",
                "fact": "A file publishes components, sets, styles and variables; other files "
                        "enable it by link and receive asset-level updates.",
                "source": ("crates/engine/src/library.rs", "//! Local libraries"),
            },
        ],
    },
    {
        "area": "STYLES",
        "title": "Styles and variables",
        "role": "Design decisions live as data and are bound to layers, so they are made once.",
        "source": ("crates/doc/src/styles.rs", "//! Shared styles and variables."),
        "leaves": [
            {
                "title": "Styles",
                "fact": "Paint, text, effect and layout grid styles, with folders and order, "
                        "shared inside a file and through libraries.",
                "source": ("crates/doc/src/styles.rs", "//! Shared styles and variables."),
            },
            {
                "title": "Variables and modes",
                "fact": "Colour, number, string and boolean variables in collections, with modes, "
                        "aliases and scopes; a light and a dark theme are two modes of one value.",
                "source": ("crates/doc/src/lib.rs", "pub variables: Variables,"),
            },
            {
                "title": "The Variables table",
                "fact": "Collections on the left — this file's, then each library's — and a table "
                        "with one value column per mode, edited in place.",
                "source": ("crates/ui-egui/src/variables_panel.rs", "//! The Variables editor:"),
            },
            {
                "title": "Bindings",
                "fact": "Fills, strokes, effects, layout grids, text styles and instance "
                        "properties can hold a variable instead of a value.",
                "source": ("crates/engine/src/style_vars.rs", "Variables bound inside shared styles."),
            },
        ],
    },
    {
        "area": "VECTOR",
        "title": "Vector and shapes",
        "role": "Real vector networks, booleans you can still edit, and a shape builder.",
        "source": ("crates/doc/src/vector.rs", "//! Vector networks:"),
        "leaves": [
            {
                "title": "Vector networks",
                "fact": "Vertices joined by curved segments, any number per vertex, with filled "
                        "regions made from closed loops.",
                "source": ("crates/doc/src/vector.rs", "//! Vector networks:"),
            },
            {
                "title": "Vector edit mode",
                "fact": "Move points, segments and bezier handles on the canvas, with its own pen, "
                        "bend, paint bucket, lasso and cut tools.",
                "source": ("crates/engine/src/vectoredit.rs", "//! Vector edit mode:"),
            },
            {
                "title": "Booleans",
                "fact": "Union, subtract, intersect and exclude; the operands stay editable and "
                        "the result follows every drag.",
                "source": ("crates/pathops/src/lib.rs", "Union, Subtract, Intersect and Exclude"),
            },
            {
                "title": "Shape builder",
                "fact": "Click or drag across the regions where overlapping shapes cross to merge "
                        "them; Alt removes instead.",
                "source": ("crates/engine/src/shapebuilder.rs", "//! The shape builder:"),
            },
            {
                "title": "Shapes and arcs",
                "fact": "Rectangles with smooth corners, polygons, stars, and arcs and rings "
                        "shaped with on-canvas sweep, start and ratio handles.",
                "source": ("crates/engine/src/arc_handles.rs", "//! Arc handles:"),
            },
        ],
    },
    {
        "area": "PAINT",
        "title": "Paint, strokes and effects",
        "role": "Fills, strokes and effects are document values; one renderer draws them the same "
                "on screen and in every export.",
        "source": ("crates/color/src/lib.rs", "One fill or stroke paint."),
        "leaves": [
            {
                "title": "Fills",
                "fact": "Solid, linear, radial, angular and diamond gradients with on-canvas "
                        "handles, and image fills with fit, crop, tile and adjustments.",
                "source": ("crates/color/src/lib.rs", "Image(ImagePaint),"),
            },
            {
                "title": "Effects",
                "fact": "Drop and inner shadows, layer and background blur (uniform or "
                        "progressive), noise, texture, glass and masks.",
                "source": ("crates/render/src/fx.rs", "Raster effects on premultiplied RGBA8 pixmaps"),
            },
            {
                "title": "Strokes",
                "fact": "Dashes, per-side weights, arrowheads and variable width strokes that "
                        "follow a profile along the path.",
                "source": ("crates/pathops/src/width.rs", "Variable width strokes:"),
            },
            {
                "title": "Colour management",
                "fact": "A file is sRGB or Display P3; conversions and the ICC or PNG tag on "
                        "exports follow the file's own profile.",
                "source": ("crates/color/src/profile.rs", "//! Colour profiles:"),
            },
        ],
    },
    {
        "area": "TEXT",
        "title": "Text",
        "role": "A text stack of its own: fonts, shaping, bidi, line layout and glyph outlines.",
        "source": ("crates/text/src/lib.rs", "FigCraft text: fonts, shaping and layout."),
        "leaves": [
            {
                "title": "Editing on the canvas",
                "fact": "Caret, selection and typing in place, mixed styles within a paragraph, "
                        "IME composition; a whole editing session is one undo step.",
                "source": ("crates/engine/src/textedit.rs", "//! On-canvas text editing:"),
            },
            {
                "title": "Shaping and bidi",
                "fact": "HarfRust shaping, the Unicode bidi algorithm for mixed direction text, "
                        "CJK line breaking, variable font axes and glyph outlines.",
                "source": ("crates/text/src/lib.rs", "Fonts come from the optional craft-fonts build input"),
            },
            {
                "title": "Typography",
                "fact": "Family search with previews, style, size, line and letter spacing, "
                        "lists, OpenType features, truncation and text styles.",
                "source": ("crates/ui-egui/src/typography.rs", "//! Typography section of the Design panel:"),
            },
        ],
    },
    {
        "area": "PROTOTYPE",
        "title": "Prototyping",
        "role": "Interactions are document data, played by the same canvas in a player.",
        "source": ("crates/doc/src/proto.rs", "//! Prototype data: interactions"),
        "leaves": [
            {
                "title": "Reactions",
                "fact": "Triggers, actions and transitions, including Smart Animate-style "
                        "animation, overlays and overflow scrolling, stored on the layer they "
                        "start from.",
                "source": ("crates/doc/src/proto.rs", "//! Prototype data: interactions"),
            },
            {
                "title": "Variables and expressions",
                "fact": "Set-variable actions and conditionals whose values are expressions over "
                        "variables, modes and operators — logic without code.",
                "source": ("crates/doc/src/proto_expr.rs", "//! Prototype expressions:"),
            },
            {
                "title": "Presentation",
                "fact": "A full-window player and a preview pane floating over the canvas, with "
                        "device frames and hotspot hints.",
                "source": ("crates/ui-egui/src/present.rs", "//! The prototype player:"),
            },
        ],
    },
    {
        "area": "HANDOFF",
        "title": "Handoff",
        "role": "The same file becomes the spec: measurements, annotations, code, comments, "
                "versions.",
        "source": ("crates/engine/src/devinspect.rs", "//! Dev-mode inspection:"),
        "leaves": [
            {
                "title": "Inspect",
                "fact": "A layer's layout, typography, fills, strokes and effects grouped for "
                        "handoff, each with the style or variable behind it.",
                "source": ("crates/engine/src/devinspect.rs", "//! Dev-mode inspection:"),
            },
            {
                "title": "Code",
                "fact": "CSS, HTML, React, SwiftUI or Compose; components become components with "
                        "parameters and instances become calls passing their values.",
                "source": ("crates/engine/src/codegen.rs", "Code for a layer"),
            },
            {
                "title": "Ready for dev",
                "fact": "Mark frames ready, focus one, and compare property-level changes since "
                        "it was marked, with annotations pinned to layers.",
                "source": ("crates/engine/src/cmd/section.rs", "//! Sections and handoff:"),
            },
            {
                "title": "Comments and versions",
                "fact": "Threads pinned to layers; named versions with restore, and branches "
                        "with review and merge.",
                "source": ("crates/doc/src/comments.rs", "//! Comments: pinned notes with reply threads"),
            },
        ],
    },
    {
        "area": "FILES",
        "title": "Files and formats",
        "role": "The file is a zip you own, and imports and exports meet other tools at their "
                "formats.",
        "source": ("crates/format/src/lib.rs", "The native"),
        "leaves": [
            {
                "title": ".figcraft",
                "fact": "document.json plus images by content hash, a thumbnail, version snapshots "
                        "and branches, saved atomically.",
                "source": ("crates/format/src/lib.rs", "The native"),
            },
            {
                "title": "Import",
                "fact": "The public REST API file JSON with variables and images, .fig files on a "
                        "best-effort path, SVG, images and clipboard content.",
                "source": ("crates/format/src/fig/mod.rs", "//! Import of `.fig` files (best effort)"),
            },
            {
                "title": "Export",
                "fact": "PNG, JPG, WebP, SVG and PDF at any scale, with print options; code "
                        "export lives in Inspect.",
                "source": ("crates/ui-egui/src/export_panel.rs", "//! Exporting:"),
            },
            {
                "title": "Recovery",
                "fact": "Autosave and recovery copies reopen unsaved files after a crash or a "
                        "forced quit, per document.",
                "source": ("crates/ui-egui/src/lib.rs", "opened again from recovery copies"),
            },
        ],
    },
    {
        "area": "MULTIPLAYER",
        "title": "Multiplayer and server",
        "role": "One document, many editors: a sync core with no networking, a hub, and a server "
                "you can host yourself.",
        "source": ("crates/collab/src/lib.rs", "//! Real-time collaboration core for FigCraft"),
        "leaves": [
            {
                "title": "Live sessions",
                "fact": "Hosted by the app or a server: cursors, selections, follow, cursor chat, "
                        "spotlight and per-person undo.",
                "source": ("crates/engine/src/live.rs", "//! Live sessions"),
            },
            {
                "title": "The sync core",
                "fact": "Synced objects and properties, change sets, fractional positions, server "
                        "ordering and client reconciliation — a pure library.",
                "source": ("crates/collab/src/lib.rs", "//! Real-time collaboration core for FigCraft"),
            },
            {
                "title": "Your own server",
                "fact": "Files, edit and view links, persistence as a snapshot plus an append-only "
                        "log, team libraries and a plugin directory.",
                "source": ("apps/figcraft-server/src/lib.rs", "Files persist as a snapshot plus an append-only log"),
            },
            {
                "title": "In the browser",
                "fact": "The same engine and the same UI compiled to WebAssembly, joining the same "
                        "sessions from a tab.",
                "source": ("apps/figcraft-web/src/main.rs", "Runs the same"),
            },
        ],
    },
    {
        "area": "EXTEND",
        "title": "Extend and automate",
        "role": "Every feature is a command, so a menu, a script, a plugin or an agent reaches the "
                "same surface.",
        "source": ("crates/engine/src/cmd/mod.rs", "macro_rules! spec {"),
        "leaves": [
            {
                "title": "400+ commands",
                "fact": "spec! names each id, label, parameter docs and undo flag; menus and "
                        "shortcuts are generated from the catalog, not written twice.",
                "source": ("crates/engine/src/cmd/mod.rs", "macro_rules! spec {"),
            },
            {
                "title": "Agents",
                "fact": "An MCP server over stdio, headless or attached to the running app, plus a "
                        "CLI and a JSON control channel — the same tools either way.",
                "source": ("crates/mcp/src/server.rs", "//! JSON-RPC 2.0 framing"),
            },
            {
                "title": "Plugins",
                "fact": "Sandboxed WebAssembly behind a manifest's permissions, metered, with no "
                        "file system or network, one undo step per run.",
                "source": ("crates/plugin/src/lib.rs", "//! FigCraft plugins:"),
            },
        ],
    },
]

# One command's path through the code. This is the "code wise, high level" page:
# the crates are the answer to where, not the axis of the map.

ANATOMY = {
    "title": "How a feature is built",
    "lede": "One path from a panel to a pixel. A feature adds a command; the command edits the "
            "document; layout and text give it geometry; the renderer gives it pixels; and the "
            "file or the wire carries it. Nothing takes a side door.",
    "stages": [
        {
            "number": "01",
            "name": "Panel",
            "detail": "The egui frontend reads engine state and acts through app.run(id, params). "
                      "It draws the shell, not the document.",
            "source": ("crates/ui-egui/src/lib.rs", "pub fn run(&mut self, id: &str"),
        },
        {
            "number": "02",
            "name": "Command",
            "detail": "spec! declares the id, label, parameter documentation, undo flag and run "
                      "function; the registry is collected once.",
            "source": ("crates/engine/src/cmd/mod.rs", "macro_rules! spec {"),
        },
        {
            "number": "03",
            "name": "Document",
            "detail": "The edit lands in one persistent node tree. Undo is a snapshot, and only "
                      "what changed is revisited.",
            "source": ("crates/doc/src/lib.rs", "nodes: imbl::OrdMap<NodeId, Arc<Node>>,"),
        },
        {
            "number": "04",
            "name": "Layout and render",
            "detail": "Layout and text turn the tree into geometry, and tiny-skia turns that into "
                      "pixels for the canvas and every export.",
            "source": ("crates/render/src/lib.rs", "Everything is CPU (tiny-skia)"),
        },
        {
            "number": "05",
            "name": "File and wire",
            "detail": "The same tree becomes a zip on disk and change sets over a socket, so a "
                      "file and a live session are the same document.",
            "source": ("crates/format/src/lib.rs", "A zip archive holding"),
        },
    ],
    "surfaces": {
        "title": "Same command, every surface",
        "body": "The menu, a shortcut, the Actions palette, the CLI, the MCP server, the control "
                "channel and a WebAssembly plugin all dispatch the same command ids. That is why "
                "an agent can do anything a person can — and why a new feature is reachable "
                "everywhere at once, or nowhere.",
        "chips": "MENU   SHORTCUT   ACTIONS   CLI   MCP   CONTROL   PLUGIN",
        "source": ("crates/engine/src/catalog.rs", "pub static MENU: &[MenuItem] = &["),
        "note": "A feature area is one cmd module, one panel, and — when it changes pixels — one "
                "layout or render path.",
    },
}
