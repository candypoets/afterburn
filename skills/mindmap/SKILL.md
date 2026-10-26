---
name: mindmap
description: Maintain live repository designs through a local Mindmap node and implement scoped design changes with Git-backed Sync.
---

# Mindmap

The node on the repository's machine owns the live design. Git owns its committed copy at
`.mindmap/map.figcraft`. Agents edit the session; only the node writes that file. No owner
links, server keys or `projects.json` records are needed.

## Connect and inspect

Start the stdio MCP adapter on the machine that owns the worktree:

```sh
mindmap-node mcp --path /absolute/worktree
```

Tools: `inspect_document`, `get_node`, `create_node`, `set_properties`, `run_command`,
`list_commands`, `screenshot`, `node_request`. Discover command parameters with `list_commands`.
`run_command` applies a FigCraft command through the authoritative session and broadcasts the
changes. Use `command.batch` for selection-dependent steps. Inspect before and after editing, and
look at your work with `screenshot` (a PNG of layers or a page; it writes nothing).

The CLI also exposes the protocol:

```sh
mindmap-node request '{"method":"open","path":"/absolute/worktree"}'
mindmap-node request '{"method":"session.inspect","session":"SESSION","depth":8}'
mindmap-node request '{"method":"diff","session":"SESSION","page":123,"offset":0,"limit":200}'
mindmap-node request '{"method":"session.command","session":"SESSION","command":"node.set","params":{"ids":[123],"name":"New name"}}'
```

Use the actual session ID returned by `open`. `--address` selects a node reached through an
existing SSH tunnel; the default is `127.0.0.1:47822`. The CLI adds the protocol version and
request ID. After checking out another branch, reopen the worktree to use its session.

## Comments and scope

Context identifies the worktree, branch, session, page and selected cards. Read their live
content, then address the request there. A page-scoped job covers that page and its subpages;
other pages are out of scope. Preserve card names and page IDs unless the task requires a
change. Use existing components and layout conventions. Do not rebuild the whole map for a
local edit.

Every factual card needs a repository source. Read the implementation before claiming behavior.
Keep one claim per card and explain it plainly. Label proposals as proposals. Do not edit
`.mindmap/map.figcraft` directly or use the obsolete model renderer for live jobs.

## Three kinds of comment work

A comment's prompt carries a `REQUEST` section (its request id, the page's last record and the
design edits still waiting for Sync) and says how to finish. Decide the kind first and start the
reply with `Kind: …`:

- **Explanation**: describe today's code on the map. Change no code. Record it as `explain`.
- **Bug fix**: fix and test the code; correct cards that described the bug wrongly. Record it as
  `fix`.
- **Design edit**: propose new behaviour on the map only, labelled as a proposal. Do not record
  or commit: it waits for Sync. When unsure, it is a design edit. A mixed request is split.

Waiting cards are proposals, not current behaviour; do not describe them as what the code does.

To record, after the last map edit of the request:

```sh
mindmap-node request '{"method":"record","session":"SESSION","request":"REQUEST","kind":"explain","page":123}'
# explain: commit only the staged map; fix: commit the code with it. Trailer last.
mindmap-node request '{"method":"sync.finish","job":"JOB","commit":"SHA"}'
```

`record` commits the last recorded map plus only this request's edits: design edits that were
waiting stay out. Where an edit overlaps a waiting edit it is not recorded and keeps waiting;
the reply lists those cards (`kept_waiting`). Without an earlier record for the page, the page
as the request found it is recorded too, which declares it current. Init is one big
explanation and ends with a `record` of the whole map. `record.start` keeps a starting point for
work that did not come from a comment. An open record blocks Sync; `sync.cancel` releases it.
`mindmap-node` adds your Herdr pane to `record`, so the node cancels the record if you exit, and
the Sync panel shows it (kind, agent, age) with a Cancel button.

## Sync implements the design

Sync freezes the authoritative design at job start. Its prompt lists the waiting design changes
and the code committed since the last Sync. The design baseline is the last reachable record of
any kind (`Mindmap-Sync:` trailer ending in nothing, `explain` or `fix`); the code baseline is
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
   `.mindmap/map.figcraft` together with the **exact trailer returned by `sync.prepare`**. Do not
   invent a marker or mark unrelated pages complete. The trailer contains job ID and scope.
3. Call `sync.finish` with job ID and commit SHA. The node verifies the frozen blob, branch,
   ancestry and trailer before reporting success. It then saves the current live design; edits
   made during the job remain uncommitted drift for the next sync.

```sh
mindmap-node request '{"method":"sync.prepare","job":"JOB"}'
# Stage implementation files and commit with the returned trailer.
mindmap-node request '{"method":"sync.finish","job":"JOB","commit":"SHA"}'
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
