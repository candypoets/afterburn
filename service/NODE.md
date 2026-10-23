# Mindmap node

`mindmap-node` is the backend on each machine. It hosts live worktree designs and drives that
machine's shared Herdr server. The app uses one persistent connection per node, through loopback
or an SSH tunnel. The Docker document server is the only legacy migration source; the old
`mindmap-service` binary has been removed (it is in Git history before the node cleanup).

```sh
CARGO_HOME="$PWD/.cargo-home" cargo +1.95.0 build --release \
  --manifest-path service/Cargo.toml --bin mindmap-node
service/target/release/mindmap-node serve --bind 127.0.0.1:47822
```

The listener refuses non-loopback addresses. SSH authenticates remote access. Protocol 1 carries
newline JSON requests with `id`, `version: 1`, `method`, and method parameters at the top level.
Replies contain the same ID, `version`, `ok`, and a result or `error`. Messages are capped at
16 MiB. Server events have `event` instead of `id`. Slow agent startup runs independently of
collaboration dispatch so edits continue while a job is starting.

## Worktrees and live documents

`open {path}` or `open {workspace}` returns `session`, canonical `path`, `branch`, `workspace`,
`label` and `name`. No server key, owner link or project record is involved. A session belongs to
a canonical worktree and Git branch. On checkout, the app automatically opens the new branch's
session on the same selected node after pending workspace activity finishes. CLI callers use
`open` again; the MCP adapter resolves its worktree on every call. Linked worktrees remain
independent. Files load from `.mindmap/map.figcraft` or start with a blank page.

FigCraft's collaboration authority orders and validates property edits. `session.join` accepts
a FigCraft `Hello` and returns `{peer,messages}` with its welcome. `session.send {message}` carries
the existing `ClientMsg`; `collab` events carry `ServerMsg`. Presence uses the same connection.
`session.leave` or a disconnected socket removes the peer. Engine commands execute through
`session.command {command,params}`; the node prevents file/network/plugin commands from bypassing
its ownership, including inside batches. `session.inspect` and `session.commands` expose reads.

Only the node saves the design: atomic, debounced writes plus explicit `session.save`. Git-local
cache preserves authority state and edits across restarts. File changes are three-way merged
through the live session. Conflicts remain explicit until `session.resolve {picks}` chooses
each key. The node never silently overwrites a changed file. `session.import {source}` is an
explicit migration/import operation; it applies the imported document through the authority.

Agents use `mindmap-node mcp --path WORKTREE`, or `mindmap-node request '<JSON>'`. The MCP adapter
answers in the same shape as FigCraft's own MCP: `inspect_document` and `get_node` return the node
itself, `create_node` (`node.create`), `set_properties` (`node.set`) and `run_command` return the
command result, `list_commands` discovers commands, and `node_request` reaches any node method.
`screenshot {ids?, page?, scale?}` returns a PNG through `session.render`, which exports a copy of
the live design: nothing is written and the session is not edited. Document commands that write
files (`*.export`, `*.import`) stay refused.

The context packet sent with comments, new pages and Sync is a short summary, never raw JSON: the
page and its subpages (from `page` or the client's `page_id`, else the page of the first commented
card), the page's top-level cards, and each commented card with its id, type, path and text,
capped at 8,000 characters by whole lines (`src/node_context.rs`).

## Herdr and host setup

`host.status` reports `installed`, `running`, `approval_required`, `binary`, `socket`, `session`.
`host.ensure` starts an installed server headlessly. If Herdr is missing it reports that approval
is needed; only `host.ensure {install_herdr:true}` runs the official installer. The node uses the
shared `HERDR_SOCKET_PATH`/`HERDR_SESSION`, or the default user session. It never creates a private
Herdr or stops the shared daemon when the node exits.

`workspaces` returns the live Herdr inventory, installed CLIs, agent details and worktree paths.
`init` and `send` build context from the node's own document and prompt the chosen agent using
the existing agent lifecycle adapter. `worktree.create` uses explicit workspace/cwd context.
The node subscribes to Herdr's socket events, resynchronizes after reconnect, and publishes
agent/inventory updates. Reading the Mac's saved remote machine shows those same agents.

The Mac app starts its bundled local node as needed. Adding an SSH host detects its platform,
copies the matching bundled node, starts it, forwards one loopback port, and registers the shared
Herdr session in the Mac's saved machines. Binaries live under
`Mindmap.app/Contents/Resources/node/{darwin-arm64,linux-x64}/mindmap-node` for this deployment.
Other platforms require a corresponding native build. Herdr installation approval is shown by
the app only when needed.

## Git-backed Sync

`diff {session,page?,offset?,limit?}` loads the last reachable matching `Mindmap-Sync:` trailer
commit and compares its native design with the authoritative live document using FigCraft's
property diff. It includes the existing UI's added/changed/removed summaries, full property
changes, and code drift. Pagination reports complete totals and next offsets. A page-specific
marker covers only that page; an `all` marker covers the whole document.

`sync` validates the target agent, freezes the design under the worktree's Git directory,
persists the job, and prompts the agent with the baseline, session and scope. `sync.capture`
performs the same freeze for an already-running external agent without submitting another prompt.
The agent reads full drift from the node and Git, plans and implements the task.

At completion, `sync.prepare {job}` gates autosave, writes the exact frozen design and stages its
blob. The agent stages its code and commits both with the returned exact trailer:

```text
Mindmap-Sync: page=219043332230 job=JOB_ID
```

`sync.finish {job,commit}` verifies branch, ancestry, exact design blob and trailer before
reporting done. Agent idle alone leaves the job awaiting its verified commit. The node then
restores the latest live design to disk, so edits made during implementation remain drift.
`sync_status` also discovers a verified commit. `sync.cancel` releases the gate without declaring
success. Jobs survive node restart in the worktree's Git directory; no sync state is stored in
`projects.json`.

`merge.plan {session,incoming}` obtains Git's merge base and compares native designs per property.
`merge.apply` accepts explicit conflict picks. It incorporates the resolved design through the
session; code conflicts must be resolved before a Git merge can be committed. No binary/text Git
merge driver is installed. A merge does not invent a new Sync acknowledgement; reachable parent
markers remain the baselines.

## Deployment and migration

Install `service/systemd/mindmap-node.service` on the Linux host. Install the matching node binaries
with the Mac client; `service/install-mac-node.sh` registers the native node with launchd.
`mindmap-node migrate --records service/data/projects.json` exports and verifies legacy documents
through the running server and node, preserving existing repository designs and never logging
owner links. Keep the existing document server online while exporting its live designs,
then import each into its repository through the node and verify pages and IDs. Preserve server
data for rollback. Only after verification stop the mindmap Docker container. `server/compose.yaml` retains it behind the explicit `legacy` profile; the separate
older FigCraft installation is unaffected.

## Verification

```sh
CARGO_HOME="$PWD/.cargo-home" cargo +1.95.0 test --manifest-path service/Cargo.toml
CARGO_HOME="$PWD/.cargo-home" cargo +1.95.0 clippy --manifest-path service/Cargo.toml \
  --all-targets -- -D warnings
```

The service suite covers ordered concurrent clients, disconnect cleanup, atomic saves and
restart recovery, complete property drift with pagination, page-scoped Git baselines,
frozen Sync commits with later live edits preserved, and explicit design-merge conflicts.
Herdr protocol fixtures cover event subscription and reconnect; the opt-in live test reads
the shared server without prompting agents. Client checks are documented in
[`client/README.md`](../client/README.md).

The backend cutover was verified with native Linux and macOS nodes, the rebuilt Mac app
opening this repository through one SSH tunnel, both machines' existing shared Herdr
sessions, and the agent MCP reading the backend page. The Mindmap document container is
stopped; its migration data is retained for rollback.
