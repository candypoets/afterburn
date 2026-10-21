# service — Mindmap project service (Rust)

A Herdr workspace is a project. This binary lists the workspaces on the host, gives each one a
FigCraft document the first time it is opened, and, on **Init**, makes sure an agent is working on
it. `adopt` attaches a map that was already drawn, without prompting an agent. The edit link (with
its token) stays in the service; callers get a link and a file id they can hand to a client that
knows how to join.

This is the Rust port of the behavior in [`../project/service.py`](../project/service.py), with the
product shape from [`../project/README.md`](../project/README.md). The Python file is unchanged.

## What is here

| File | Role |
|---|---|
| `src/main.rs` | CLI: `list`, `open`, `init`, `adopt`, `serve`, help. |
| `src/project.rs` | Workspace inventory, publish-or-reuse, Init orchestration, adopt. |
| `src/herdr.rs` | Typed wrapper over `herdr workspace/agent/tab/pane`; always `HERDR_ENV=1`. |
| `src/mcp.rs` | Newline-delimited JSON-RPC client for `figcraft-cli mcp`. |
| `src/agent.rs` | Agent detection, tab creation, start, prompt, in-band failure. |
| `src/records.rs` | `service/data/projects.json` store (atomic writes, mode 0600). |
| `src/server.rs` | Newline-delimited JSON TCP API. |
| `src/link.rs` | Rewrites the join link's `host` parameter to the public address. |
| `src/config.rs` | Paths and environment overrides. |
| `src/time.rs`, `src/error.rs` | UTC timestamps and the small error type. |
| `assets/empty.figcraft` | The one-page skeleton ("No map yet") published on first open. |

The only dependencies are `serde` and `serde_json`; the network side is `std::net` and the MCP
client is `std::process` + `std::sync::mpsc`.

## Build

```sh
cd /root/code/mindmap/service
CARGO_HOME=/root/code/mindmap/.cargo-home cargo build --release
```

The repository has a pre-populated `.cargo-home`; plain `cargo build --release` works too when the
normal registry is available. `CARGO_HOME` is only needed when the default is not writable.

The binary is `target/release/mindmap-service`. It finds the repository root by walking up from the
current directory to the checkout containing `service/assets/empty.figcraft`, or from
`CARGO_MANIFEST_DIR` at build time. `MINDMAP_ROOT` overrides that for an installed binary.

## Run

```sh
./target/release/mindmap-service list
./target/release/mindmap-service open eisledger
./target/release/mindmap-service init eisledger
./target/release/mindmap-service init wX --target pane:wX:p2
./target/release/mindmap-service adopt wX ../build/mindmap-system.figcraft --name "Mindmap · mindmap"
./target/release/mindmap-service serve
```

`list` prints one line per workspace, in the same table shape as the Python service:

```
wW    alzette             none   1 agent · idle      /root/code/alzette
wP    eisledger           empty  1 agent · idle      /root/code/eisledger
...
wX    mindmap             none   3 agents · done     /root/code/mindmap
```

`open` prints JSON:

```json
{
  "workspace": "wP",
  "label": "eisledger",
  "path": "/root/code/eisledger",
  "link": "figcraft://join?host=100.104.9.15%3A47910&file=3e3bb408a90e35a1&token=…",
  "file": "3e3bb408a90e35a1",
  "name": "Mindmap · eisledger"
}
```

`init` prints the same object plus `agent` (see below).

### Environment

| Variable | Default | Meaning |
|---|---|---|
| `MINDMAP_ROOT` | discovered checkout | Root with `service/`, `server/`, `skills/`. |
| `MINDMAP_DATA` | `<root>/service/data` | State directory; records live in `projects.json`. |
| `MINDMAP_LEGACY_DATA` | `<root>/project/data` | One-time import source for Python-era records. |
| `MINDMAP_BIND` | `0.0.0.0:47821` | TCP API address. |
| `MINDMAP_SERVER` | `100.104.9.15:47910` | Public document-server address written into links. |
| `MINDMAP_LOCAL_SERVER` | `ws://127.0.0.1:47910/` | Server the CLI publishes to; must be reachable locally. |
| `MINDMAP_HERDR` | `herdr` | Herdr executable. |
| `MINDMAP_FIGCRAFT_CLI` | `<root>/server/figcraft/target/release/figcraft-cli` | FigCraft CLI with `mcp`. |

## Inventory

`workspaces()` runs `herdr workspace list` and `herdr agent list` (never starts, stops, or focuses
anything), parses the `result` field, and merges:

* `id`, `label`, `focused`, `tabs`, `panes` from the workspace;
* `path` = `worktree.checkout_path` when the workspace has a checkout. Otherwise it runs
  `herdr pane list --workspace <id>` and uses the focused pane's `cwd` (else the first pane's
  `cwd`). `repo` = `worktree.repo_name`, `worktree` = `worktree.is_linked_worktree`. A workspace
  created without git (such as the Mindmap repository, `wX`) therefore still has a map root;
* `agents` = number of agents in the workspace, `agent_names` = sorted unique agent kinds;
* `agent_status` = the workspace's status (`unknown` when absent);
* `agents_detail` = one object per agent with a pane: `pane`, `kind` (the Herdr `agent` field),
  `name` when the payload has one, `status`, and `focused`;
* `clis` = the supported agent CLIs found on this host's `PATH`, in `AGENT_ORDER`
  (`codex`, `claude`, `kimi`, `opencode`, `gemini`, `cursor`);
* `map` = `map` when the record is built, `empty` when a record exists but is not built, `none`
  when there is no record; `file` is the record's published file id;
* sorted by lowercased label, then id.

## Publishing a skeleton

`ensure_project` reuses a record whose `edit` link is non-empty. Otherwise it:

1. reads `server/data/server.key` (trimmed) and requires it to be non-empty;
2. spawns `figcraft-cli mcp`, then `initialize` (protocol `2025-03-26`, client
   `mindmap-service`/version), `notifications/initialized`;
3. opens `service/assets/empty.figcraft`;
4. runs `collab.publish` with `server = ws://127.0.0.1:47910/?key=…` (or
   `MINDMAP_LOCAL_SERVER`) and `name = "Mindmap · <label>"`;
5. polls `collab.peers` every 500 ms, up to 60 times (30 s), until `status == "live"`. A
   `"failed"` status fails early with its `reason`;
6. reads `published.edit` (falling back to `link`) and `published.file`/`published.view`;
7. rewrites the `host` query parameter to `MINDMAP_SERVER`, keeping `file` and `token` exactly,
   and stores the record with `built: false`.

The same routine backs `adopt`: it opens the given `.figcraft` file instead of the skeleton,
publishes it with the requested name, and stores the record with `built: true`.

Tool results are parsed from `content[0].text` as JSON; `isError: true` is an error. A 60 s RPC
timeout is enforced on the MCP reader thread, so a wedged CLI cannot hold a service thread forever.

The reference's `run_command` command for opening a file is `file.open` in this CLI build;
`open_file` is an MCP tool and that is what the Python reference calls. The service uses the
`open_file` tool and falls back to `run_command {"command":"file.open"}` only if the tool is
missing.

## Init and agents

`init` accepts an optional target, from the CLI (`--target`) or the TCP request field:

* **absent or `auto`** keeps the original policy: prompt an existing agent, else start the first
  installed CLI in `AGENT_ORDER`;
* **`pane:<pane-id>`** (`pane:wX:p2`) prompts that exact agent. It must be an agent in the resolved
  workspace and must have a pane; otherwise the request fails before anything is published;
* **`kind:<kind>`** (`kind:claude`) always starts a new agent of that kind in a new background tab,
  even when the workspace already has agents. The kind must be one of `AGENT_ORDER`; a kind that
  is not installed fails in-band at the agent stage.

A syntactically invalid target, an unsupported kind, or a pane that is not in the workspace is a
request error (`ok: false`), not an in-band agent failure. This validation happens before
`ensure_project`, so a bad picker choice cannot publish a skeleton or start an agent.

After validation, `init` opens/publishes the document (so the link exists even if the agent stage
fails), then:

* lists agents again; a `pane:` target prompts exactly that agent, while `auto` prompts an existing
  agent (preferring a focused agent and avoiding one already `blocked`);
* otherwise requires a checkout path, starts the requested (or first installed) kind, creates a
  background tab with `herdr tab create --workspace <id> --cwd <path> --label mindmap --no-focus`,
  reads `result.tab.tab_id` and `result.root_pane.pane_id` (falling back to
  `pane list --workspace` filtered by tab), starts
  `herdr agent start mindmap --kind <kind> --pane <pane>` (using `mindmap-2`, … if the name is live
  elsewhere), and submits the prompt with `herdr agent prompt <pane> <text>`.

The prompt tells the agent to read `<root>/skills/mindmap/SKILL.md`, map `<path>`, write
`<path>/.mindmap/model.json`, render into the project, start with a sourced 6–12-area Overview,
and includes the document file id when one exists. It is submitted without `--wait`: Init returns
when the prompt is accepted, not when the agent finishes.

Agent failures are in-band and do not fail the document part:

```json
{
  "ok": true,
  "link": "figcraft://join?…",
  "file": "3e3bb408a90e35a1",
  "name": "Mindmap · eisledger",
  "agent": {
    "state": "error",
    "target": "kind:claude",
    "kind": "claude",
    "reason": "agent CLI \"claude\" is not installed on PATH"
  }
}
```

Success states are `started` (new CLI) or `existing` (prompted), with `target`, `kind`, `name`,
`pane`, `tab`, and `prompted: true` when known. A workspace with no checkout path and no pane with
a `cwd` gets an error outcome rather than an agent in the wrong directory.

## Adopting a built map

`adopt` seeds a workspace whose map was already drawn, without prompting any agent:

```sh
cd /root/code/mindmap
./service/target/release/mindmap-service adopt wX build/mindmap-system.figcraft
./service/target/release/mindmap-service adopt wX build/mindmap-system.figcraft --name "Mindmap · mindmap"
```

It opens the given `.figcraft` file, publishes it through the same MCP `collab.publish` path as
`open`, stores the record with `built: true`, and prints:

```json
{"file":"…","link":"figcraft://join?…","name":"Mindmap · mindmap"}
```

The name defaults to `Mindmap · <label>`. The `link` is the owner edit link; like every other
stored link it contains a token and is not logged. The workspace may have an existing record; it
is replaced. The TCP API exposes the same operation as `adopt` (see below).

## TCP API

`serve` binds `MINDMAP_BIND` (default `0.0.0.0:47821`). The framing is one JSON object per line,
in both directions; a connection may carry any number of requests. One thread serves each
connection. There is no auth or TLS yet.

```sh
printf '%s\n' '{"id":1,"method":"health"}' | nc 127.0.0.1 47821
```

Requests:

| Method | Request | Success response |
|---|---|---|
| `health` | `{"id":1,"method":"health"}` | `{"id":1,"ok":true,"service":"mindmap-service","version":"0.1.0","server":"100.104.9.15:47910"}` |
| `workspaces` | `{"id":2,"method":"workspaces"}` | `{"id":2,"ok":true,"workspaces":[…]}` |
| `open` | `{"id":3,"method":"open","workspace":"wP"}` | `{"id":3,"ok":true,"link":"figcraft://…","file":"…","name":"…"}` |
| `init` | `{"id":4,"method":"init","workspace":"wP","target":"pane:wX:p2"}` | same as `open` plus `"agent":{…}` |
| `adopt` | `{"id":5,"method":"adopt","workspace":"wX","path":"build/mindmap-system.figcraft","name":"Mindmap · mindmap"}` | `{"id":5,"ok":true,"file":"…","link":"figcraft://…","name":"…"}` |

`target` is optional: absent or `"auto"`, `"pane:<pane-id>"`, or `"kind:<agent-kind>"`. `name` is
optional for `adopt` and defaults to `Mindmap · <label>`.

`workspace` accepts the Herdr id (`wP`) or label (`eisledger`), case-insensitively as a fallback.
Errors are `{"id":N,"ok":false,"error":"…"}`. Invalid JSON gets `id: null`; the connection stays
open. Request lines are capped at 1 MiB.

## State and security

Records are `service/data/projects.json`, keyed by workspace id, with the reference shape:

```json
{"id","label","path","file","edit","view","name","built","created"}
```

The edit/view links contain tokens, so the store is written atomically with mode `0600` and links
are never logged. On first use, if the new store does not exist, records from
`project/data/projects.json` are imported so an already-published workspace (eisledger) keeps its
document instead of getting a duplicate. Later imports do not happen.

## Verification performed

* `cargo build --release`: clean, no warnings.
* `cargo test --release`: 26 unit tests pass, including the original 15. The new tests cover
  target parsing (`auto`, `pane:`, `kind:`, invalid), pick policy (pane selection, kind always
  starts new, auto reuse/fallback/blocked avoidance), target error reporting, and the pane-cwd
  fallback.
* `cargo clippy --release --all-targets`: clean.
* `list` from the service directory and from the repository root shows the real workspaces. The
  `wX` (`mindmap`) line now has `/root/code/mindmap`; worktree-backed workspaces still use
  `worktree.checkout_path`.
* A TCP test instance on `MINDMAP_BIND=127.0.0.1:47899` with a scratch `MINDMAP_DATA` returned
  `agents_detail` (including `wX:p2`/`claude`/`idle`) and `clis` (`codex`, `claude`, `kimi`) for
  `wX`, and preserved the worktree path for `wP`.
* The same instance rejected `pane:wP:p5`, `pane:wX:p999`, `kind:not-a-cli`, `bogus`, a
  non-string target, a missing `adopt` path, and a non-existent adopt file. All returned
  `ok: false`; the scratch store stayed empty and `wX` kept its 3 agents and 3 tabs, so no agent
  was started or prompted.
* `adopt` was exercised with the scratch store: the CLI adopted `build/mindmap-system.figcraft`
  for `w11` and the TCP method adopted it for `wZ` with the default name. Both published through
  the MCP path, rewrote the link host, stored `built: true`, and made the inventory show `map`
  with the new file id.
* The real `service/data/projects.json` was not modified: it still contains only the `wP`
  record.
* No Herdr tab, pane, workspace, or session was modified during testing.

## Not done yet / known gaps

* **Mac launch and auto-join.** The Python service starts `Mindmap.app` over SSH and drives its
  control channel (`engine.execute collab.join`, Escape). This port stops at the internal link;
  the client/launcher half is a separate slice.
* **HTTP.** The API is TCP only; `/health`, `/workspaces`, `/open/<id>` and `/init/<id>` are not
  served.
* **No authentication or TLS.** Bind to the tailnet or loopback with `MINDMAP_BIND`, or put a
  gateway in front. Anyone who can reach the port can ask to publish/prompt.
* **Agent-built maps stay `built: false`.** The model renderer (`mindmap render`) and the code
  that validates sources and publishes the real map do not exist in this crate. Only `adopt`
  marks a record `built: true`; an agent following the prompt still leaves the record as `empty`.
* **Adopt trusts the file.** It publishes whatever `.figcraft` path it is given and does not
  validate that the document is a mindmap or that it was produced by the skill. Any workspace
  record can be replaced by `adopt`.
* **No job lifecycle.** Init submits the prompt and returns; there is no job record, progress,
  retry, agent stop, or cleanup. If a tab was created and `agent start`/`prompt` then failed, the
  empty tab is left in place for diagnosis.
* **Local Herdr only.** Remote Herdr machines / merged multi-host inventory are not implemented,
  and the service does not ensure or start a private Herdr session.
* **Inter-process publish race.** One `ProjectService` serializes publish-or-reuse with a mutex,
  but a CLI process and a running `serve` can still publish the same missing workspace at the same
  time. A shared file lock is future work.
* **Workspaces without a checkout path or pane `cwd`** can be listed and opened (skeleton), but
  Init returns an agent error because there is no directory to map or prompt about. The pane-cwd
  fallback covers the normal case (including `wX`).
* **Not ported from `service.py`:** the `MINDMAP_MAC` SSH/control path, `MINDMAP_APP`, and the
  Mac-side `open` side effect. The link is returned instead.
* Unit tests cover pure helpers; the Herdr tab/agent start path is untested on purpose because it
  mutates a real session. Re-exercise it in a scratch Herdr workspace before shipping.
* Linux/macOS are the targets. File mode hardening is Unix-only; the `which` scan does not
  implement Windows `PATHEXT`.
