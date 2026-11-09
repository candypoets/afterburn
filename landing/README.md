# Afterburn landing page

The page built from the map's **Landing page** design (hero v2: desktop 1440, mobile 390), served
at <https://afterburn.nuts.cash> by its own nginx container behind the host's Traefik (`web`
entrypoint, network `traefik-net`; TLS ends in front of Traefik).

```sh
docker compose -f landing/compose.yaml up -d --build
```

- `site/`: static HTML, CSS and the player script; no build step.
- `files/` (not committed) is mounted read-only at `/srv/files`:
  - `files/media/demo.mp4` (and `demo.en.vtt`): the demo video. Until it is there, Play says
    "Demo lands soon".
  - `files/download/…`: builds. `/download/` serves `files/download/index.html` when present,
    else the "on their way" page.
- The poster is a real screenshot of the app (the Brand & design language board).
- Fonts: JetBrains Mono (JetBrains, Apache-2.0/OFL), subset to woff2. Icons: Phosphor (MIT).
