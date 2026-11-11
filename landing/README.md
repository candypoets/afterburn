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

    **The download page now links to GitHub Releases**, not to these files: the app builds are
    produced by `.github/workflows/release.yml` and attached to
    <https://github.com/candypoets/afterburn/releases>. The page uses stable asset URLs, so a new
    release needs no site change:

    | Link                      | GitHub asset (latest release)                       |
    |---------------------------|-----------------------------------------------------|
    | DOWNLOAD .DMG             | `afterburn-macos-arm64.dmg`                         |
    | INTEL .DMG                | `afterburn-macos-x64.dmg`                           |
    | DOWNLOAD .ZIP             | `afterburn-windows-x64.zip`                         |
    | DOWNLOAD .APPIMAGE        | `afterburn-linux-x64.AppImage`                      |
    | .TAR.GZ                   | `afterburn-linux-x64.tar.gz`                        |
    | CHECKSUMS · SHA-256       | `SHA256SUMS`                                        |
    | RELEASE NOTES             | <https://github.com/candypoets/afterburn/releases/latest> |
    | ALL RELEASES              | <https://github.com/candypoets/afterburn/releases>  |

    Under `files/download/` only the headless node remains (next bullet); an asset missing from the
    latest release is a GitHub 404, since nginx no longer fronts those files.
  - `files/download/node/`: the headless node, in the layout `service/build-node.sh` produces —
    one directory per platform (`darwin-arm64`, `darwin-x64`, `linux-x64`, `linux-arm64`) with
    `afterburn-node` and `build-info.json`, plus `install-node.sh` and `install-node.py` beside
    them. `/node.sh` picks this machine's directory, checks the published SHA-256 and runs the
    installer with `--scope user` (`system` as root on Linux). `AFTERBURN_NODE_BASE` points it at
    another base (for example `file:///tmp/node` when testing); `AFTERBURN_NODE_UPGRADE=1`
    replaces a supervised, idle node.
- The poster is a real screenshot of the app (the Brand & design language board).
- Fonts: JetBrains Mono (JetBrains, Apache-2.0/OFL), subset to woff2. Icons: Phosphor (MIT).
