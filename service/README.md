# service — the Afterburn node (Rust)

This crate builds one binary, `afterburn-node`: the per-machine backend that hosts live worktree
designs, drives the machine's shared Herdr and runs Git-backed Sync. How it works, its protocol,
deployment and migration are in [`NODE.md`](NODE.md).

## Build and install

This is package **`afterburn-node`** in the root workspace. Its FigCraft dependencies come from
`server/figcraft/`, the same source used by the desktop app; there is one root lockfile.

```sh
# From the repository root (rust-toolchain.toml selects 1.97.1):
cargo build -p afterburn-node
./target/debug/afterburn-node serve --bind 127.0.0.1:47822
# Package a release node with build provenance and a portable installer:
./service/build-node.sh
```

The headless package does not build the desktop or require a display server. The portable
`install-node.sh`/`install-node.py` installs immutable versioned binaries and registers launchd
or systemd, with a manual option for hosts without a supervisor. It never invokes sudo.
See [installation commands and upgrade safeguards](NODE.md#deployment-and-migration).
`systemd/afterburn-node.service` is a root-checkout example using `target/release`; the portable
installer generates units pointing to the installed version instead.

| File | What it does |
|---|---|
| `src/node_main.rs` | CLI (`serve`, `request`, `mcp`, `migrate`, `build-info`) and the stdio MCP adapter for agents. |
| `src/node.rs` | Versioned, multiplexed JSON transport on `127.0.0.1:47822`; every method dispatches here. |
| `src/node_context.rs` | The context packet: page, subpages, top-level cards and commented cards. |
| `src/node_documents.rs` | One live session per worktree and branch; debounced atomic saves; renders. |
| `src/node_git.rs` | `Afterburn-Sync` baselines, frozen Sync inputs, design merges. |
| `src/node_herdr.rs` (+ `node_herdr_events.rs`) | The shared Herdr: inventory, agents, install, socket events. |
| `src/node_migration.rs` (+ `mcp.rs`) | One-time import of documents from the old document server. |
| `src/agent.rs`, `src/comments.rs` | Find or start an agent and submit a prompt; comment bundles. |
| `src/herdr.rs`, `src/workspace.rs` | Typed `herdr` CLI wrapper; the workspace shapes sent to the client. |
| `src/config.rs`, `src/error.rs` | Tool paths and errors. |

The previous `afterburn-service` binary (the TCP API on `:47821`, `projects.json` records, edit
links) was removed; it is in Git history before the node cleanup commit.
