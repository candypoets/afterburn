# service — Mindmap project service (Rust)

A Herdr workspace is a project. This binary lists the workspaces on the host, gives each one a
FigCraft document the first time it is opened, and, on **Init**, makes sure an agent is working on
it. **Send** prompts an agent with canvas comments without opening or publishing a document.
**Diff** compares the client's map snapshot with the last successful sync and with the repository,
Every `send` is preceded by a **MINDMAP CONTEXT** packet: workspace, repository, document id,
owner edit link, FigCraft CLI path, page name/id, top-level cards, and for each commented node its
id, path, type, text and children. The packet also says how to edit the live document (MCP:
`collab.join`, `get_node`, `run_command`) and never includes the link in a log. The builder joins
the live document read-only through the FigCraft CLI; if that fails, the static workspace/page
context is still sent and the failure is named.

**dsh-tui** is detected by Herdr but rejects the agent prompt protocol as an unnamed pane. The
service falls back to `herdr pane run` (literal text + Enter) for an **idle** dsh; a working dsh is
refused with a reason rather than silently accepted. `send` now returns `ok:false` when the agent
did not accept the prompt, so the client keeps comments and shows the error.

**Sync** prompts an agent with both real diffs and starts an in-memory job, and **sync_status**
follows that job until Herdr shows the agent finished. `adopt` attaches a map that was already
drawn, without prompting an agent. The edit link (with its token) stays in the service; callers get
a link and a file id they can hand to a client that knows how to join.

This is the Rust port of the behavior in [`../project/service.py`](../project/service.py), with the
product shape from [`../project/README.md`](../project/README.md). The Python file is unchanged.

## What is here

| File | Role |
|---|---|
| `src/main.rs` | CLI: `list`, `open`, `init`, `adopt`, `serve`, help. |
| `src/project.rs` | Workspace inventory, publish-or-reuse, Init/Send orchestration, adopt. |
| `src/herdr.rs` | Typed wrapper over `herdr workspace/agent/tab/pane`; always `HERDR_ENV=1`. |
| `src/mcp.rs` | Newline-delimited JSON-RPC client for `figcraft-cli mcp`. |
| `src/agent.rs` | Agent detection, tab creation, start, prompt (Init and Send), in-band failure. |
| `src/comments.rs` | Canvas-comment validation and prompt rendering for `send`. |
| `src/sync.rs` | Map snapshot validation, design/code diffs, sync prompt, job transitions. |
| `src/records.rs` | `service/data/projects.json` store (atomic writes, mode 0600, `last_sync`). |
| `src/server.rs` | Newline-delimited JSON TCP API (`health`, `workspaces`, `open`, `init`, `send`, `adopt`, `diff`, `sync`, `sync_status`). |
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

## Sending canvas comments

`send` is the no-document path: a client submits the comments collected on the canvas plus an
optional prompt to an agent in a workspace. It resolves the workspace, validates the target,
renders the comments, and submits one prompt. It never publishes, opens, or edits a document and
never writes the project store.

```json
{"id":1,"method":"send","workspace":"wX","target":"pane:wX:p2","prompt":"Why does this rebuild?","comments":[{"text":"This should say where overrides reset.","node":"Components","page":"Mindmap"}]}
```

* `workspace` (required): id or label, resolved like every other method (exact id or label first,
  then case-insensitive; prefer the id when labels collide).
* `target` (optional): `auto`, `pane:<pane-id>`, or `kind:<agent-kind>`, exactly as Init. It is
  validated against the live agent list before any prompt or tab, so an unknown pane, unsupported
  kind, or malformed target returns `ok:false` without mutations.
* `prompt` (optional string, may be empty): free-form instructions.
* `comments` (optional list, at most 50): objects with a required string `text` and optional string
  `node` and `page`. Blank text, non-string fields, more than 50 comments, or more than 20,000
  characters of comment content are `ok:false`. Payload validation happens before the workspace is
  resolved, so a bad request cannot touch Herdr.

The prompt submitted to the agent is:

```text
Mindmap comments for <workspace label> (<checkout path>):

1. [<page>] <node>: <text>
2. ...

<prompt>
```

Missing pieces are omitted: the parenthetical when the workspace has no path, `[<page>]` when the
comment has no page, `<node>: ` when it has no node, and the blank line when `prompt` is empty. An
empty prompt is replaced with the one-line instruction
`Address these comments in the project document.` Without any comments the method submits the
prompt alone (so a prompt-only request still needs a prompt). Newlines, tabs, and backslashes in
`text`, `node`, and `page` are escaped (`\n`, `\t`, `\\`) so each comment stays on one numbered
line. Comment content above the cap is rejected rather than truncated.

Agent handling is shared with Init: a `pane:` target prompts that agent, `auto` reuses an existing
agent (focused first, avoiding blocked ones) or starts the first installed CLI, and `kind:` starts
a new agent of that kind in a background tab. First-run folder trust is accepted the same way.
Agent failures are in-band, like Init:

```json
{
  "id": 1,
  "ok": true,
  "sent": 1,
  "agent": {
    "state": "error",
    "target": "pane:wX:p2",
    "pane": "wX:p2",
    "reason": "…"
  }
}
```

`sent` is the number of comments included in the prompt; `agent.prompted` is true only when Herdr
accepted it. Like Init, the agent stage needs a checkout path; without one the request still
returns `ok:true` with the error in `agent`.

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

## Diffing and syncing the map

A sync is two real diffs, not a semantic guess. The client supplies the map it is showing; the
service compares it by node id with the snapshot written by the last successful sync, and runs
`git` against the commit that sync recorded. Both lists go to the agent in one prompt. The service
never locks, opens, or edits the document: the map stays editable while a sync is in flight.

### The map snapshot

`diff` and `sync` both take the whole map as it is on the client:

```json
{
  "pages": [{"id": 1, "name": "Mindmap"}],
  "nodes": [
    {"id": 36, "name": "heading", "parent": null, "page": 1, "kind": "frame", "text": ""},
    {"id": 37, "name": "title", "parent": 36, "page": 1, "kind": "text", "text": "The workspace client"}
  ]
}
```

Validation happens before the workspace is resolved or anything is prompted:

* `map` must be an object with a `pages` list and a `nodes` list; both are required and may be
  empty. Any other shape, a missing list, or a non-object entry is `ok:false`.
* Page and node `id`s are positive integers and unique. A node's `page` must be one of the page ids.
* `parent` is optional: missing, `null`, or a positive integer. It is recorded as sent; the service
  does not require the parent node to be in the same snapshot.
* `name`, `kind`, and `text` are strings. Missing or non-string values are rejected (page `name` and
  node `name` are required too).
* At most **500 pages** and **5,000 nodes**. A larger snapshot is rejected, never truncated. The
  request line itself is capped at 1 MiB.
* Unknown fields are ignored, not stored.

The snapshot is captured at sync start. `diff` does not store it. `sync` keeps it in the job and
writes it to `last_sync` only when the agent finishes successfully.

### `diff`

```json
{"id":2,"method":"diff","workspace":"wX","map":{"pages":[{"id":1,"name":"Mindmap"}],"nodes":[…]}}
```

```json
{
  "id": 2,
  "ok": true,
  "last": {"exists": true, "commit": "3dbbcd8…", "at": "2026-10-22T12:00:00Z"},
  "map": {
    "added": [{"id": 9, "name": "New card", "page": "Mindmap"}],
    "removed": [{"id": 7, "name": "Old card", "page": "Mindmap"}],
    "changed": [{"id": 3, "name": "Components", "fields": ["name", "text"]}]
  },
  "code": {
    "head": "ce9d2b2…",
    "from": "3dbbcd8…",
    "files": [{"path": "crates/x", "added": 10, "removed": 2}],
    "commits": [{"hash": "abc1234", "subject": "Fix the map"}]
  }
}
```

* `last` is the workspace's recorded last successful sync. With none it is `{"exists": false}`.
* `map` compares the supplied snapshot with `last_sync.map` by node id, sorted by id. Added and
  removed rows carry `name` and the page label: current label for added, last-synced label for
  removed. Changed rows carry the current name and the fields that differ, in `name`, `text`,
  `kind`, `parent`, `page` order. A page rename alone is not a node change, so it is not reported.
* A first sync (`last.exists:false`) reports every node as added and no removed or changed rows.
  The code lists are empty; `code.head` is still filled in when the checkout is a repository.
* `code` runs `git -C <path> rev-parse HEAD`, `git diff --numstat <last>..HEAD`, and
  `git log --format=%h%x09%s <last>..HEAD`. Binary numstat lines (`- -`) count as 0 added/removed.
  Files and commits are each capped at **100**. `code.from` is the last-sync commit.
* A workspace with no checkout path, a directory that is not a git repository, an unknown last
  commit, or a git failure is **not a request error**: the code lists are empty and `code.note`
  says why. This is the only extra field in `code`.
* `diff` does **not** write project records. It resolves the workspace from a read-only record view
  (no legacy import) and reads `last_sync` through a read-only store path.

### `sync`

```json
{"id":3,"method":"sync","workspace":"wX","target":"pane:wX:p2","prompt":"optional note","map":{…}}
```

```json
{
  "id": 3,
  "ok": true,
  "job": {"id": "sync-18f3…-1", "state": "in_flight", "started": "2026-10-22T12:01:00Z", "agent": {…}},
  "agent": {"state": "existing", "target": "pane:wX:p2", "pane": "wX:p2", "prompted": true, …}
}
```

`target` is optional and parsed exactly like `send`: `auto`, `pane:<pane-id>`, or
`kind:<agent-kind>`. Before anything is started or prompted, the service requires:

* a target that can actually be prompted: a `pane:` agent that is in the workspace, or for `auto`
  an existing agent/installed CLI, or for `kind:` an installed CLI;
* a non-empty checkout path for the workspace;
* an existing project record (`open` the workspace first). Without one, `sync` is `ok:false` rather
  than inventing a record whose `last_sync` could never be written.

Any failure in that preflight is `ok:false` with no prompt, tab, job, or record write. The agent is
then handled by the same `ensure_with_prompt` policy as Send: reuse an existing agent or start the
requested/first installed kind in a background tab, accept first-run folder trust, and submit the
prompt. If the prompt is not accepted, `sync` is `ok:false` and no job is created. Agent failures
are not in-band here, because a sync without a job would be dishonest.

The submitted prompt is built from the two diffs:

```text
Sync <label> (<path>) against the code.

DESIGN CHANGES SINCE <last|first> sync:
+ <card> (<page>)
~ <card> (name, text)
- <card> (<page>)

CODE SINCE <commit>:
<n files>, <n commits>
  <path> (+a/−r)
  <hash> <subject>

NOTE: <prompt>
```

* `DESIGN CHANGES SINCE last sync:` when a last sync exists, otherwise `… SINCE first sync:`.
  Added, changed, and removed rows are in that order; design and code sections are omitted when
  empty. A first sync therefore has no code section.
* `CODE SINCE <commit>:` uses the commit recorded by the last sync; the count line and rows are
  omitted when there are no files or commits.
* `NOTE:` is omitted when `prompt` is blank. Present notes are trimmed.
* The prompt is capped at **20,000 characters**. Individual lines are shortened to 240 characters,
  the note keeps at most 2,000, and a section that runs out of room ends with `… and N more`.
* Labels, page names, paths, and commit subjects are escaped to one line (`\n`, `\t`, `\\`).

The job is in memory only:

```json
{"id","workspace","started","started_secs","head","map_snapshot","pane","agent","state":"in_flight","seen_active":false}
```

It keeps the head commit and the map snapshot from before the prompt. `sync` itself writes no
records. There is no document lock; the client may keep editing the map.

### `sync_status`

```json
{"id":4,"method":"sync_status","job":"sync-18f3…-1"}
```

```json
{
  "id": 4,
  "ok": true,
  "job": {"id": "sync-18f3…-1", "state": "done", "started": "2026-10-22T12:01:00Z", "agent": {…}},
  "last": {"exists": true, "commit": "ce9d2b2…", "at": "2026-10-22T12:02:30Z"}
}
```

For every poll of an in-flight job, the service reads `herdr agent list` and looks for the job's
workspace and pane:

* `working` or `blocked` sets `seen_active` and keeps the job `in_flight` (blocked means the agent
  is alive and waiting, not that the sync failed).
* `idle` or `done` finishes the job only if `seen_active` is already true. Before the first active
  observation, idle means the prompt has not started yet; the job stays `in_flight`.
* A missing agent (the pane no longer appears) or a status such as `error`, `failed`, `exited`,
  `stopped`, `crashed`, or `gone` fails the job with a `job.reason`.
* A job older than **30 minutes** fails with a timeout before Herdr is polled.
* Unknown statuses keep the job in flight; the timeout is the backstop.
* If `herdr agent list` itself fails, `sync_status` returns `ok:false` and leaves the job in
  flight; the client can retry the poll.

On success, just before reporting `done`, the service writes only `last_sync` through the store:

```json
{"commit": "<head captured by sync>", "at": "<completion time>", "map": {<start snapshot>}}
```

The store lock and atomic rename make that update safe, and all other record fields (including
unknown ones) are preserved. A failed job writes nothing; the previous `last_sync` stays intact.
A finished job is returned as-is on later polls. Unknown or empty job ids are `ok:false`.

### Limits

| Item | Limit |
|---|---|
| Request line | 1 MiB |
| Map pages | 500 |
| Map nodes | 5,000 |
| Code files in a diff | 100 |
| Code commits in a diff | 100 |
| Sync prompt | 20,000 characters |
| Note kept in a prompt | 2,000 characters |
| Prompt line | 240 characters |
| In-flight job | 30 minutes |

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
| `send` | `{"id":5,"method":"send","workspace":"wX","target":"pane:wX:p2","prompt":"Why does this rebuild?","comments":[{"text":"…","node":"Components","page":"Mindmap"}]}` | `{"id":5,"ok":true,"agent":{…},"sent":1}` |
| `adopt` | `{"id":6,"method":"adopt","workspace":"wX","path":"build/mindmap-system.figcraft","name":"Mindmap · mindmap"}` | `{"id":6,"ok":true,"file":"…","link":"figcraft://…","name":"…"}` |
| `diff` | `{"id":7,"method":"diff","workspace":"wX","map":{…}}` | `{"id":7,"ok":true,"last":{…},"map":{…},"code":{…}}` |
| `sync` | `{"id":8,"method":"sync","workspace":"wX","target":"pane:wX:p2","prompt":"optional note","map":{…}}` | `{"id":8,"ok":true,"job":{"id":"sync-…","state":"in_flight",…},"agent":{…}}` |
| `sync_status` | `{"id":9,"method":"sync_status","job":"sync-…"}` | `{"id":9,"ok":true,"job":{…},"last":{…}}` |

`target` is optional: absent or `"auto"`, `"pane:<pane-id>"`, or `"kind:<agent-kind>"`. `name` is
optional for `adopt` and defaults to `Mindmap · <label>`. `send` never returns a link: it only
prompts an agent, and its `comments`/`prompt` rules and prompt format are described in
[Send](#sending-canvas-comments). `diff`, `sync`, and `sync_status` require the `map` snapshot,
target/record rules, prompt, and job lifecycle described in [Diffing and syncing the
map](#diffing-and-syncing-the-map).

`workspace` accepts the Herdr id (`wP`) or label (`eisledger`), case-insensitively as a fallback.
Errors are `{"id":N,"ok":false,"error":"…"}`. Invalid JSON gets `id: null`; the connection stays
open. Request lines are capped at 1 MiB.

## State and security

Records are `service/data/projects.json`, keyed by workspace id, with the reference shape plus the
optional sync state:

```json
{"id","label","path","file","edit","view","name","built","created","last_sync"?}
```

`last_sync` is written only by a successful `sync_status`; its shape is:

```json
{"commit":"<head at sync start>","at":"<completion time>","map":{"pages":[…],"nodes":[…]}}
```

A sync updates only `last_sync`; `built`, `file`, `edit`, `view`, and every other field stay as
they were. Unknown top-level fields are preserved on that write. The store lock and the
write-temp-then-rename path keep concurrent readers and writers safe.

The edit/view links contain tokens, so the store is written atomically with mode `0600` and links
are never logged. On first use, if the new store does not exist, records from
`project/data/projects.json` are imported so an already-published workspace (eisledger) keeps its
document instead of getting a duplicate. Later imports do not happen. Read-only paths (`diff`,
`Store::last_sync`, `Store::all_readonly`) do not trigger that import.

## Verification performed

The commands below were run from `service/` with
`CARGO_HOME=/root/code/mindmap/.cargo-home`.

* `cargo build --release`: clean, no warnings.
* `cargo test --release`: **61 unit tests** and **2 TCP integration tests** pass. The 24 new unit
  tests cover map-snapshot validation (types, ids, duplicate ids, page references, parent, caps),
  map diff (first sync, added/removed/changed, field order, removed-page labels), prompt rendering
  (first/full/empty sections, escaping, 20,000-character cap, omission and note truncation), job
  transitions (working/blocked/idle/done/gone/error/timeout), git numstat/log parsing, a real
  temporary repository, a non-repository note, `last_sync` round-trips with unknown fields, and the
  sync preflight. The original 37 tests still pass.
* `cargo clippy --release --all-targets`: clean.
* `cargo fmt --check`: clean.
* `cargo test --release --test sync_tcp` prints:

  ```text
  running 2 tests
  test diff_sync_and_sync_status_over_tcp ... ok
  test a_fake_agent_finishes_a_sync_and_writes_last_sync ... ok

  test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
  ```

  The first test starts its own binary on `MINDMAP_BIND=127.0.0.1:47899` with scratch
  `MINDMAP_DATA`/`MINDMAP_LEGACY_DATA`, resolves real `wX` from Herdr (read-only), runs `diff` with
  a two-node map, rejects a zero node id, rejects `sync` with `pane:wX:p999999`, and rejects an
  unknown `sync_status` job. The scratch directory stayed empty: no `projects.json`, no agent
  prompt. The second test creates a disposable two-commit git repository, replaces `MINDMAP_HERDR`
  with a shell stub, seeds a scratch record with the older commit and an unknown `future_field`,
  and drives the full path: `diff` reported the design and code changes, `sync` returned
  `in_flight` and the fake agent received the prompt, a `working` poll kept the job in flight, and
  an `idle` poll finished it. The store then had the new `last_sync` commit and the exact start
  snapshot, with `future_field`, `file`, and `edit` untouched. No real Herdr call and no real
  record was involved.
* A manual scratch instance on `127.0.0.1:47899` (scratch data, stopped after the calls, live
  service on 47821 untouched) returned:

  ```text
  {"id":1,"ok":true,"server":"100.104.9.15:47910","service":"mindmap-service","version":"0.1.0"}
  {"code":{"commits":[],"files":[],"from":null,"head":"ce9d2b2cf8270cc5c5ddeb5d8d59bc166eb27cd7"},"id":2,"last":{"exists":false},"map":{"added":[{"id":36,"name":"heading","page":"Mindmap"},{"id":37,"name":"title","page":"Mindmap"}],"changed":[],"removed":[]},"ok":true}
  {"error":"pane wX:p999999 is not an agent in workspace wX (mindmap)","id":3,"ok":false}
  {"error":"unknown sync job \"sync-nope\"","id":4,"ok":false}
  ```

  The scratch data directory remained empty and the log showed only `health`, `diff`, and the two
  rejected requests. This is the exact diff response shape the client can render; `head` changes
  with the repository.
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
* A scratch TCP instance on `127.0.0.1:47899` (scratch `MINDMAP_DATA`, stopped afterwards)
  rejected 15 invalid `send` requests with `ok:false`: unknown workspace, missing/non-string
  workspace, missing/bad/non-string target, unknown pane, unsupported kind, empty comment text,
  empty prompt plus empty comments, non-list comments, non-object comment, non-string comment
  node, and 51 comments. `wX` stayed at 3 tabs, 3 panes, and agent panes `wX:p1`–`wX:p3`; the
  scratch store had no `projects.json`; no agent was started or prompted. Formatting is asserted
  exactly by the unit tests above, and the live service on 47821 was not touched.
* `adopt` was exercised with the scratch store: the CLI adopted `build/mindmap-system.figcraft`
  for `w11` and the TCP method adopted it for `wZ` with the default name. Both published through
  the MCP path, rewrote the link host, stored `built: true`, and made the inventory show `map`
  with the new file id.
* A fake `MINDMAP_HERDR` stub (not the live Herdr) drove the full TCP `send` path. The response was
  `{"id":1,"ok":true,"agent":{"state":"existing","kind":"claude","name":"mindmap","pane":"wX:p2","tab":"wX:t2","target":"pane:wX:p2","prompted":true},"sent":2}`
  and the stub received exactly the formatted prompt shown above. Only `workspace list`,
  `agent list`, `pane list`, and `agent prompt` were called; there were no `tab create`,
  `agent start`, or `agent send-keys` calls, no document was opened, and the scratch store stayed
  empty.
* The real `service/data/projects.json` was not modified by these tests: every TCP instance used a
  scratch `MINDMAP_DATA` directory, and no document was opened or published.
* No Herdr tab, pane, workspace, or session was modified during testing.
* The live service on `47821` was **not** restarted and still runs the previous binary. These
  changes were built only; the parent rollout integrates them.

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
* **Sync jobs are in memory only.** They disappear on restart and have no cancel, retry, progress
  percentage, per-file log, or push notification; the client must poll `sync_status`. Finished
  jobs are retained for the life of the process and are not pruned. Init and Send still have no job
  at all. If a tab was created and `agent start`/`prompt` then failed, the empty tab is left in
  place for diagnosis.
* **The map snapshot is client-supplied.** The service never reads the FigCraft document to check
  it. A client that omits, invents, or mislabels nodes gets a diff for that snapshot. It is used
  only for the prompt and `last_sync`; it does not edit the map.
* **Sync needs a project record and a checkout path.** A workspace with no record (never opened)
  or no `cwd`/`worktree` path is rejected before prompting. There is no code-only sync.
* **Page renames are not diffed by themselves.** Nodes are compared by id; a page label change with
  unchanged nodes is invisible. A node that moves pages is reported with `page` in `changed`.
* **Long sync prompts are truncated.** A section that does not fit the 20,000-character prompt ends
  with `… and N more`; the agent sees a prefix, not every row. The `diff` response still carries the
  full map lists (and up to 100 code files/commits) for the client to render.
* **No semantic design ↔ code mapping.** The two lists are real; which design change should become
  which code change is the agent's job. The service does not compare snapshots to diffs.
* **Git is local and synchronous.** The code diff runs `git` in the workspace checkout on the
  service host, with no remote fetch before the diff. A wedged git process holds that request
  thread; there is no timeout on the git calls.
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
