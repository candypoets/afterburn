//! The project service itself: inventory, publish-or-reuse, and Init.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};

use crate::agent::{self, AgentOutcome, Target};
use crate::comments::{self, Comment};
use crate::config::Config;
use crate::context;
use crate::error::{Error, Result, ResultExt};
use crate::herdr::{Agent, Herdr, Pane};
use crate::link;
use crate::mcp::FigcraftSession;
use crate::records::{LastSync, ProjectRecord, Store};
use crate::sync::{self, CodeDiff, JobState, MapDiff, MapSnapshot, SyncJob, SyncJobView};
use crate::time;

/// The publish poll interval and budget from the product contract.
const POLL_INTERVAL: Duration = Duration::from_millis(500);
const POLL_ATTEMPTS: usize = 60;

/// One agent as the Init picker sees it.
#[derive(Clone, Debug, Serialize)]
pub struct AgentDetail {
    /// Pane hosting the agent.
    pub pane: String,
    /// Agent kind (`codex`, `claude`, …), when the payload names one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Herdr display name, when the payload has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Lifecycle state (`idle`, `working`, `blocked`, `done`, `unknown`).
    pub status: String,
    /// Focused in the user's Herdr UI.
    pub focused: bool,
}

/// One workspace as the client sees it: Herdr state plus our map state.
#[derive(Clone, Debug, Serialize)]
pub struct WorkspaceView {
    /// Herdr workspace id.
    pub id: String,
    /// Herdr label.
    pub label: String,
    /// Checkout path, from the worktree or the focused pane's cwd.
    pub path: Option<String>,
    /// Repository name.
    pub repo: Option<String>,
    /// Whether the checkout is a linked git worktree.
    pub worktree: bool,
    /// The checked-out branch, when the path is a git checkout.
    pub branch: Option<String>,
    /// Number of recognized agents.
    pub agents: usize,
    /// Sorted, deduplicated agent kinds.
    pub agent_names: Vec<String>,
    /// Workspace-level agent status.
    pub agent_status: String,
    /// Per-agent detail for the Init picker.
    pub agents_detail: Vec<AgentDetail>,
    /// Agent CLIs installed on this host, in `AGENT_ORDER`.
    pub clis: Vec<String>,
    /// Focused in the user's Herdr UI.
    pub focused: bool,
    /// `map`, `empty`, or `none`.
    pub map: String,
    /// Published FigCraft file id, when there is a record.
    pub file: Option<String>,
    /// Number of tabs.
    pub tabs: Option<u64>,
    /// Number of panes.
    pub panes: Option<u64>,
}

/// A workspace plus its project record.
#[derive(Clone, Debug)]
pub struct Opened {
    /// The resolved workspace.
    pub workspace: WorkspaceView,
    /// The project record, always with an edit link after a successful open.
    pub record: ProjectRecord,
}

/// The result of Init: an opened project plus the agent outcome.
#[derive(Clone, Debug)]
pub struct InitResult {
    /// The project that was opened (or published).
    pub opened: Opened,
    /// What happened when ensuring an agent.
    pub agent: AgentOutcome,
}

/// The result of adopting an already-built map.
#[derive(Clone, Debug, Serialize)]
pub struct Adopted {
    /// FigCraft file id on the document server.
    pub file: String,
    /// Owner edit link (`figcraft://join?...`).
    pub link: String,
    /// Document name.
    pub name: String,
}

/// The result of Send: the agent outcome and how many comments were handed to it.
#[derive(Clone, Debug)]
pub struct SendResult {
    /// What happened when ensuring an agent and submitting the comments.
    pub agent: AgentOutcome,
    /// Number of canvas comments included in the prompt.
    pub sent: usize,
}

/// The result of Diff: the last-sync metadata and both diffs.
#[derive(Clone, Debug)]
pub struct DiffResult {
    /// Last successful sync, when there was one.
    pub last: Option<LastSync>,
    /// Design changes between the supplied snapshot and the last-synced one.
    pub map: MapDiff,
    /// Code changes since the last sync's commit.
    pub code: CodeDiff,
}

/// The result of starting a Sync: the in-flight job and the prompted agent.
#[derive(Clone, Debug)]
pub struct SyncStarted {
    /// The job the client polls with `sync_status`.
    pub job: SyncJobView,
    /// The agent that accepted the prompt.
    pub agent: AgentOutcome,
}

/// The result of one `sync_status` poll.
#[derive(Clone, Debug)]
pub struct SyncStatusResult {
    /// The job as it is now.
    pub job: SyncJobView,
    /// The workspace's last successful sync, including one just written.
    pub last: Option<LastSync>,
}

/// The stateful project service shared by the CLI and the TCP server.
#[derive(Debug)]
pub struct ProjectService {
    config: Config,
    herdr: Herdr,
    store: Store,
    create_lock: Mutex<()>,
    jobs: Mutex<BTreeMap<String, SyncJob>>,
}

impl ProjectService {
    /// Build the service, creating the state directory if needed.
    pub fn new(config: Config) -> Result<Self> {
        fs::create_dir_all(&config.data_dir).context(format!(
            "create state directory {}",
            config.data_dir.display()
        ))?;
        let store = Store::new(&config.data_dir, config.legacy_records.clone());
        let herdr = Herdr::new(config.herdr_bin.clone());
        Ok(Self {
            config,
            herdr,
            store,
            create_lock: Mutex::new(()),
            jobs: Mutex::new(BTreeMap::new()),
        })
    }

    /// Configuration accessor for logging and health.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Herdr's workspaces, enriched with agent counts and project state, sorted by label.
    pub fn workspaces(&self) -> Result<Vec<WorkspaceView>> {
        let records = self.store.all()?;
        self.workspaces_with(&records)
    }

    fn workspaces_with(
        &self,
        records: &BTreeMap<String, ProjectRecord>,
    ) -> Result<Vec<WorkspaceView>> {
        let workspaces = self.herdr.workspaces()?;
        let agents = self.herdr.agents()?;
        let clis = agent::installed_kinds();

        let mut by_workspace: BTreeMap<&str, Vec<&Agent>> = BTreeMap::new();
        for agent in &agents {
            by_workspace
                .entry(agent.workspace_id.as_str())
                .or_default()
                .push(agent);
        }

        let mut views = Vec::with_capacity(workspaces.len());
        for workspace in workspaces {
            let mine = by_workspace
                .get(workspace.workspace_id.as_str())
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let names: BTreeSet<String> = mine
                .iter()
                .filter_map(|agent| agent.agent.clone())
                .collect();
            let agents_detail = mine
                .iter()
                .filter_map(|agent| {
                    Some(AgentDetail {
                        pane: agent.pane_id.clone()?,
                        kind: agent.agent.clone(),
                        name: agent.name.clone(),
                        status: agent
                            .agent_status
                            .clone()
                            .unwrap_or_else(|| "unknown".to_string()),
                        focused: agent.focused,
                    })
                })
                .collect();
            let record = records.get(&workspace.workspace_id);
            let map = match record {
                Some(record) if record.built => "map",
                Some(_) => "empty",
                None => "none",
            };
            let worktree = workspace.worktree.as_ref();
            let path = match worktree.and_then(|tree| tree.checkout_path.clone()) {
                Some(path) => Some(path),
                // Herdr workspaces created without git (for example the repository itself) have
                // no `worktree`; their panes still know where they are.
                None => {
                    let panes = self
                        .herdr
                        .panes(&workspace.workspace_id)
                        .unwrap_or_default();
                    pane_path(&panes)
                }
            };
            let branch = path.as_deref().and_then(git_branch);
            views.push(WorkspaceView {
                id: workspace.workspace_id.clone(),
                label: workspace.label.clone(),
                path,
                repo: worktree.and_then(|tree| tree.repo_name.clone()),
                branch,
                worktree: worktree
                    .and_then(|tree| tree.is_linked_worktree)
                    .unwrap_or(false),
                agents: mine.len(),
                agent_names: names.into_iter().collect(),
                agent_status: workspace
                    .agent_status
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string()),
                agents_detail,
                clis: clis.clone(),
                focused: workspace.focused,
                map: map.to_string(),
                file: record.and_then(|record| record.file.clone()),
                tabs: workspace.tab_count,
                panes: workspace.pane_count,
            });
        }
        views.sort_by(|left, right| {
            left.label
                .to_lowercase()
                .cmp(&right.label.to_lowercase())
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(views)
    }

    /// Resolve a workspace by exact id or label, then case-insensitively.
    pub fn find_workspace(&self, query: &str) -> Result<WorkspaceView> {
        resolve_workspace(self.workspaces()?, query)
    }

    /// Resolve a workspace without importing or writing project records.
    ///
    /// `diff` only needs the Herdr label and checkout path, so it must not trigger the one-time
    /// legacy import on a fresh data directory.
    fn find_workspace_readonly(&self, query: &str) -> Result<WorkspaceView> {
        let records = self.store.all_readonly()?;
        resolve_workspace(self.workspaces_with(&records)?, query)
    }

    /// Open a workspace: reuse its published document, or publish the skeleton.
    pub fn open(&self, query: &str) -> Result<Opened> {
        let workspace = self.find_workspace(query)?;
        let record = self.ensure_project(&workspace)?;
        Ok(Opened { workspace, record })
    }

    /// Open a workspace and ensure an agent is working on it.
    ///
    /// The target is validated before the document is published, so a bad pane or kind cannot
    /// create a side effect. A failure in the agent stage itself is reported in the returned
    /// `agent` object, not as an error: the document link must remain usable.
    pub fn init(&self, query: &str, target: &Target) -> Result<InitResult> {
        let workspace = self.find_workspace(query)?;
        agent::validate_target(&self.herdr, &workspace, target)?;
        let record = self.ensure_project(&workspace)?;
        let opened = Opened { workspace, record };
        let agent = agent::ensure(
            &self.config,
            &self.herdr,
            &opened.workspace,
            opened.record.file.as_deref(),
            target,
        );
        Ok(InitResult { opened, agent })
    }

    /// Prompt an agent with canvas comments, without opening or publishing a document.
    ///
    /// The target is validated against Herdr's live agent list before any prompt or tab is
    /// created, so a bad `pane:` or `kind:` choice cannot mutate the session. The comments are
    /// rendered with the workspace label and path, then submitted through the same agent policy
    /// as [`Self::init`] (reuse an existing agent or start one). Agent failures are reported
    /// in-band in the returned outcome, exactly like Init.
    pub fn send(
        &self,
        query: &str,
        target: &Target,
        prompt: &str,
        comments: &[Comment],
        page_id: Option<u64>,
        new_page: bool,
    ) -> Result<SendResult> {
        let workspace = self.find_workspace(query)?;
        agent::validate_target(&self.herdr, &workspace, target)?;
        let record = self.store.get(&workspace.id)?;
        let page_id = page_id.or_else(|| comments.iter().find_map(|comment| comment.page_id));
        let node_ids: Vec<u64> = comments
            .iter()
            .filter_map(|comment| comment.node_id)
            .collect();
        let context = context::build(
            &self.config,
            &context::Request {
                label: &workspace.label,
                id: &workspace.id,
                path: workspace.path.as_deref(),
                file: record.as_ref().and_then(|record| record.file.as_deref()),
                edit: record.as_ref().and_then(|record| record.edit.as_deref()),
                page_id,
                new_page,
                node_ids: &node_ids,
            },
        );
        let text = comments::format_prompt(
            &workspace.label,
            workspace.path.as_deref(),
            comments,
            prompt,
            Some(&context),
        )?;
        let agent = agent::ensure_with_prompt(&self.herdr, &workspace, target, &text);
        Ok(SendResult {
            agent,
            sent: comments.len(),
        })
    }

    /// Compare a map snapshot with the last successful sync and with the repository.
    ///
    /// This is a read-only request: it reads the last snapshot through [`Store::last_sync`] (no
    /// legacy import and no write) and runs `git` without touching the repository. With no last
    /// sync every node is reported as added and the code lists stay empty.
    pub fn diff(&self, query: &str, map: &MapSnapshot) -> Result<DiffResult> {
        let workspace = self.find_workspace_readonly(query)?;
        let last = self.store.last_sync(&workspace.id)?;
        let map_diff = sync::diff_maps(map, last.as_ref().map(|last| &last.map));
        let last_commit = last.as_ref().map(|last| last.commit.as_str());
        let code = sync::code_diff(workspace.path.as_deref(), last_commit);
        Ok(DiffResult {
            last,
            map: map_diff,
            code,
        })
    }

    /// Prompt an agent to apply the design diff and catch up with the code diff.
    ///
    /// The target is preflighted before anything is prompted or started, so an unusable agent is
    /// an `ok:false` request and creates no job. The map snapshot and repository head are captured
    /// before the prompt; the returned job keeps them until [`Self::sync_status`] sees the agent
    /// finish. Nothing is written to the project record here.
    pub fn start_sync(
        &self,
        query: &str,
        target: &Target,
        note: &str,
        map: &MapSnapshot,
    ) -> Result<SyncStarted> {
        let workspace = self.find_workspace_readonly(query)?;
        agent::preflight(&self.herdr, &workspace, target)?;
        let record = self.store.get(&workspace.id)?.ok_or_else(|| {
            Error::new(format!(
                "workspace {} ({}) has no project record; open it before syncing",
                workspace.id, workspace.label
            ))
        })?;
        let last = record.last_sync;
        let map_diff = sync::diff_maps(map, last.as_ref().map(|last| &last.map));
        let last_commit = last.as_ref().map(|last| last.commit.as_str());
        let code = sync::code_diff(workspace.path.as_deref(), last_commit);
        let prompt = sync::format_prompt(
            &workspace.label,
            workspace.path.as_deref(),
            &map_diff,
            &code,
            last.is_some(),
            note,
        );

        let agent = agent::ensure_with_prompt(&self.herdr, &workspace, target, &prompt);
        if !agent.prompted {
            let reason = agent
                .reason
                .clone()
                .unwrap_or_else(|| "the agent did not accept the prompt".to_string());
            return Err(Error::new(format!(
                "sync could not prompt an agent: {reason}"
            )));
        }
        let pane = agent
            .pane
            .clone()
            .ok_or_else(|| Error::new("sync agent outcome has no pane"))?;

        let job = SyncJob {
            id: sync::new_job_id(),
            workspace: workspace.id,
            started: time::now_iso(),
            started_secs: time::now_secs(),
            head: code.head.clone(),
            map_snapshot: map.clone(),
            pane,
            agent: agent.clone(),
            state: JobState::InFlight,
            seen_active: false,
            reason: None,
        };
        let view = job.view();
        self.insert_job(job)?;
        Ok(SyncStarted { job: view, agent })
    }

    /// Poll one in-flight sync job against Herdr and finish it when the agent settles.
    ///
    /// A job is done only after Herdr has reported the agent as `working` or `blocked` and then
    /// `idle` or `done`. Finishing writes only `last_sync` (commit, completion time, start
    /// snapshot) through the store; a gone/errored agent or a job older than
    /// [`sync::JOB_TIMEOUT_SECS`] is failed with a reason and writes nothing.
    pub fn sync_status(&self, job_id: &str) -> Result<SyncStatusResult> {
        let job_id = job_id.trim();
        if job_id.is_empty() {
            return Err(Error::new("sync_status needs a non-empty \"job\" id"));
        }
        let mut job = self.get_job(job_id)?;
        if job.state != JobState::InFlight {
            return self.job_status_result(job);
        }

        let age = time::now_secs().saturating_sub(job.started_secs);
        if age >= sync::JOB_TIMEOUT_SECS {
            return self.fail_job(
                job,
                format!(
                    "sync timed out after {} minutes",
                    sync::JOB_TIMEOUT_SECS / 60
                ),
            );
        }

        let agents = self
            .herdr
            .agents()
            .context("read the agent list for sync_status")?;
        let status = agents
            .iter()
            .find(|agent| {
                agent.workspace_id == job.workspace
                    && agent.pane_id.as_deref() == Some(job.pane.as_str())
            })
            .map(|agent| agent.agent_status.as_deref().unwrap_or("unknown"));
        let next = sync::transition(job.seen_active, status, age);
        job.seen_active = next.seen_active;

        match next.state {
            JobState::InFlight => {
                self.save_job(&job)?;
                self.job_status_result(job)
            }
            JobState::Failed => {
                let reason = next.reason.unwrap_or_else(|| "sync failed".to_string());
                self.fail_job(job, reason)
            }
            JobState::Done => {
                let last = LastSync {
                    commit: job.head.clone().unwrap_or_default(),
                    at: time::now_iso(),
                    map: job.map_snapshot.clone(),
                };
                match self.store.set_last_sync(&job.workspace, last)? {
                    Some(record) => {
                        job.state = JobState::Done;
                        job.reason = None;
                        self.save_job(&job)?;
                        Ok(SyncStatusResult {
                            job: job.view(),
                            last: record.last_sync,
                        })
                    }
                    None => self.fail_job(job, "project record no longer exists".to_string()),
                }
            }
        }
    }

    fn get_job(&self, id: &str) -> Result<SyncJob> {
        let jobs = self
            .jobs
            .lock()
            .map_err(|_| Error::new("sync job lock poisoned"))?;
        jobs.get(id)
            .cloned()
            .ok_or_else(|| Error::new(format!("unknown sync job {id:?}")))
    }

    fn insert_job(&self, job: SyncJob) -> Result<()> {
        let mut jobs = self
            .jobs
            .lock()
            .map_err(|_| Error::new("sync job lock poisoned"))?;
        jobs.insert(job.id.clone(), job);
        Ok(())
    }

    fn save_job(&self, job: &SyncJob) -> Result<()> {
        let mut jobs = self
            .jobs
            .lock()
            .map_err(|_| Error::new("sync job lock poisoned"))?;
        jobs.insert(job.id.clone(), job.clone());
        Ok(())
    }

    fn fail_job(&self, mut job: SyncJob, reason: String) -> Result<SyncStatusResult> {
        job.state = JobState::Failed;
        job.reason = Some(reason);
        self.save_job(&job)?;
        self.job_status_result(job)
    }

    fn job_status_result(&self, job: SyncJob) -> Result<SyncStatusResult> {
        let last = self
            .store
            .get(&job.workspace)?
            .and_then(|record| record.last_sync);
        Ok(SyncStatusResult {
            job: job.view(),
            last,
        })
    }

    /// Publish an already-built `.figcraft` file as a workspace's map.
    ///
    /// Unlike [`Self::init`], this never touches an agent: it is the path for seeding a workspace
    /// whose map was drawn outside the service. The stored record is a normal one except that
    /// `built` is true.
    pub fn adopt(&self, query: &str, path: &Path, name: Option<&str>) -> Result<Adopted> {
        let workspace = self.find_workspace(query)?;
        let default_name = format!("Mindmap · {}", workspace.label);
        let name = name
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| default_name.clone());

        let _guard = self
            .create_lock
            .lock()
            .map_err(|_| Error::new("project creation lock poisoned"))?;
        let record = self.publish_map(&workspace, path, name, true)?;
        self.store.upsert(record.clone())?;
        Ok(Adopted {
            file: record
                .file
                .ok_or_else(|| Error::new("adopted map has no file id"))?,
            link: record
                .edit
                .ok_or_else(|| Error::new("adopted map has no edit link"))?,
            name: record.name.unwrap_or(default_name),
        })
    }

    fn ensure_project(&self, workspace: &WorkspaceView) -> Result<ProjectRecord> {
        let _guard = self
            .create_lock
            .lock()
            .map_err(|_| Error::new("project creation lock poisoned"))?;
        if let Some(record) = self.store.get(&workspace.id)? {
            if record.has_link() {
                return Ok(record);
            }
        }
        let record = self.publish_skeleton(workspace)?;
        self.store.upsert(record.clone())?;
        Ok(record)
    }

    fn publish_skeleton(&self, workspace: &WorkspaceView) -> Result<ProjectRecord> {
        self.publish_map(
            workspace,
            &self.config.empty_template,
            format!("Mindmap · {}", workspace.label),
            false,
        )
    }

    fn publish_map(
        &self,
        workspace: &WorkspaceView,
        source: &Path,
        name: String,
        built: bool,
    ) -> Result<ProjectRecord> {
        if !source.is_file() {
            let what = if built {
                "map file"
            } else {
                "skeleton template"
            };
            return Err(Error::new(format!(
                "{what} not found at {}",
                source.display()
            )));
        }
        let key = fs::read_to_string(&self.config.key_file).context(format!(
            "read server key {}",
            self.config.key_file.display()
        ))?;
        let key = key.trim();
        if key.is_empty() {
            return Err(Error::new(format!(
                "server key {} is empty",
                self.config.key_file.display()
            )));
        }

        let description = if built { "map" } else { "skeleton" };
        eprintln!(
            "mindmap-service: publishing {description} {} for {} ({})",
            source.display(),
            workspace.label,
            workspace.id
        );

        let mut session = FigcraftSession::start(&self.config.figcraft_cli)?;
        session.open_document(source)?;
        session.run_command(
            "collab.publish",
            json!({
                "server": self.config.publish_server_url(key),
                "name": name.clone(),
            }),
        )?;
        let peers = self.wait_until_live(&mut session)?;

        let published = peers.get("published").filter(|value| value.is_object());
        let edit = published
            .and_then(|value| value.get("edit"))
            .and_then(Value::as_str)
            .or_else(|| peers.get("link").and_then(Value::as_str))
            .ok_or_else(|| Error::new("publish returned no edit link"))?;
        let file = published
            .and_then(|value| value.get("file"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| link::query_param(edit, "file"))
            .ok_or_else(|| Error::new("publish returned no file id"))?;
        let view = published
            .and_then(|value| value.get("view"))
            .and_then(Value::as_str);
        let published_name = published
            .and_then(|value| value.get("name"))
            .and_then(Value::as_str)
            .map(str::to_owned);

        let edit = link::rewrite_host(edit, &self.config.public_server)?;
        let view = match view {
            Some(view) => Some(link::rewrite_host(view, &self.config.public_server)?),
            None => None,
        };

        let record = ProjectRecord {
            id: workspace.id.clone(),
            label: workspace.label.clone(),
            path: workspace.path.clone(),
            file: Some(file.clone()),
            edit: Some(edit),
            view,
            name: Some(published_name.unwrap_or(name)),
            built,
            created: time::now_iso(),
            last_sync: None,
            extra: BTreeMap::new(),
        };
        eprintln!("mindmap-service: published {} as {}", workspace.label, file);
        Ok(record)
    }

    fn wait_until_live(&self, session: &mut FigcraftSession) -> Result<Value> {
        let mut last_status = "unknown".to_string();
        let mut last_reason = String::new();
        for _ in 0..POLL_ATTEMPTS {
            thread::sleep(POLL_INTERVAL);
            let peers = session.run_command("collab.peers", json!({}))?;
            let status = peers
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            if status == "live" {
                return Ok(peers);
            }
            last_status = status.to_string();
            if let Some(reason) = peers.get("reason").and_then(Value::as_str) {
                if !reason.is_empty() {
                    last_reason = reason.to_string();
                }
            }
            if status == "failed" {
                break;
            }
        }
        let reason = if last_reason.is_empty() {
            String::new()
        } else {
            format!(": {last_reason}")
        };
        Err(Error::new(format!(
            "publish did not become live (last status {last_status}){reason}"
        )))
    }
}

/// Resolve a workspace by exact id or label, then case-insensitively.
fn resolve_workspace(workspaces: Vec<WorkspaceView>, query: &str) -> Result<WorkspaceView> {
    if let Some(found) = workspaces
        .iter()
        .find(|workspace| workspace.id == query || workspace.label == query)
    {
        return Ok(found.clone());
    }
    let original = query;
    let query = original.to_lowercase();
    workspaces
        .into_iter()
        .find(|workspace| {
            workspace.label.to_lowercase() == query || workspace.id.to_lowercase() == query
        })
        .ok_or_else(|| Error::new(format!("no workspace {original:?}")))
}

/// The focused pane's cwd, else the first pane that knows one.
///
/// Herdr workspaces without a `worktree` have no checkout path, but every pane reports the
/// directory it was opened in. That is the best available answer for the map root, and for the
/// repository workspace it is exactly the repository root.
fn pane_path(panes: &[Pane]) -> Option<String> {
    panes
        .iter()
        .find(|pane| pane.focused)
        .and_then(|pane| pane.cwd.clone())
        .filter(|cwd| !cwd.trim().is_empty())
        .or_else(|| {
            panes
                .iter()
                .filter_map(|pane| pane.cwd.clone())
                .find(|cwd| !cwd.trim().is_empty())
        })
}

/// The checked-out branch of `path`, or `None` when it is not a git checkout.
fn git_branch(path: &str) -> Option<String> {
    let output = Command::new("git")
        .args(["-C", path, "rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!branch.is_empty() && branch != "HEAD").then_some(branch)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(pane_id: &str, cwd: Option<&str>, focused: bool) -> Pane {
        Pane {
            pane_id: pane_id.into(),
            tab_id: None,
            cwd: cwd.map(str::to_owned),
            foreground_cwd: None,
            focused,
        }
    }

    #[test]
    fn pane_path_prefers_the_focused_pane() {
        let panes = [
            pane("wX:p1", Some("/root/code/mindmap"), false),
            pane("wX:p2", Some("/root/code/alzette"), true),
        ];
        assert_eq!(pane_path(&panes).as_deref(), Some("/root/code/alzette"));
    }

    #[test]
    fn pane_path_falls_back_to_the_first_pane_with_a_cwd() {
        let panes = [
            pane("wX:p1", None, false),
            pane("wX:p2", Some("/root/code/mindmap"), false),
        ];
        assert_eq!(pane_path(&panes).as_deref(), Some("/root/code/mindmap"));
        assert_eq!(pane_path(&[]), None);
    }
}
