# project — open a folder, get the map

The goal: **no links in the human's hands.** Opening a local or remote folder gives a project,
its design, its pages, and an agent already working on it. Links and tokens become an internal
detail of the project service.

## The objects

| Object | Holds |
|---|---|
| **Project** | id, name, folder (`/path` or `host:/path`), host, document id on our server, the edit token, the Herdr workspace id, created/updated. |
| **Agent host** | the machine that owns the folder and runs the private Herdr session (one per Mindmap server). |
| **Document** | the FigCraft file on the 0.7.0 fork; pages = the map's altitudes. |

Stored server-side (a small JSON store or SQLite under `server/data/projects/`), never shown to the
human.

## Open flow

```
open folder ──▶ project service ──▶ project exists? ──▶ document exists?
                    │                      │ no                │ no
                    │                      └─ create record     └─ publish a skeleton page, store id + token
                    │
                    ├─ ensure the private Herdr session (start it if down)
                    ├─ ensure a workspace for the folder (`herdr workspace create --cwd …`)
                    ├─ count agents (`herdr agent list`); if 0, start one
                    └─ return {server, file, token} to the client
```

The client receives `{server, file, token}` and calls `collab.join` **itself** — no join dialog, no
link text. `--project <id>` (or a project URL over Tailscale) is the only argument.

Pages are ready immediately: the service publishes a skeleton page (project name, folder, a note
that the agent is reading the repository), then the agent builds the real map on top of it.

## Remote folders

The project service runs where the folder lives, because that is where Herdr, the agent CLIs and
the worktrees are. The server holds the document. The Mac client talks to:

- the document server (`ws://…:47910`) for the canvas,
- the project service (HTTP, over Tailscale) for "open this project".

No ports are exposed publicly; both ride the tailnet.

## Herdr and the agents

- **Bundled, not required.** Ship the pinned `herdr` binary with the server. On startup: use a
  `herdr` already on `PATH` if its version matches; otherwise install the bundled one under
  `server/bin/`. The user never installs or starts it.
- **Private session**, as proven in `server/private-herdr.sh`: own socket under
  `server/data/agents`, every inherited `HERDR_*` variable unset, named session. The user's own
  Herdr is never touched.
- **One workspace per project** (`--cwd` the folder); worktrees for parallel designs come from
  `herdr worktree`.
- **Agent count from Herdr**: parse `herdr agent list` for the project's workspace. If 0, start
  one: detect available CLIs in order (`codex`, `dsh`, `claude`, `opencode`), start the first with
  `herdr agent start`, then prompt it.
- **The prompt** is the `/init` contract: read the repository, emit a map model for our builder,
  render the pages through the FigCraft MCP joined to the project document, and leave sources on
  each card.

## Read-only humans

The client is a viewer with a voice: selection, navigation, comments, questions. The command guard
in map mode refuses every history command except `comment.*` (implemented). The comments surface
(creating a thread on a selection, reading replies) is the next UI piece; today the guard allows
the commands but the panel is still hidden.


## Herdr is the inventory (the new shape)

A **workspace is the project**, not the other way round:

- Mindmap lists the Herdr workspaces on the host(s) — label, repo/worktree path, agents and their
  states — including workspaces that have no map yet.
- A map is attached to a workspace by identity (`host` + worktree path + branch), not by a link.
  No map → the canvas shows an empty state with **Init map**.
- **Init** picks the agent: if the workspace has one, prompt it; if not, start the first available
  CLI (`codex`, `dsh`, `claude`, `opencode`) in a new pane and prompt that.
- If a Herdr daemon is already running on the host, the service **attaches to it read-only for
  listing** and starts agents in it only on Init. If none runs, it starts the bundled one. The
  private session stays for servers that have no user Herdr, so the two never collide.
- Remote hosts are Herdr's saved SSH machines: the service lists them per host, and the client
  shows one merged list.

## Agents write models; the service draws

The agent does not push hundreds of nodes into FigCraft. It writes a **model** and the service
renders it, the same way `design/build_map.py` does today:

```
agent: read repo → write .mindmap/model.json → run `mindmap render` → look at the PNG → iterate
service: validate sources → build components/anchors/pages → publish to the project document
```

That keeps quality (components, slots, anchors, layout) in code we control, keeps layer ids out of
the agent's context, and makes the map updatable: a code change means re-reading the repo, editing
the model, and rendering again. The **mindmap skill** (`skills/mindmap/SKILL.md`) is the contract
for that model: what an area is, how many, what a source must be, when to open a detail page.

## Init flow, end to end

```
workspace selected (Herdr) ─▶ project record (host + worktree + branch)
  ├─ document exists?  no → publish an empty page (the empty state lives in the canvas)
  ├─ agent in the workspace?  no → start the first detected CLI
  └─ prompt: "use the mindmap skill on this repo; model at .mindmap/model.json;
             render into project <id>; stop when the overview has 6–12 sourced areas"
```

The client shows the Init button while the model is empty, and the agent's state
(`working / blocked / idle / done`) in the bar. The first map appears page by page as the agent
renders; comments and questions work from the first card.

## Status — built in Rust

The Python reference was scaffolding and has been removed. The product service is
[`service/`](../service/README.md), a Rust binary (serde only, std TCP), and the client browser is
in `client-workspaces.patch`.

- **Workspace inventory**: `mindmap-service list` reads Herdr's workspaces and agents and marks
  each `map` / `empty` / `none`.
- **In-app browser**: Mindmap opens on the workspace list (map mode): rows with label, path,
  agents and MAP/EMPTY/INIT badges; the right pane opens the map or starts Init.
- **No links**: `mindmap-service` publishes the skeleton, keeps the join link in
  `service/data/projects.json`, and the app joins the returned link through `collab.join`. The
  human never sees it.
- **Init with a choice**: publishes the document, then prompts a chosen agent already in the
  workspace (`pane:<id>`) or starts a chosen CLI (`kind:<codex>`) in a background tab, accepts a
  first-run folder-trust prompt, and submits the mindmap skill prompt. Auto still picks the
  focused/non-blocked agent or the first installed CLI. Verified on disposable worktrees: a fresh
  codex accepted trust; kimi started by kind; the same kimi prompted by pane.
- **Adopt**: `mindmap-service adopt <workspace> <file>` publishes an already-built map and marks
  the workspace ready. Used to give `mindmap` (wX) its first map: "Mindmap · mindmap" with the
  System, Screens and Product pages (including this picker design).
- **TCP API** on `47821`: `health`, `workspaces`, `open`, `init` (newline JSON).

Still to come: the model renderer (the agent prompt points at `mindmap render`, which does not
exist yet), the comments surface, remote hosts, auth on the service socket, and a systemd unit.

## Milestones

1. **Project service + auto-join** — folder → skeleton document → client opens it, no links.
2. **Comments in map mode** — one affordance: select, comment, read the thread.
3. **Agent bootstrap** — Herdr ensure/install, workspace, agent count, `/init` through the builder.
4. **Click-to-navigate** — a card with a destination moves the canvas to it (no prototype player).
