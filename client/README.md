# Afterburn desktop client

Afterburn's app (`figcraft`) and backend (`afterburn-node`) belong to the root Cargo workspace.
The only FigCraft source tree is `server/figcraft/`; the node crate is `service/`. Both use the
root `Cargo.lock` and root `target/`. The old ignored `client-src/` checkout and source overlays
are not build inputs. Edit the canonical source directly.

## Full Mac install

Run from the repository root on macOS, with Rust 1.97.1, Xcode command-line tools and Python 3.9+
installed:

```sh
./client/build-mac.sh
open "$HOME/Applications/Afterburn.app" --args --map-mode --node 127.0.0.1:47822
```

The build script runs **one** `cargo +1.97.1 build --locked -p figcraft -p afterburn-node` invocation
(with the release profile by default). It creates and ad-hoc signs `Afterburn.app`, bundles the
native node produced by that invocation, and runs the bundled helper's installer. The helper
is copied to `~/.local/lib/afterburn/node/<build-id>/afterburn-node`; launchd points directly to that
immutable copy. `~/.local/bin/afterburn-node` is a CLI symlink to it. Quitting or replacing the app
leaves the helper and shared Herdr running.

`APP` selects a bundle destination, `PROFILE=debug` selects debug binaries, and `CARGO_TOOLCHAIN`
overrides the pinned toolchain. `INSTALL_NODE=0` builds a package without registering a helper;
this is not a completed full install. macOS user installation requires a logged-in GUI user
and Python 3.9+. The helper is a LaunchAgent, so login starts it; it is not a machine-wide boot
service before login. Logs are in `~/Library/Logs/Afterburn/`.

A running node from a different build is left untouched unless the user approves an upgrade:

```sh
# Explicitly approved full install and helper upgrade:
UPGRADE_NODE=1 ./client/build-mac.sh
# Or upgrade the helper from an already-built app:
./service/install-mac-node.sh "$HOME/Applications/Afterburn.app" --upgrade
```

A prepared Sync or ready merge prevents an upgrade. The installer checks `session.list`, verifies
that the selected supervisor owns the listener's PID, then obtains the node's `upgrade.prepare`
lease before stopping it. That lease prevents a new save gate from racing the restart. Failed
starts restore the previous service definition when possible; immutable older binaries remain
available. A legacy node without identity/PID reporting needs explicit operator migration.
The installer does not kill unmanaged processes or stop Herdr.

## Deploy this checkout to a Mac

```sh
./client/deploy-mac.sh macbook
# Only after approving replacement of the running node:
UPGRADE_NODE=1 ./client/deploy-mac.sh macbook
```

Deployment copies the current canonical workspace, lockfile, sources, assets and packaging scripts
into an isolated `~/.cache/afterburn/builds/full.*/source` snapshot on the Mac. It includes new
nonignored source files, so uncommitted work can be deployed. It does not reconstruct an overlay,
copy `client-src`, overwrite a worktree, or copy private design/server state. The remote root
build uses `~/.cache/afterburn/target` as its shared build cache. Snapshots are retained for inspection.
The Mac needs the same build prerequisites as a local build; the deploying host needs Git,
Python 3, SSH and rsync. SSH host keys/authentication use the user's configuration.

## Optional companion platforms

Native node input is **never** required separately. `NODE_ASSETS` is optional and supplies remote
platforms only. Build each companion natively from the **same canonical source contents** and
root lockfile using `service/build-node.sh` (see [headless installation](../service/NODE.md)).
The script emits a binary plus `build-info.json` containing the shared source `build_id`, product
version, protocol, platform and binary SHA256. Packaging verifies those fields against its native
node and refuses stale, missing or modified sidecars. Foreign binaries are never executed during
Mac packaging. Ad-hoc signing refreshes Mach-O checksums without changing their build identity.
Sidecars establish matching build provenance; they are not a signed supply-chain attestation.

```sh
# On Linux, with this exact canonical source snapshot:
./service/build-node.sh "$PWD/build/node"
# From that same checkout, copy companions while building app + native Mac node:
NODE_ASSETS="$PWD/build/node" ./client/deploy-mac.sh macbook
```

The supported asset directory names are `darwin-arm64`, `darwin-x64`, `linux-arm64` and `linux-x64`.
A normal Mac build includes only its native architecture. Other platforms require corresponding
native builds; naming a directory does not make a cross-platform binary. Linux builds retain their
host libc/runtime requirements. Windows nodes and cross compilation are not packaged here.

## Nodes and host selection

Use `--map-mode --node 127.0.0.1:47822` locally or `--node ssh:befree` for an SSH config alias.
`AFTERBURN_NODE` supplies the default address; `--service` remains an alias for `--node`. Remote
connections use SSH to a loopback listener. Workspaces open by session ID, worktree path and
branch; no owner links or server keys are used.

Explicit host setup installs the matching bundled node through `Resources/install-node.sh` and
`install-node.py`, using launchd on macOS or systemd on Linux. Python 3.9+ and the selected service
manager must be available on the host. Root Linux hosts use system scope; others use user scope.
A different build is shown as an update requiring approval. Ordinary reconnect does not silently
replace or launch a node. When the app lacks a host's platform asset, build/package that platform
or install its matching node manually first. Existing installed nodes can be reached without a
bundled companion asset.

Herdr installation needs separate approval when missing. Setup uses the shared Herdr session
and preserves `HERDR_SESSION`/`HERDR_SOCKET_PATH`; it never starts a private Herdr daemon. Saved
hosts live in `~/.config/afterburn/hosts.json`, for example:

```json
[{"name":"befree","address":"ssh:befree","ssh":"befree"}]
```

Comments, first-page prompts, diffs and Sync requests carry the document's node session. Changing
hosts cannot redirect an open map. Collaboration, Herdr status and job events share a connection.
A checkout queues a reopen on the same node after pending workspace activity finishes. Lost
mutating requests are not retried automatically because their outcome may be unknown. Quitting
closes pooled connections and SSH tunnels; the independently supervised node remains running.

## Releases (GitHub Actions)

`.github/workflows/release.yml` builds and publishes the downloadables, so the download page can
link to stable asset URLs (`…/releases/latest/download/<asset>`) instead of files on the web host:

| Asset | Built on | Notes |
|---|---|---|
| `afterburn-macos-arm64.dmg` / `.zip` | `macos-15` | `client/build-mac.sh` with `INSTALL_NODE=0`, ad-hoc signed |
| `afterburn-macos-x64.dmg` / `.zip` | `macos-15-intel` | same script, Intel |
| `afterburn-linux-x64.AppImage` / `.tar.gz` | `ubuntu-22.04` | `client/build-linux.sh`, glibc 2.35 baseline |
| `afterburn-windows-x64.zip` | `windows-2025` | app plus `node/windows-x64/afterburn-node.exe` |
| `SHA256SUMS` | — | checksums of everything above |

- A push to `main` rebuilds the rolling **nightly** release (deleted and recreated, marked latest).
- A `v*` tag publishes that tag as its own release, with generated notes:
  `git tag v0.1.0 && git push origin v0.1.0`.
- **Actions → Release → Run workflow** repeats either, and takes an optional tag.

## Windows

The node builds and runs natively on Windows: Herdr's control socket is a named pipe there
(`interprocess` derives the pipe name from the same socket path on both sides), Herdr's installer is
`install.ps1`, and `%APPDATA%\herdr` replaces `~/.config/herdr`. The app has no `sh` and no service
manager, so on first run it starts the bundled `node/windows-x64/afterburn-node.exe serve` detached
(no console window) and waits for 127.0.0.1:47822; the node keeps serving after the window closes. Herdr itself is still installed only with explicit approval, from the Add machine
flow.

Herdr's Windows support is documented at <https://herdr.dev/tui/docs/windows-beta/> (ConPTY panes,
some integrations narrower than Unix).

Builds are unsigned: macOS bundles are ad-hoc signed but not notarized (first launch needs
right-click → Open), and the Windows binary is not Authenticode signed. Signed builds need the
Apple/Windows certificates wired into the workflow as secrets.

## Verification

```sh
cargo test -p figcraft-ui-egui node --lib
python3 -m unittest discover -s service -p 'test_install_node.py' -v
bash -n client/build-mac.sh client/build-linux.sh client/deploy-mac.sh service/build-node.sh service/install-mac-node.sh
sh -n service/install-node.sh
```

Packaging tests use temporary directories and mocked supervisors/RPCs. Native build, signature,
launchd and app-quit survival checks must be performed on the destination Mac.
