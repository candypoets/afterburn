# Afterburn node

`afterburn-node` is the backend on each machine. It hosts live worktree designs and drives that
machine's shared Herdr server. The app uses one persistent connection per node, through loopback
or an SSH tunnel. The Docker document server is the only legacy migration source; the old
`afterburn-service` binary has been removed (it is in Git history before the node cleanup).

```sh
cargo build -p afterburn-node --release --locked
./target/release/afterburn-node serve --bind 127.0.0.1:47822
```

Run these commands at the repository root. `rust-toolchain.toml` pins Rust 1.97.1.
Selecting `afterburn-node` builds only the backend and its engine dependencies, without the
desktop GUI or a display server. The root workspace also contains the app (`figcraft`); both
share `server/figcraft/`, `Cargo.lock`, and `target/`.

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
independent. Files load from `.afterburn/map.figcraft` or start with a blank page.

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

Agents use `afterburn-node mcp --path WORKTREE`, or `afterburn-node request '<JSON>'`. The MCP adapter
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

The app's explicit host setup invokes its bundled portable installer. A full Mac install ships
its native node from the same Cargo invocation as the app; optional remote platform nodes must
carry matching build provenance. Launchd owns the Mac helper; systemd owns Linux installations.
The app does not spawn a competing helper during ordinary reconnect. A build mismatch requires
an explicit approved update, and a missing companion platform must be built natively or installed
separately. Herdr installation approval is shown only when needed.

`afterburn-node build-info` prints `{build_id,version,protocol}`. The source SHA256 is calculated by
the shared `afterburn-build` crate, consistently across native platforms. `hello` reports the same
identity and the running PID. Binary sidecars also record platform and executable SHA256, so
packaging can validate a foreign companion without executing it.

## Git-backed Sync

`diff {session,page?,offset?,limit?}` loads the last reachable matching `Afterburn-Sync:` trailer
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
Afterburn-Sync: page=219043332230 job=JOB_ID
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

The portable installer requires Python 3.9+ and copies a native binary to an immutable versioned
directory. A CLI symlink points to that copy. The app and headless installers use the same entry
point, `service/install-node.sh`, with `install-node.py` beside it. Neither needs a GUI on Linux.

```sh
# Linux: build the native headless package, without building the app.
./service/build-node.sh
NODE="$PWD/target/release/afterburn-node"
BUILD_ID="$("$NODE" build-info | python3 -c 'import json,sys; print(json.load(sys.stdin)["build_id"])')"

# Normal Linux user: requires a running systemd user manager.
./service/install-node.sh --binary "$NODE" --build-id "$BUILD_ID" --scope user
# A root operator can instead choose a system service (the script never invokes sudo).
./service/install-node.sh --binary "$NODE" --build-id "$BUILD_ID" --scope system
```

Choose **one** scope per machine. User scope installs under `~/.local/lib/afterburn/node/<build-id>`
and `~/.local/bin`, with a unit in `~/.config/systemd/user`. It starts on user-manager startup;
persistence after logout/before login requires administrator-configured lingering. System scope
requires root, uses `/usr/local/lib/afterburn/node/<build-id>` and `/usr/local/bin`, and installs
`/etc/systemd/system/afterburn-node.service`. That service runs as the invoking root operator.
For a non-root service account use its user manager. No privilege escalation or lingering changes
are performed automatically. The supplied `systemd/afterburn-node.service` is a separate example
for an existing `/root/code/mindmap` checkout, with root `target/release` paths.
An older `~/.local/bin/afterburn-node` can shadow a system-scope `/usr/local/bin/afterburn-node`.
Check `command -v afterburn-node` after migration; the installer does not overwrite a different
scope's CLI choice.

On macOS the same `--scope user` command registers `dev.afterburn.node` in the logged-in user's
launchd GUI domain. Its plist lives in `~/Library/LaunchAgents`; logs are in
`~/Library/Logs/Afterburn`. The binary lives under `~/.local/lib/afterburn/node/<build-id>`, outside
any `.app`. This survives app quit and app replacement, and starts at login. A headless SSH-only
Mac without a logged-in GUI domain cannot use this LaunchAgent automatically.

```sh
# Explicit fallback on a host without a usable supervisor:
./service/install-node.sh --binary "$NODE" --build-id "$BUILD_ID" --scope manual
"$HOME/.local/bin/afterburn-node" serve --bind 127.0.0.1:47822
```

Manual mode installs only the versioned binary and CLI symlink. It does not start a background
process, survive logout, or register reboot persistence; the operator supplies its lifecycle.
It refuses to replace a running node.

The package in `build/node/` contains native platform directories and both installer files. Copy
those together over SSH, then invoke the same installer against the host's platform binary.
Platform names are `linux-x64`, `linux-arm64`, `darwin-arm64`, and `darwin-x64`; only the platform
actually built is present. Linux executable compatibility depends on the build host's libc and
runtime libraries. No automatic cross compilation or Windows service installation is provided.

### Approved updates and lifecycle

Pass `--upgrade` (alias `--replace`) only after approving a restart. A different running build
without this flag exits 20. Uninspectable sessions, prepared Syncs or ready merges exit 21;
an unmanaged/legacy listener or supervisor PID mismatch exits 22. Other failures exit 1.
Successful installation prints JSON with `status`, `build_id`, and installed `binary`.

The installer verifies native `build-info`, PID ownership and `session.list`, then takes an atomic
`upgrade.prepare` lease immediately before stopping the node. The 30-second lease rejects
`sync.prepare`/`merge.apply` while edits continue; `upgrade.cancel {lease}` releases it if the old
process survives a failed install. It waits for the expected node to answer before reporting
success. Existing immutable binaries are retained, and failed starts restore the old supervisor
configuration when possible. An install lock prevents concurrent installers within one scope.
Old nodes lacking build identity/PID/upgrade lease support require an operator to verify no
prepared work and stop their existing service explicitly before migration. The installer never
kills an unmanaged listener or disables an unrelated service.

Systemd units use `KillMode=process`, and launchd uses `AbandonProcessGroup`, so node replacement
does not terminate shared Herdr/agent processes. The installer retains existing Herdr socket and
session settings, with explicit `HERDR_SESSION`, `HERDR_SOCKET_PATH`, or `HERDR_BIN_PATH` environment
overrides. The default remains the user's existing shared Herdr session.

The Mac app's **Check for Updates…** (`app.checkForUpdates`, also checked at launch and daily)
installs prebuilt releases this way, never from source. It reads a JSON release feed from
`AFTERBURN_UPDATE_FEED` (no built-in feed exists yet, so the check ends "Up to date" with a note):

```json
{"version":"0.8.0","notes":["…"],
 "platforms":{"darwin-arm64":{
   "app":{"url":"https://…/Afterburn.zip","sha256":"<hex>"},
   "node":{"url":"https://…/afterburn-node","sha256":"<hex>","build_id":"<BUILD_ID>"}}}}
```

It downloads both with `curl`, checks the SHA-256s and `codesign --verify --deep --strict`, refuses
while `session.list` reports a prepared Sync or ready merge, runs the new bundle's
`install-node.sh --binary … --build-id … --upgrade` (exit 21 is the same wait), swaps the `.app`
and relaunches. See `server/figcraft/crates/ui-egui/src/update.rs`.

### Legacy document migration

`afterburn-node migrate --records service/data/projects.json` exports and verifies legacy documents
through the running server and node, preserving existing repository designs and never logging
owner links. Keep the existing document server online while exporting its live designs,
then import each into its repository through the node and verify pages and IDs. Preserve server
data for rollback. Only after verification stop the afterburn Docker container. `server/compose.yaml` retains it behind the explicit `legacy` profile; the separate
older FigCraft installation is unaffected.

## Verification

```sh
cargo test -p afterburn-node
cargo clippy -p afterburn-node --all-targets -- -D warnings
python3 -m unittest discover -s service -p 'test_install_node.py' -v
```

The service suite covers ordered concurrent clients, disconnect cleanup, atomic saves and
restart recovery, complete property drift with pagination, page-scoped Git baselines,
frozen Sync commits with later live edits preserved, and explicit design-merge conflicts.
Herdr protocol fixtures cover event subscription and reconnect; the opt-in live test reads
the shared server without prompting agents. Client checks are documented in
[`client/README.md`](../client/README.md).

The earlier backend cutover was verified with native Linux and macOS nodes, the rebuilt Mac app
opening this repository through one SSH tunnel, both machines' existing shared Herdr
sessions, and the agent MCP reading the backend page. The Afterburn document container is
stopped; its migration data is retained for rollback.

Packaging lifecycle tests mock all supervisors and node requests. Those tests do not establish
native launchd/systemd deployment or app-quit survival for the current build; verify those on
the destination hosts after deployment.
