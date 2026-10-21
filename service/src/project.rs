//! The project service itself: inventory, publish-or-reuse, and Init.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};

use crate::agent::{self, AgentOutcome};
use crate::config::Config;
use crate::error::{Error, Result, ResultExt};
use crate::herdr::{Agent, Herdr};
use crate::link;
use crate::mcp::FigcraftSession;
use crate::records::{ProjectRecord, Store};
use crate::time;

/// The publish poll interval and budget from the product contract.
const POLL_INTERVAL: Duration = Duration::from_millis(500);
const POLL_ATTEMPTS: usize = 60;

/// One workspace as the client sees it: Herdr state plus our map state.
#[derive(Clone, Debug, Serialize)]
pub struct WorkspaceView {
    /// Herdr workspace id.
    pub id: String,
    /// Herdr label.
    pub label: String,
    /// Checkout path, when the workspace is a repository.
    pub path: Option<String>,
    /// Repository name.
    pub repo: Option<String>,
    /// Whether the checkout is a linked git worktree.
    pub worktree: bool,
    /// Number of recognized agents.
    pub agents: usize,
    /// Sorted, deduplicated agent kinds.
    pub agent_names: Vec<String>,
    /// Workspace-level agent status.
    pub agent_status: String,
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

/// The stateful project service shared by the CLI and the TCP server.
#[derive(Debug)]
pub struct ProjectService {
    config: Config,
    herdr: Herdr,
    store: Store,
    create_lock: Mutex<()>,
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
        })
    }

    /// Configuration accessor for logging and health.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Herdr's workspaces, enriched with agent counts and project state, sorted by label.
    pub fn workspaces(&self) -> Result<Vec<WorkspaceView>> {
        let workspaces = self.herdr.workspaces()?;
        let agents = self.herdr.agents()?;
        let records = self.store.all()?;

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
            let record = records.get(&workspace.workspace_id);
            let map = match record {
                Some(record) if record.built => "map",
                Some(_) => "empty",
                None => "none",
            };
            let worktree = workspace.worktree.as_ref();
            views.push(WorkspaceView {
                id: workspace.workspace_id.clone(),
                label: workspace.label.clone(),
                path: worktree.and_then(|tree| tree.checkout_path.clone()),
                repo: worktree.and_then(|tree| tree.repo_name.clone()),
                worktree: worktree
                    .and_then(|tree| tree.is_linked_worktree)
                    .unwrap_or(false),
                agents: mine.len(),
                agent_names: names.into_iter().collect(),
                agent_status: workspace
                    .agent_status
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string()),
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
        let workspaces = self.workspaces()?;
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

    /// Open a workspace: reuse its published document, or publish the skeleton.
    pub fn open(&self, query: &str) -> Result<Opened> {
        let workspace = self.find_workspace(query)?;
        let record = self.ensure_project(&workspace)?;
        Ok(Opened { workspace, record })
    }

    /// Open a workspace and ensure an agent is working on it.
    ///
    /// A failure in the agent stage is reported in the returned `agent` object, not as an error:
    /// the document link must remain usable.
    pub fn init(&self, query: &str) -> Result<InitResult> {
        let opened = self.open(query)?;
        let agent = agent::ensure(
            &self.config,
            &self.herdr,
            &opened.workspace,
            opened.record.file.as_deref(),
        );
        Ok(InitResult { opened, agent })
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
        if !self.config.empty_template.is_file() {
            return Err(Error::new(format!(
                "skeleton template not found at {}",
                self.config.empty_template.display()
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

        let name = format!("Mindmap · {}", workspace.label);
        eprintln!(
            "mindmap-service: publishing skeleton for {} ({})",
            workspace.label, workspace.id
        );

        let mut session = FigcraftSession::start(&self.config.figcraft_cli)?;
        session.open_document(&self.config.empty_template)?;
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
            built: false,
            created: time::now_iso(),
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
