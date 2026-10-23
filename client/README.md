# Mindmap desktop client

Mindmap reuses FigCraft's engine, collaboration model, canvas, and page tree. In map mode the
app connects only to Mindmap nodes: one persistent protocol-v1 connection per machine, over
loopback or one SSH tunnel. Ordinary FigCraft mode keeps its existing transport.

## Reproduce the complete client

The ignored `client-src/` directory is a working checkout, not the source of record.
`source-overlay/` contains **every differing file** relative to `server/figcraft/`, including
previous Mindmap features and the node integration. `source-overlay.json` records baseline
and resulting checksums. The old `mindmap-client.patch` is historical and incomplete.

```sh
cp -a server/figcraft /tmp/mindmap-client
python3 client/apply-source-overlay.py /tmp/mindmap-client
cargo +1.97.1 check --manifest-path /tmp/mindmap-client/Cargo.toml -p figcraft
cargo +1.97.1 test --manifest-path /tmp/mindmap-client/Cargo.toml -p figcraft-ui-egui node --lib
```

The apply script checks all inputs before writing and refuses unrelated checkout edits. It is
idempotent. After editing `client-src/`, run `python3 client/refresh-source-overlay.py` and review
the resulting overlay changes before committing. No generated binaries or build caches belong
in the overlay.

## Nodes and host selection

Launch with `--map-mode --node 127.0.0.1:47822` for local work, or `--node ssh:befree` for an SSH
config alias. `MINDMAP_NODE` supplies the default address; `--service` remains a CLI alias for
`--node`. Raw remote TCP connections are refused. Workspaces open by session ID, worktree path,
and branch; no owner links or server keys are used.

The host picker installs/starts the bundled node in a worker thread. It requests approval only
when Herdr needs installation, then ensures Herdr's shared session is running. Remote host
setup reuses a matching saved Herdr machine or registers one targeting that same session.
It never creates a private Herdr instance. SSH authentication and host keys use the user's SSH
configuration; password prompts must be completed in a terminal first. Local Herdr discovery
honors `HERDR_BIN_PATH`, then checks `~/.local/bin`, `~/.cargo/bin`, Homebrew, `/usr/local/bin`,
and finally `PATH`, including GUI launches with a minimal environment.

Saved hosts live in `~/.config/mindmap/hosts.json`:

```json
[{"name":"befree","address":"ssh:befree","ssh":"befree"}]
```

Node binaries belong in the app bundle's `Contents/Resources/node/{platform}/mindmap-node`,
where platform is `darwin-arm64`, `darwin-x64`, `linux-arm64`, or `linux-x64`. The app also checks
`~/Applications/Mindmap.app/Contents/Resources/` when running outside its bundle. Remote setup
copies the appropriate asset to `~/.local/bin/mindmap-node` and starts
`mindmap-node serve --bind 127.0.0.1:47822`.

## Behavior and validation

Comments, first-page prompts, diffs, and sync requests carry the active document's node session,
so choosing another host cannot redirect an open map. The node owns the design snapshot.
Collaboration messages, live Herdr status, and job updates share the connection; periodic job
queries remain a fallback. A checkout notification queues an automatic reopen of the active
workspace on its selected node after any pending workspace action finishes; the new branch
session supplies its existing page tree. The agent MCP opens the current branch on each call.
A protocol mismatch or lost connection is shown as an error. Lost
mutating requests are never retried automatically because their outcome may be unknown.

Transport tests cover interleaved events and out-of-order RPC replies on one socket, protocol
rejection, disconnect errors, the engine's actual join/welcome/presence path, loopback checks,
and SSH target validation. Shutdown drains pooled connections and reaps their SSH processes,
including the Mac app's explicit exit path; a quit/reopen cycle was verified on the deployed app.
Existing page-tree tests cover the retained page hierarchy.

`build-mac.sh` wraps the desktop executable and icon, copies `NODE_ASSETS` into the bundle,
and requires the local-platform node before packaging. Include the Linux platform directories
in `NODE_ASSETS` for remote setup. `deploy-mac.sh` reconstructs the full tracked overlay by
default (or accepts a working source directory as its second argument), copies complete sources
to the Mac, and optionally copies `NODE_ASSETS` before building/signing. It refuses to deploy
an app missing its local node. The node deployment workflow builds these binaries separately.
