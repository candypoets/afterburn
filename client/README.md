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
cd ~/code/figcraft && git checkout -b mindmap-client
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
- Renames the app: window title, run name, app id (`dev.mindmap.client`), the home wordmark and the
  version line. The bundle carries `CFBundleName`/`CFBundleDisplayName` **Mindmap** and `AppIcon`.
- Leaves the engine untouched: this is a UI gate, not a permission model. Editing shortcuts still
  exist, and the document is still writable — the next step is to refuse mutating commands in map
  mode rather than only hiding their entry points.

## Known gaps (the next slice)

- **Page navigation.** With tabs and the layers panel hidden there is no way to move between pages;
  today the control channel does it (`page.select`). The map needs in-canvas links (the detail
  screen's prototype link is the pattern) or a minimal page switcher.
- **Ask bar.** The composer in the "Screens" design page is a drawing, not wired yet.
- **Comments.** Pinning and reading threads should survive in map mode; the Comments panel was in
  the hidden chrome.
- **Read-only.** Hiding the tools is not the same as refusing edits; the server's view link or an
  engine-level guard is the honest enforcement.

The installed FigCraft.app (0.5.1) is untouched; this is a separate app bundle. The server it talks
to is our 0.7.0 fork (`server/`), and the design it opens is "Mindmap · system".
