//! The workspace shapes the node reports for Herdr workspaces and agents. The client reads this
//! JSON for its workspace list, Init picker and agent rows.

use serde::Serialize;

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
    /// Number of tabs.
    pub tabs: Option<u64>,
    /// Number of panes.
    pub panes: Option<u64>,
}
