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

## Agents: a private Herdr the user never sees

The server runs **one private Herdr daemon as its child**. Users install and start only our
server; Herdr is an implementation detail of the agent host, not a second thing to run.

`private-herdr.sh` is the working recipe (proven 2026-10-19 against Herdr 0.9.0 on this host):

- **`herdr server`** — the headless mode, made for supervised/service-style setups.
- **a named session** (`HERDR_SESSION=mindmap`) — its own socket and workspace list.
- **private `XDG_CONFIG_HOME`/`XDG_STATE_HOME`** — sockets, sessions and state live under
  `data/agents/`, never in `~/.config/herdr`.
- **every inherited `HERDR_*` variable unset** — this is the one that bit us: `HERDR_SOCKET_PATH`
  overrides the session and points at the user's daemon. Without `-u HERDR_SOCKET_PATH`, starting
  the private server fails with "already running" and every command talks to the wrong instance.
- **stop only through the script** (`./private-herdr.sh cli server stop`), which carries the
  isolation; a bare `herdr server stop` stops the user's daemon.

Verified on this host while a user Herdr session was live: the private instance started with its
own socket, created a workspace and ran a command in a pane, then stopped — the user's daemon kept
running with its 7 workspaces before and after.

What our server maps onto it:

| Ours | Herdr |
|---|---|
| project | workspace (`--cwd` the repo) |
| job | pane + agent prompt (`agent start`, `agent prompt`) |
| parallel design | worktree (`herdr worktree create`) |
| job status | lifecycle: `working`, `blocked`, `idle`, `done` |

Packaging: ship the pinned `herdr` 0.9.0 binary with our server (Apache-2.0; keep its `LICENSE`
and `NOTICE`), so no user ever installs it. Agents run where our server runs — on the Mac for a
local repo, on the host for a hosted one. The current container has no repos or agent CLIs inside,
so agent hosting wants a host install (systemd) or workspace mounts; the document plane can stay
containerised either way.

Next in the fork: a Rust supervisor (spawn, health, restart, stop on shutdown) and the job API in
front of it.

## Deployment modes

**Host service (the target): one binary, one service.** The document plane and the agent host
are the same process: it serves files, runs the private Herdr child, and spawns agents in the
repos on that machine. The unit is `systemd/mindmap-server.service`; install it once:

```sh
cp systemd/mindmap-server.service /etc/systemd/system/
systemctl daemon-reload
systemctl enable --now mindmap-server
```

The running sandbox here cannot reach systemd (`Failed to connect to bus`), so this step is
yours to run; the unit is committed ready.

**Container (current dev server): document plane only.** It serves files and live sessions and
has no repos, agent CLIs or shells to host agents in. That is what the compose file does today.
Mounting the host's repos, Codex/Claude configs, Node/Volta, git and Herdr into it would make a
second, fatter environment to maintain, which is why agent hosting belongs on the host service.

Either way the document plane is the same server; only the ability to spawn agents differs.

## What we change here next

- Projects: a project record binds a repo/worktree, an agent and this server's documents.
- Tokens: the app mints per-job edit tokens; humans never copy a link.
- Jobs + MCP: an endpoint the runner and remote agents use, so "open project → open this
  document" is one call.
- Then, and only then, a cut-over plan for the existing files.

Upstream license and notices are kept in `figcraft/LICENSE-*`, `figcraft/NOTICE` and
`figcraft/ATTRIBUTION.md`.
