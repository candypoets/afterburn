# Afterburn landing page

The pages built from the map's **Landing page** design — hero v2 and the download page — desktop
1440, mobile 390. Served at <https://afterburn.nuts.cash> by its own nginx container behind the
host's Traefik (`web` entrypoint, network `traefik-net`; TLS ends in front of Traefik).

```sh
docker compose -f landing/compose.yaml up -d --build
```

- `site/`: static HTML, CSS and the scripts; no build step.
  - `/` — the hero (`site/index.html`, `site/styles.css`, `site/player.js`).
  - `/download/` — the download page (`site/download/index.html`, `site/download.css`,
    `site/download.js`). `/download` redirects to it.
  - `/node.sh` — the headless-node installer (`site/node.sh`), served as `text/x-shellscript`.
- `files/` (not committed) is mounted read-only at `/srv/files`:
  - `files/media/demo.mp4` (and `demo.en.vtt`): the demo video. Until it is there, Play says
    "Demo lands soon".
  - `files/download/…`: the builds the download page links to. Drop them in; no rebuild needed.

    | Link                      | File                                  |
    |---------------------------|---------------------------------------|
    | DOWNLOAD .DMG             | `afterburn-macos.dmg`                 |
    | DOWNLOAD .MSI             | `afterburn-windows-x64.msi`           |
    | ARM64 .ZIP                | `afterburn-windows-arm64.zip`         |
    | DOWNLOAD .APPIMAGE        | `afterburn-linux-x86_64.AppImage`     |
    | .DEB                      | `afterburn-linux-amd64.deb`           |
    | .TAR.GZ · ARM64           | `afterburn-linux-arm64.tar.gz`        |
    | CHECKSUMS · SHA-256       | `SHA256SUMS`                          |
    | RELEASE NOTES             | `release-notes.html`                  |
    | ALL RELEASES              | `releases/` (older builds)            |

    A link to a file that is not there yet shows the "on their way" page (`site/soon.html`).
  - `files/download/node/`: the headless node, in the layout `service/build-node.sh` produces —
    one directory per platform (`darwin-arm64`, `darwin-x64`, `linux-x64`, `linux-arm64`) with
    `afterburn-node` and `build-info.json`, plus `install-node.sh` and `install-node.py` beside
    them. `/node.sh` picks this machine's directory, checks the published SHA-256 and runs the
    installer with `--scope user` (`system` as root on Linux). `AFTERBURN_NODE_BASE` points it at
    another base (for example `file:///tmp/node` when testing); `AFTERBURN_NODE_UPGRADE=1`
    replaces a supervised, idle node.
- The poster is a real screenshot of the app (the Brand & design language board).
- Fonts: JetBrains Mono (JetBrains, Apache-2.0/OFL), subset to woff2. Icons: Phosphor (MIT).
