# service — the Mindmap node (Rust)

This crate builds one binary, `mindmap-node`: the per-machine backend that hosts live worktree
designs, drives the machine's shared Herdr and runs Git-backed Sync. How it works, its protocol,
deployment and migration are in [`NODE.md`](NODE.md).

| File | What it does |
|---|---|
| `src/node_main.rs` | CLI (`serve`, `request`, `mcp`, `migrate`) and the stdio MCP adapter for agents. |
| `src/node.rs` | Versioned, multiplexed JSON transport on `127.0.0.1:47822`; every method dispatches here. |
| `src/node_context.rs` | The context packet: page, subpages, top-level cards and commented cards. |
| `src/node_documents.rs` | One live session per worktree and branch; debounced atomic saves; renders. |
| `src/node_git.rs` | `Mindmap-Sync` baselines, frozen Sync inputs, design merges. |
| `src/node_herdr.rs` (+ `node_herdr_events.rs`) | The shared Herdr: inventory, agents, install, socket events. |
| `src/node_migration.rs` (+ `mcp.rs`) | One-time import of documents from the old document server. |
| `src/agent.rs`, `src/comments.rs` | Find or start an agent and submit a prompt; comment bundles. |
| `src/herdr.rs`, `src/workspace.rs` | Typed `herdr` CLI wrapper; the workspace shapes sent to the client. |
| `src/config.rs`, `src/error.rs` | Tool paths and errors. |

The previous `mindmap-service` binary (the TCP API on `:47821`, `projects.json` records, edit
links) was removed; it is in Git history before the node cleanup commit.
