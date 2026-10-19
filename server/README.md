# server — our FigCraft server fork

The document plane, forked from FigCraft **0.7.0** (upstream `storytold/figcraft`, commit
`99e5db3`, 2026-10-18) so we can change the server's design instead of running it as a black box.

It is the **whole upstream tree**, not just `apps/figcraft-server`, because the protocol, store and
hub live in `crates/collab-net` (which depends on `crates/engine`). Forking only the app would let
us rename a binary but not change how sessions, files and auth work.

```
figcraft/   the fork (23 MB of source; target/ and .git are not kept)
data/       files, links and the server key (git-ignored)
run.sh      start it on 0.0.0.0:47910 with data/
```

## Build

Linux, Rust 1.95 (`rustup` has it; the host default is older):

```sh
cd figcraft
cargo +1.95.0 build --release -p figcraft-server -p figcraft-cli
```

The CLI is built from the same tree so smoke tests speak the same wire version as the server.

## Run

```sh
./run.sh                 # 0.0.0.0:47910, data/
LISTEN=127.0.0.1:47910 ./run.sh
```

The key and the address to publish to:

```sh
figcraft/target/release/figcraft-server key --data data
```

It prints `ws://<host>:47910/?key=<key>`. The Mac app and any Mac build connect to the Tailscale
address (`ws://100.104.9.15:47910/?key=…`) or the LAN address; TLS comes later, behind the
gateway.

## It runs beside the old server, not instead of it

The team's current server (FigCraft **0.5.1**, `figcraft.eisledger.com`, files for Eisledger and
Nuts) keeps running untouched on its Docker stack. This fork has its **own port and its own data
directory**, so nothing migrates until we decide to cut over. A 0.7.0 client cannot talk to the
0.5.1 server and vice versa; match the version on both sides.

## What we change here next

- Projects: a project record binds a repo/worktree, an agent and this server's documents.
- Tokens: the app mints per-job edit tokens; humans never copy a link.
- Jobs + MCP: an endpoint the runner and remote agents use, so "open project → open this
  document" is one call.
- Then, and only then, a cut-over plan for the existing files.

Upstream license and notices are kept in `figcraft/LICENSE-*`, `figcraft/NOTICE` and
`figcraft/ATTRIBUTION.md`.
