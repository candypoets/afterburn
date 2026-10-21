# client — Mindmap.app

The Mac client, built from a FigCraft 0.7.0 checkout: the same engine and canvas, with the editor
taken off the human's screen. It is named **Mindmap**, has its own icon, and opens in **map mode**.

| File | What it is |
|---|---|
| `mindmap-client.patch` | The whole change against a 0.7.0 checkout (4 files, ~30 lines). |
| `icon-1024.png` | The app icon, drawn for this project (not FigCraft's, not Figma's). |
| `build-mac.sh` | Build, wrap in `Mindmap.app`, sign ad hoc, optionally launch with a join link. |

## Apply and build

```sh
git clone https://github.com/storytold/figcraft ~/code/figcraft
cd ~/code/figcraft && git checkout -b mindmap-client   # patch is against 0.7.0, main a0d1162
git apply ~/code/mindmap/client/mindmap-client.patch
~/code/mindmap/client/build-mac.sh ~/code/figcraft
```

Launch with a document:

```sh
open -a ~/Applications/Mindmap.app --args --map-mode --control 49125 \
  "figcraft://join?host=100.104.9.15:47910&file=…&token=…"
```

## What the patch does

- Adds **map mode** (`--map-mode`, or `MINDMAP_CLIENT=1`): the left rail and layers/Assets panels,
  the right Design/Prototype/Inspect panels, the bottom toolbar, the tab bar and the frame labels
  are not drawn. The canvas, selection, zoom, prototype links and live session stay.
- Adds a **map bar** at the top: one tab per page (click to `page.select`, accent underline on the
  active page) and a live count (`N live` while joined, `offline` otherwise, counting you).
- **Refuses edits.** Every command whose spec is a document edit is refused in map mode
  (`FigApp::run` checks `CommandSpec::history`), except `comment.*`; `tool.*` and `text.edit` are
  refused too. Selection, navigation, zoom, joins and comments still run. Hiding the tools was
  not enough; this is the permission model.
- **Keeps the bundle icon.** eframe calls `NSApplication.setApplicationIconImage` from the embedded
  FigCraft icon when `with_icon` is set, which is why the Dock showed FigCraft even with a new
  bundle. Map mode now skips `with_icon`, so macOS uses `AppIcon.icns` from the bundle.
- Renames the app: window title, run name, app id (`dev.mindmap.client`), the home wordmark and the
  version line. The bundle carries `CFBundleName`/`CFBundleDisplayName` **Mindmap** and `AppIcon`.
- Leaves the engine untouched: this is a UI gate, not a permission model. Editing shortcuts still
  exist, and the document is still writable — the next step is to refuse mutating commands in map
  mode rather than only hiding their entry points.

## Known gaps (the next slice)

- **Opening a project.** The client now opens on the Herdr workspace list (map mode) and opens or
  initialises a workspace through the Rust service; see `project/README.md`.
- **The map itself.** Init starts and prompts the agent, but the model renderer (`mindmap render`)
  is still missing, so the agent cannot draw the overview yet.
- **Comments surface.** The guard allows comments, but the panel that creates and reads a thread
  is still hidden with the old chrome.
- **In-canvas links.** Cards show "Open →" in the design, but clicking one still selects it rather
  than moving the canvas. The next step is: a click on a card with a navigate interaction selects
  its destination page and zooms to the target — no prototype player.
- **Auto-join.** A join link passed at launch opens the Join dialog prefilled; the control channel
  (`collab.join`) completes it. It should join straight away.
- **Ask bar.** The composer in the "Screens" design page is a drawing, not wired yet.
- **Comments.** Pinning and reading threads should survive in map mode; the Comments panel was in
  the hidden chrome.
- **Read-only.** Hiding the tools is not the same as refusing edits; the server's view link or an
  engine-level guard is the honest enforcement.

The installed FigCraft.app (0.5.1) is untouched; this is a separate app bundle. The server it talks
to is our 0.7.0 fork (`server/`), and the design it opens is "Mindmap · system".
