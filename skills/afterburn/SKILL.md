---
name: afterburn
description: Maintain live repository designs through a local Afterburn node and implement scoped design changes with Git-backed Sync.
---

# Afterburn

The node on the repository's machine owns the live design. Git owns its committed copy at
`.afterburn/map.figcraft`. Agents edit the session; only the node writes that file. No owner
links, server keys or `projects.json` records are needed.

## Connect and inspect

Start the stdio MCP adapter on the machine that owns the worktree:

```sh
afterburn-node mcp --path /absolute/worktree
```

Tools: `inspect_document`, `get_node`, `create_node`, `set_properties`, `run_command`,
`list_commands`, `screenshot`, `add_video`, `apply_edit`, `video_frames`, `video_probe`, `node_request`. Discover command parameters with `list_commands`.
`run_command` applies a FigCraft command through the authoritative session and broadcasts the
changes. Use `command.batch` for selection-dependent steps. Inspect before and after editing, and
look at your work with `screenshot` (a PNG of layers or a page; it writes nothing).

The CLI also exposes the protocol:

```sh
afterburn-node request '{"method":"open","path":"/absolute/worktree"}'
afterburn-node request '{"method":"session.inspect","session":"SESSION","depth":8}'
afterburn-node request '{"method":"diff","session":"SESSION","page":123,"offset":0,"limit":200}'
afterburn-node request '{"method":"session.command","session":"SESSION","command":"node.set","params":{"ids":[123],"name":"New name"}}'
```

Use the actual session ID returned by `open`. `--address` selects a node reached through an
existing SSH tunnel; the default is `127.0.0.1:47822`. The CLI adds the protocol version and
request ID. After checking out another branch, reopen the worktree to use its session.

## Comments and scope

Context identifies the worktree, branch, session and the page in scope. Each comment names its
card (`On Card (id N) · path`); a selected card no comment names gets a `Selected:` line. Nothing
else is listed: read the page with `get_node <page>` (its cards, with ids and places) and any
layer with `get_node <id>`. Read their
live content, then address the request there. A page-scoped job covers that page and its subpages;
other pages are out of scope. Preserve card names and page IDs unless the task requires a
change. Use existing components and layout conventions. Do not rebuild the whole map for a
local edit.

## Video

`add_video {source, parent, x, y, width?, name?}` (node method `media.add`) puts a clip on the
canvas. `source` is the clip's absolute path on the node's machine; `parent` is the page or card
id. The node copies the clip to `.afterburn/media/<hash>.mp4` and adds a card filled with its
first frame; the app plays it in place (play button, pause, seek). The app decodes Constrained
Baseline H.264 in MP4; any other clip is converted with ffmpeg when the node's machine has it,
else refused. Screenshots and exports show the first frame only. Move or resize the card as any
layer; keep its `afterburn.video` plugin data. Commit the clip's file with the map, or other
machines' checkouts get a card whose clip is missing.

## Video editing

An edit is a file, `.afterburn/edits/<name>.json`: a cut list. It is the only truth; the canvas
shows it. Send the whole cut list with `apply_edit` (node method `edit.apply`):

```json
{"name": "intro", "width": 1280, "height": 720, "fps": 30, "target_seconds": 30,
 "clips": [{"id": "hook", "source": "footage/map.mp4", "in": 2.0, "out": 7.0, "title": "Hook"},
           {"id": "comment", "source": "footage/comment.mp4", "in": 0, "out": 6, "crossfade": 0.5}],
 "voice": [{"id": "v1", "source": "voice/hook.wav", "at": 0.5, "text": "Your code, on one canvas."}],
 "captions": [{"text": "Your code, on one canvas.", "at": 0.5, "until": 3.0, "voice": "v1"}],
 "music": {"source": "music/bed.wav", "gain_db": -22}}
```

Sources are files in the worktree; times are seconds. The node checks the list (sources exist,
cuts fit their sources, the length is `target_seconds` within one frame, a `crossfade` is the only
overlap, voice segments do not overlap, a caption sits inside the voice it names), renders it with
ffmpeg, measures the render and redraws the edit's frame: the playable take and a timeline
picture. A refused edit lists every reason; fix them all and send again. Pass `parent`, `x`, `y`
the first time only. Never move, restyle or delete the frame's layers: they are locked, and keep
their ids from one apply to the next so comments stay on their shot. Each apply replaces the
take's file; commit the edit file, its sources and the kept take with the map.

You cannot watch or hear a clip. `video_frames {node, times}` (`media.frames`) returns a contact
sheet of frames at those seconds; `video_probe {node}` (`media.probe`) returns length, loudness
(LUFS), seconds of silence and of black. Check a cut with both before saying it works.

## Page modes

A page is the **state of the art** (it describes the code as it is) or an **exploration** (a new
idea the code does not have yet). When a prompt creates a page, mark its mode before designing:
an exploration gets `node.set {"ids":[PAGE],"pluginData":{"afterburn.page_mode":"exploration"}}`;
the state of the art stays unmarked. Comments never change a page's mode. A Sync of an
exploration builds it; once that Sync finishes, the page is the state of the art. The mode is
never a waiting change.

## Show, don't tell

The canvas is the medium. An image is worth a thousand words: design one. Carry each idea with a
rich diagram in whatever form it needs (structure, flow, chart, comparison, several at once) and
as few words as possible. Words only label the parts and source them (file:line). No shape is
prescribed; a wall of text cards is the thing to avoid. A design is its own picture: do not add a
diagram that explains it. Check the result with `screenshot`.

Every factual card needs a repository source. Read the implementation before claiming behavior.
Keep one claim per card and explain it plainly. Label proposals as proposals. Do not edit
`.afterburn/map.figcraft` directly or use the obsolete model renderer for live jobs.

## Three kinds of comment work

A comment's prompt ends with `HOW TO FINISH`: the exact `record` command for its request. Some
cards on a page may be design edits still waiting for Sync; `diff` lists them when it matters
(follow pagination). Decide the kind before changing anything and start the reply with one line: `Kind: explanation`, `Kind: bug fix`, `Kind: design edit` or
`Kind: bug fix + design edit`.

- **Explanation**: a question, or the map should describe how the app works today. Read the
  code; change only the map, only as asked, and source every claim (file:line). Change no code.
  Record it as `explain`.
- **Bug fix**: the app is broken today. Fix and test the code; correct cards that described the
  bug wrongly. Record it as `fix`.
- **Design edit**: propose behaviour the code does not have, on the map only, labelled as a
  proposal. Do not change code, record or commit: it waits for Sync.
- **Unsure** is a design edit (it waits instead of disappearing). **Mixed**: do and record the
  fix, and leave the proposal as a waiting design edit.

Waiting cards are proposals, not current behaviour; do not describe them as what the code does.

To record, after the last map edit of the request:

```sh
afterburn-node request '{"method":"record","session":"SESSION","request":"REQUEST","kind":"explain","page":123}'
# explain: commit only the staged map; fix: commit the code with it. Trailer last.
# Cannot commit? {"method":"sync.cancel","job":"JOB"}: an open record blocks Sync.
afterburn-node request '{"method":"sync.finish","job":"JOB","commit":"SHA"}'
```

`record` commits the last recorded map plus only this request's edits: design edits that were
waiting stay out. Where an edit overlaps a waiting edit it is not recorded and keeps waiting;
the reply lists those cards (`kept_waiting`). Without an earlier record for the page, the page
as the request found it is recorded too, which declares it current. Init is one big
explanation and ends with a `record` of the whole map. `record.start` keeps a starting point for
work that did not come from a comment. An open record blocks Sync; `sync.cancel` releases it.
`afterburn-node` adds your Herdr pane to `record`, so the node cancels the record if you exit, and
the Sync panel shows it (kind, agent, age) with a Cancel button.

## Sync implements the design

Sync freezes the authoritative design at job start. Its prompt lists the waiting design changes
and the code committed since the last Sync. The design baseline is the last reachable record of
any kind (`Afterburn-Sync:` trailer ending in nothing, `explain` or `fix`); the code baseline is
the last Sync. A page-specific marker does not declare other pages synced. The node compares all FigCraft properties, including geometry,
arrows, styles and document-level data. Follow `diff` pagination to read the entire change list.
Read code drift with Git from that commit; the prompt is an entry point, not all the evidence.

Plan large work and split independent parts across sub-agents in your own harness. Implement,
integrate, test and deploy everything authorized before stopping. Keep scope boundaries when
delegating. An idle agent is not proof that the design was implemented.

After implementation and verification:

1. Call `sync.prepare` with the job ID. The node gates file saves, writes the frozen start design
   into the worktree and stages its exact Git blob. Live edits continue in the session/cache.
2. Stage the implementation files. Inspect the index, then commit code and
   `.afterburn/map.figcraft` together with the **exact trailer returned by `sync.prepare`**. Do not
   invent a marker or mark unrelated pages complete. The trailer contains job ID and scope.
3. Call `sync.finish` with job ID and commit SHA. The node verifies the frozen blob, branch,
   ancestry and trailer before reporting success. It then saves the current live design; edits
   made during the job remain uncommitted drift for the next sync.

```sh
afterburn-node request '{"method":"sync.prepare","job":"JOB"}'
# Stage implementation files and commit with the returned trailer.
afterburn-node request '{"method":"sync.finish","job":"JOB","commit":"SHA"}'
```

Call `sync.cancel` to release the save gate without declaring success if the job is cancelled.
Never overwrite the live file, substitute a newer design into the frozen commit, or clear drift
by updating metadata. Cache files stay under the worktree's Git directory and are not committed.
Never copy legacy owner links into source, another agent's prompt, commits or reports.

## Branches and merges

The committed design travels with Git push/fetch. `merge.plan` uses Git's merge base and
FigCraft's three-way property merge. Review conflicts and supply explicit per-key picks to
`merge.apply`; never use a text merge driver for the binary design. Resolve external-file
conflicts with `session.resolve`, preserving both sides until the user or task supplies a pick.
