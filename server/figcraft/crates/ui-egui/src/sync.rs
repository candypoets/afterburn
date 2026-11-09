//! Sync: the map's drift from the last sync, the code drift since it, and one action.
//!
//! The UI calls a Sync a Burn (design: `Mindmap page · Afterburn`): the button, the sidebar and
//! its action say Burn and show the burning disc ([`crate::burn_disc`]); the code, the command
//! ids and the node protocol keep the name Sync.
//!
//! A sync covers the current page and its subpages by default, or every page. A page sync is
//! compared with each page's own last sync, and its code side is limited to the paths the page's
//! cards cite; the service keeps one baseline per page.
//!
//! The header toggle opens a full-height docked sidebar with the
//! cards waiting (cards, not layers: the node groups every changed layer under its card), the code
//! that changed since the last sync folded to one line, and a sticky footer with the agent target,
//! an optional note and the action. Every row takes you there: a click, its → or `[` / `]` opens
//! the card's page and fits the card in the view, outlined, while every waiting card on the canvas
//! wears its `+` / `~` badge. A row's checkbox leaves the card out: the node freezes the last
//! recorded map plus only the included cards' edits, and the rest keep waiting for the next Sync.
//! A sync runs on the node; while it is in flight the
//! widget (and the button, when the widget is closed) shows the flight, the map stays editable, and
//! a thread polls `sync_status` once a second until the service reports `done` or `failed`.
//!
//! Every request is newline JSON over plain TCP on a named thread of its own. The snapshot is built
//! from the open document each time a diff or sync is asked for, so the widget never talks about a
//! map the human is no longer looking at.

use std::collections::HashSet;
use std::sync::mpsc::{self, Receiver};

use egui::{Align2, Color32, CornerRadius, FontId, Id, Key, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use figcraft_doc::{Document, NodeId, NodeKind};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::FigApp;
use crate::a11y;
use crate::agent_menu;
use crate::burn_disc;
use crate::compose::TargetInfo;
use crate::map_colors::MapColors;
use crate::theme::{regular, semibold};
use crate::widgets;
use crate::workspaces::{self, Target};

#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;

/// The most pages one snapshot may carry (the service rejects more).
const MAX_PAGES: usize = 500;
/// The most nodes one snapshot may carry (the service rejects more).
const MAX_NODES: usize = 5_000;
/// The note is kept well under the service's prompt limit.
const MAX_NOTE_CHARS: usize = 2_000;
/// The widget's inner padding.
const PAD: f32 = 19.0;
/// How many scope paths and files the code section shows before `+N more`.
const FILES: usize = 4;

/// The map palette of the current theme (see `map_colors`).
fn c() -> MapColors {
    crate::map_colors::get()
}

/// `classic` on the Classic theme (Mindmap's look until now), `themed` on the others.
fn classic_or(classic: Color32, themed: Color32) -> Color32 {
    if crate::map_colors::theme() == crate::map_colors::Theme::Classic { classic } else { themed }
}

/// The current row's fill: Classic's warm grey, the theme's soft fill otherwise.
fn row_hot() -> Color32 {
    classic_or(Color32::from_rgb(0xF1, 0xEF, 0xEC), c().soft)
}
/// A row's height, and one detail line's under the current row.
const ROW_H: f32 = 42.0;
const DETAIL_H: f32 = 18.0;
/// The most detail lines the current row lists before `+N more`.
const DETAILS: usize = 8;

/// A diff runs `git` on the project host and may read a large repository.
#[cfg(not(target_arch = "wasm32"))]
const DIFF_TIMEOUT: Duration = Duration::from_secs(30);
/// Starting a sync may have to start an agent and accept a first-run prompt.
#[cfg(not(target_arch = "wasm32"))]
const SYNC_TIMEOUT: Duration = Duration::from_secs(120);
/// One status poll asks Herdr; it is quick, but the host may be busy.
#[cfg(not(target_arch = "wasm32"))]
const STATUS_TIMEOUT: Duration = Duration::from_secs(15);
/// A reply larger than this is refused rather than read into memory.
/// How often the status thread asks while a job is in flight.
#[cfg(not(target_arch = "wasm32"))]
const POLL_EVERY: Duration = Duration::from_secs(1);
/// Network errors in a row before the poll gives up and says so.
#[cfg(not(target_arch = "wasm32"))]
const POLL_MISSES: u8 = 3;

/// The snapshot's page: what the service stores and compares.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct SnapPage {
    pub id: u64,
    pub name: String,
    /// The parent page of a subpage (`mindmap.parent_page`), so the service knows the tree.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<u64>,
}

/// The snapshot's node: the card as the map shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct SnapNode {
    pub id: u64,
    pub name: String,
    pub parent: Option<u64>,
    pub page: u64,
    pub kind: String,
    pub text: String,
}

/// The map as it is now: pages and every card below them, depth first.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Snapshot {
    pub pages: Vec<SnapPage>,
    pub nodes: Vec<SnapNode>,
}

impl Snapshot {
    /// Walk `doc.pages`, then each page's children, capping at the service's limits.
    ///
    /// The walk is iterative and remembers visited nodes, so a malformed document with a cycle
    /// cannot loop or overflow the stack. Pages and cards beyond the caps are left out (the
    /// service rejects larger snapshots outright).
    pub fn of(doc: &Document) -> Snapshot {
        Snapshot::of_pages(doc, None)
    }

    /// Like [`Snapshot::of`], limited to `page` and its subpages when one is given.
    pub fn of_pages(doc: &Document, page: Option<NodeId>) -> Snapshot {
        let scope: Option<HashSet<NodeId>> = page.map(|page| {
            let mut pages: HashSet<NodeId> = crate::page_tree::descendants(doc, page).into_iter().collect();
            pages.insert(page);
            pages
        });
        let mut pages = Vec::new();
        let mut nodes = Vec::new();
        // The map's cards are its top-level frames. Nested frames are the card's internal layout,
        // and text layers are its body: both travel as the card's text, so the diff talks about
        // cards the human named, not every text layer in the document.
        for page_id in &doc.pages {
            if pages.len() >= MAX_PAGES || nodes.len() >= MAX_NODES {
                break;
            }
            if scope.as_ref().is_some_and(|scope| !scope.contains(page_id)) {
                continue;
            }
            let Some(page) = doc.node(*page_id) else { continue };
            let parent = crate::page_tree::parent(doc, *page_id).map(|parent| parent.0);
            pages.push(SnapPage { id: page_id.0, name: page.name.clone(), parent });
            for child in &page.children {
                if nodes.len() >= MAX_NODES {
                    break;
                }
                let Some(node) = doc.node(*child) else { continue };
                if !is_card(&node.kind) {
                    continue;
                }
                nodes.push(SnapNode {
                    id: child.0,
                    name: node.name.clone(),
                    parent: None,
                    page: page_id.0,
                    kind: kind_name(&node.kind).to_string(),
                    text: card_text(doc, *child),
                });
            }
        }
        Snapshot { pages, nodes }
    }
    /// Whether anything would differ between the two snapshots.
    pub fn differs(&self, other: &Snapshot) -> bool {
        self != other
    }
}

fn is_card(kind: &NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Frame
            | NodeKind::Group
            | NodeKind::Section { .. }
            | NodeKind::Component { .. }
            | NodeKind::ComponentSet { .. }
            | NodeKind::Instance { .. }
    )
}

/// Every text layer below `root`, joined; the card's body for the diff.
fn card_text(doc: &Document, root: NodeId) -> String {
    let mut out = String::new();
    let mut stack = vec![root];
    let mut seen = HashSet::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Some(node) = doc.node(id) else { continue };
        if let NodeKind::Text(text) = &node.kind {
            let part = text.characters.trim();
            if !part.is_empty() {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(part);
                if out.len() >= 4_000 {
                    out.truncate(4_000);
                    break;
                }
            }
        }
        for child in node.children.iter().rev() {
            stack.push(*child);
        }
    }
    out
}

/// The short kind name the service's snapshot stores.
fn kind_name(kind: &NodeKind) -> &'static str {
    match kind {
        NodeKind::Frame => "frame",
        NodeKind::Text(_) => "text",
        NodeKind::Group => "group",
        NodeKind::Rectangle => "rectangle",
        NodeKind::Ellipse { .. } => "ellipse",
        NodeKind::Vector { .. } => "vector",
        NodeKind::Component { .. } | NodeKind::ComponentSet { .. } => "component",
        NodeKind::Instance { .. } => "instance",
        _ => "other",
    }
}

/// The last successful sync, as the service reports it.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Last {
    #[serde(default)]
    pub exists: bool,
    #[serde(default)]
    pub commit: String,
    #[serde(default)]
    pub at: String,
}

/// Deserialize `last` as `None` when the service says it does not exist.
fn last_or_none<'de, D>(deserializer: D) -> Result<Option<Last>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let last = Option::<Last>::deserialize(deserializer)?;
    Ok(last.and_then(|last| last.exists.then_some(last)))
}

/// One added or removed card in the design diff.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct MapCard {
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub page: String,
    /// The layer's parent, so only the top-most of new or removed layers stands for a card.
    #[serde(default)]
    pub parent: Option<u64>,
}

/// One changed card in the design diff.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct MapChange {
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub fields: Vec<String>,
}

/// One changed layer inside an edited card.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct LayerChange {
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub name: String,
    /// `added`, `changed` or `removed`.
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub parent: Option<u64>,
    #[serde(default)]
    pub fields: Vec<String>,
}

/// One waiting card: a top-level layer on its page with every added, changed or removed layer
/// inside it, as the node groups them.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct WaitingCard {
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub name: String,
    /// The page's name.
    #[serde(default)]
    pub page: String,
    #[serde(default)]
    pub page_id: Option<u64>,
    /// `added`, `changed` or `removed`.
    #[serde(default)]
    pub state: String,
    /// `text` for a loose caption or legend; `frame`, `group`… otherwise.
    #[serde(default)]
    pub kind: String,
    /// How many layers the card has (itself included).
    #[serde(default)]
    pub layers: u64,
    /// An edited card's changed layers, in document order.
    #[serde(default)]
    pub changes: Vec<LayerChange>,
}

/// The design half of the diff.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct MapDiff {
    #[serde(default)]
    pub added: Vec<MapCard>,
    #[serde(default)]
    pub removed: Vec<MapCard>,
    #[serde(default)]
    pub changed: Vec<MapChange>,
    /// The same changes grouped under their cards: the list the widget shows.
    #[serde(default)]
    pub cards: Vec<WaitingCard>,
}

impl MapDiff {
    /// The layer changes the node found.
    pub fn total(&self) -> usize {
        self.added.len().saturating_add(self.removed.len()).saturating_add(self.changed.len())
    }

    /// The widget's rows: one per card, the loose captions and legends folded into one.
    pub fn rows(&self) -> Vec<Row> {
        if self.cards.is_empty() && self.total() > 0 {
            // A node without card grouping: its layers stand for cards.
            return rows(&self.legacy_cards());
        }
        rows(&self.cards)
    }

    /// The layer lists of a node that does not group them, as cards (top-most new and removed).
    fn legacy_cards(&self) -> Vec<WaitingCard> {
        let added: HashSet<u64> = self.added.iter().map(|card| card.id).collect();
        let removed: HashSet<u64> = self.removed.iter().map(|card| card.id).collect();
        let card = |card: &MapCard, state: &str| WaitingCard {
            id: card.id,
            name: card.name.clone(),
            page: card.page.clone(),
            state: state.to_string(),
            layers: 1,
            ..WaitingCard::default()
        };
        let mut out: Vec<WaitingCard> =
            self.added.iter().filter(|c| c.parent.is_none_or(|p| !added.contains(&p))).map(|c| card(c, "added")).collect();
        out.extend(self.changed.iter().map(|change| WaitingCard {
            id: change.id,
            name: change.name.clone(),
            state: "changed".to_string(),
            layers: 1,
            changes: vec![LayerChange {
                id: change.id,
                name: change.name.clone(),
                state: "changed".to_string(),
                parent: None,
                fields: change.fields.clone(),
            }],
            ..WaitingCard::default()
        }));
        out.extend(self.removed.iter().filter(|c| c.parent.is_none_or(|p| !removed.contains(&p))).map(|c| card(c, "removed")));
        out
    }
}

/// What a row's mark says: `+` new, `~` changed, `−` deleted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Added,
    Changed,
    Removed,
}

impl Mark {
    fn of(state: &str) -> Mark {
        match state.trim() {
            "added" => Mark::Added,
            "removed" => Mark::Removed,
            _ => Mark::Changed,
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Mark::Added => "+",
            Mark::Changed => "~",
            Mark::Removed => "−",
        }
    }

    fn color(self) -> Color32 {
        match self {
            Mark::Added => c().map_ink,
            Mark::Changed => c().working_ink,
            Mark::Removed => classic_or(c().danger, c().accent_hover),
        }
    }

    /// The badge's fill on the canvas: the theme's teal, working yellow or danger.
    fn fill(self) -> Color32 {
        match self {
            Mark::Added => c().teal,
            Mark::Changed => c().working,
            Mark::Removed => c().danger,
        }
    }

    /// The glyph on [`Mark::fill`]: ink on the bright working yellow, the accent's text otherwise.
    fn on_fill(self) -> Color32 {
        match self {
            Mark::Changed => classic_or(Color32::WHITE, c().ink),
            Mark::Added | Mark::Removed => c().on_accent,
        }
    }

    fn word(self) -> &'static str {
        match self {
            Mark::Added => "new",
            Mark::Changed => "changed",
            Mark::Removed => "deleted",
        }
    }
}

/// One row of the widget: a waiting card, or every loose caption and legend folded together.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    /// The cards the row stands for (one, or every folded caption).
    pub cards: Vec<u64>,
    pub name: String,
    pub mark: Mark,
    /// `new · 75 layers`, `38 layers changed`, `deleted · nowhere to go`.
    pub sub: String,
    /// The page the canvas opens to show the row (the first card still on the map).
    pub page: Option<u64>,
    /// The cards still on the map, which the canvas flies to and badges.
    pub shown: Vec<u64>,
    /// An edited card's changes: a layer (or `+ 6 separators`) and what changed.
    pub details: Vec<(String, String)>,
}

impl Row {
    /// A row the canvas can show: a deleted card has no place left.
    pub fn navigable(&self) -> bool {
        !self.shown.is_empty()
    }

    /// The row's key: its first card (the folded captions' row keeps its first caption's).
    pub fn key(&self) -> u64 {
        self.cards.first().copied().unwrap_or(0)
    }

    /// Whether the human left every card of this row out.
    pub fn left_out(&self, excluded: &HashSet<u64>) -> bool {
        !self.cards.is_empty() && self.cards.iter().all(|id| excluded.contains(id))
    }
}

/// The rows for `cards`: one per card in the node's order, then the loose one-line text cards
/// (captions, legends) folded into one row.
pub fn rows(cards: &[WaitingCard]) -> Vec<Row> {
    let mut out = Vec::new();
    let mut captions: Vec<&WaitingCard> = Vec::new();
    for card in cards {
        if card.kind.trim() == "text" {
            captions.push(card);
            continue;
        }
        let mark = Mark::of(&card.state);
        let shown = if mark == Mark::Removed { Vec::new() } else { vec![card.id] };
        let name = if card.name.trim().is_empty() { "Unnamed card".to_string() } else { card.name.trim().to_string() };
        out.push(Row {
            cards: vec![card.id],
            name,
            mark,
            sub: card_sub(card, mark),
            page: if mark == Mark::Removed { None } else { card.page_id },
            shown,
            details: if mark == Mark::Changed { details(&card.changes) } else { Vec::new() },
        });
    }
    if !captions.is_empty() {
        let marks: Vec<Mark> = captions.iter().map(|card| Mark::of(&card.state)).collect();
        let mark = if marks.iter().all(|m| *m == Mark::Added) {
            Mark::Added
        } else if marks.iter().all(|m| *m == Mark::Removed) {
            Mark::Removed
        } else {
            Mark::Changed
        };
        let count = |wanted: Mark| marks.iter().filter(|m| **m == wanted).count();
        let sub = [(Mark::Added, "new"), (Mark::Changed, "changed"), (Mark::Removed, "deleted")]
            .into_iter()
            .filter_map(|(m, label)| match count(m) {
                0 => None,
                n => Some(format!("{n} {label}")),
            })
            .collect::<Vec<_>>()
            .join(" · ");
        let still: Vec<&&WaitingCard> = captions.iter().filter(|card| Mark::of(&card.state) != Mark::Removed).collect();
        let page = still.first().and_then(|card| card.page_id);
        out.push(Row {
            cards: captions.iter().map(|card| card.id).collect(),
            name: if captions.len() == 1 { "1 caption or legend".to_string() } else { format!("{} captions and legends", captions.len()) },
            mark,
            sub: if mark == Mark::Removed { "deleted · nowhere to go".to_string() } else { sub },
            page,
            // Only the captions on the page the row opens: the canvas shows one page at a time.
            shown: still.iter().filter(|card| card.page_id == page).map(|card| card.id).collect(),
            details: captions.iter().map(|card| (card.name.trim().to_string(), Mark::of(&card.state).word().to_string())).collect(),
        });
    }
    out
}

/// A card row's sub line.
fn card_sub(card: &WaitingCard, mark: Mark) -> String {
    match mark {
        Mark::Added => format!("new · {}", plural(card.layers as usize, "layer")),
        Mark::Removed => "deleted · nowhere to go".to_string(),
        Mark::Changed => match card.changes.len() {
            0 => "changed".to_string(),
            n => format!("{} changed", plural(n, "layer")),
        },
    }
}

/// `1 layer`, `75 layers`.
fn plural(n: usize, word: &str) -> String {
    if n == 1 { format!("1 {word}") } else { format!("{n} {}", plural_word(word)) }
}

/// `separator` → `separators`; a name that already ends in `s` stays.
fn plural_word(word: &str) -> String {
    if word.ends_with('s') { word.to_string() } else { format!("{word}s") }
}

/// An edited card's changes as the expanded row lists them: each changed layer with its fields,
/// then the new layers (top-most only, the same name counted once: `+ 6 separators`), then the
/// removed ones.
fn details(changes: &[LayerChange]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for change in changes.iter().filter(|c| c.state == "changed") {
        out.push((layer_name(&change.name), field_words(&change.fields)));
    }
    for (state, sign, word) in [("added", "+", "new"), ("removed", "−", "removed")] {
        let ids: HashSet<u64> = changes.iter().filter(|c| c.state == state).map(|c| c.id).collect();
        let mut names: Vec<(String, usize)> = Vec::new();
        for change in changes.iter().filter(|c| c.state == state && c.parent.is_none_or(|p| !ids.contains(&p))) {
            let name = layer_name(&change.name);
            match names.iter_mut().find(|(n, _)| *n == name) {
                Some((_, count)) => *count += 1,
                None => names.push((name, 1)),
            }
        }
        for (name, count) in names {
            let label = if count == 1 { format!("{sign} {name}") } else { format!("{sign} {count} {}", plural_word(&name)) };
            out.push((label, word.to_string()));
        }
    }
    out
}

fn layer_name(name: &str) -> String {
    let name = name.trim();
    if name.is_empty() { "unnamed layer".to_string() } else { name.to_string() }
}

/// The node's property names as a person reads them: `corners · fills`, `height · layout`.
fn field_words(fields: &[String]) -> String {
    let mut words: Vec<&str> = Vec::new();
    for field in fields {
        let word = match field.as_str() {
            "transform" => "position",
            "parent" | "position" => "moved",
            "kind" => "content",
            "childLayout" | "layout" => "layout",
            "corners" => "corners",
            "pluginData" => "data",
            "isMask" | "maskType" => "mask",
            "boolOp" => "shape",
            other => other,
        };
        if !words.contains(&word) {
            words.push(word);
        }
    }
    if words.is_empty() { "changed".to_string() } else { words.join(" · ") }
}

/// One changed file in the code diff.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct FileDelta {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub added: u64,
    #[serde(default)]
    pub removed: u64,
}

/// One commit in the code diff.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct CommitInfo {
    #[serde(default)]
    pub hash: String,
    #[serde(default)]
    pub subject: String,
}

/// The code half of the diff.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct CodeDiff {
    #[serde(default)]
    pub head: Option<String>,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub files: Vec<FileDelta>,
    #[serde(default)]
    pub commits: Vec<CommitInfo>,
    #[serde(default)]
    pub note: Option<String>,
    /// The repository paths the files and commits are limited to; empty is the whole repository.
    #[serde(default)]
    pub scope: Vec<String>,
}

/// The service's answer to `diff`.
#[derive(Debug, Default, Deserialize)]
pub struct DiffReply {
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default, deserialize_with = "last_or_none")]
    pub last: Option<Last>,
    #[serde(default)]
    pub map: MapDiff,
    #[serde(default)]
    pub code: CodeDiff,
    /// The Sync or record already running for this session, if any: Sync waits for it.
    #[serde(default)]
    pub open: Option<Job>,
}

impl DiffReply {
    /// Whether a sync would have anything to apply or say.
    fn has_work(&self) -> bool {
        self.map.total() > 0 || !self.code.files.is_empty() || !self.code.commits.is_empty()
    }
}

/// One sync job the service reports.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Job {
    #[serde(default)]
    pub id: String,
    /// `in_flight`, `done` or `failed`.
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub started: String,
    #[serde(default)]
    pub agent: Option<Value>,
    #[serde(default)]
    pub reason: Option<String>,
    /// What the job records: `sync`, or `explain` / `fix` for an agent's own record.
    #[serde(default, rename = "kind")]
    pub work: String,
    /// The commit a finished job made, when the node says.
    #[serde(default)]
    pub commit: Option<String>,
}

impl Job {
    /// An agent's record of its own explanation or bug fix, not a Sync.
    pub fn is_record(&self) -> bool {
        matches!(self.work.trim(), "explain" | "fix")
    }

    /// The pane of the agent that opened the job, when it said.
    fn pane(&self) -> Option<&str> {
        self.agent.as_ref()?.get("pane")?.as_str().map(str::trim).filter(|pane| !pane.is_empty())
    }

    fn in_flight(&self) -> bool {
        self.state.trim().eq_ignore_ascii_case("in_flight")
    }

    fn failed(&self) -> bool {
        self.state.trim().eq_ignore_ascii_case("failed")
    }

    /// The agent kind the flight names: the service's, then the outcome's name.
    fn kind(&self) -> Option<String> {
        let agent = self.agent.as_ref()?;
        ["kind", "name"]
            .into_iter()
            .filter_map(|key| agent.get(key).and_then(Value::as_str))
            .map(str::trim)
            .find(|value| !value.is_empty())
            .map(str::to_string)
    }
}

/// The service's answer to `sync`.
#[derive(Debug, Default, Deserialize)]
pub struct SyncReply {
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub job: Option<Job>,
    #[serde(default)]
    pub agent: Option<Value>,
}

/// The service's answer to `sync_status`.
#[derive(Debug, Default, Deserialize)]
pub struct StatusReply {
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub job: Option<Job>,
    #[serde(default, deserialize_with = "last_or_none")]
    pub last: Option<Last>,
}

/// What a job thread sends back.
pub enum JobMsg {
    /// The `sync` request returned (or failed before it did).
    Started(Result<SyncReply, String>),
    /// One `sync_status` poll returned.
    Status(Result<StatusReply, String>),
}

/// The Sync button and widget's state.
#[derive(Default)]
pub struct State {
    /// The widget is showing.
    pub open: bool,
    /// Where the header's Sync button was drawn last frame (the first-map tooltip hangs under it).
    pub(crate) button_rect: Option<Rect>,
    /// A diff or a sync request is on its thread (status polls are not "busy").
    pub busy: bool,
    /// The diff last read; replaced after a successful sync.
    pub diff: Option<DiffReply>,
    /// The last sync the service reported (also inside `diff`, kept while a refresh runs).
    pub last: Option<Last>,
    /// Why the last request failed, shown in the widget.
    pub error: Option<String>,
    /// The optional note sent with the next sync.
    pub note: String,
    /// The current or last sync job.
    pub job: Option<Job>,
    #[cfg(not(target_arch = "wasm32"))]
    node: Option<crate::node::SessionRef>,
    #[cfg(not(target_arch = "wasm32"))]
    pushed_jobs: Vec<Value>,
    /// The diff thread's answer.
    pub rx: Option<Receiver<Result<DiffReply, String>>>,
    /// The sync-start thread and the status poll thread's messages.
    pub job_rx: Option<Receiver<JobMsg>>,
    /// The map changed after the diff was read.
    pub dirty: bool,
    /// The snapshot the current diff was taken from.
    snapshot: Option<Snapshot>,
    /// The workspace this state belongs to.
    workspace: Option<String>,
    /// The document uid the diff belongs to.
    doc: Option<u64>,
    /// The design changes the current job was started with.
    applied: usize,
    /// A diff refresh is due after the job finished.
    refresh: bool,
    /// The sync request is on its thread; the job has not come back yet.
    starting: bool,
    /// The agent menu is open.
    pub(crate) menu_open: bool,
    /// The agent menu's search and arrow cursor.
    menu: crate::agent_menu::MenuState,
    /// Sync every page instead of the current page and its subpages.
    all_pages: bool,
    /// The page the diff was read for (`None` when it covers every page).
    page: Option<u64>,
    /// What the Sync this widget started covers: `Some(page)` for a page and its subpages,
    /// `Some(None)` for every page; `None` before the first one.
    job_scope: Option<Option<u64>>,
    /// A cancel of an open record is on its thread.
    cancelling: bool,
    cancel_rx: Option<Receiver<Result<(), String>>>,
    /// The cards the human left out of the next Sync: they keep waiting. Kept across diffs, so a
    /// refresh does not tick a card the human unticked.
    pub excluded: HashSet<u64>,
    /// The current row (its key, see [`Row::key`]): expanded, outlined on the canvas.
    pub current: Option<u64>,
    /// The code section is unfolded.
    pub(crate) code_open: bool,
}

impl State {
    /// Whether a job is in flight (the button says `Syncing…`).
    fn in_flight(&self) -> bool {
        self.job.as_ref().is_some_and(Job::in_flight)
    }

    /// The pages a Sync from this widget is building right now: `Some(page)` for a page and its
    /// subpages, `Some(None)` for every page; `None` while no Sync runs.
    pub fn building(&self) -> Option<Option<u64>> {
        let syncing = self.starting || self.job.as_ref().is_some_and(|job| job.in_flight() && !job.is_record());
        if syncing { self.job_scope } else { None }
    }

    /// Whether the action button must stay disabled.
    fn locked(&self) -> bool {
        self.busy || self.starting || self.in_flight() || self.blocker().is_some()
    }

    /// A Sync or record running for this session that is not this widget's own flight: the node
    /// refuses a new Sync until it finishes or is cancelled.
    pub fn blocker(&self) -> Option<&Job> {
        let own = self.job.as_ref().filter(|job| job.in_flight()).map(|job| job.id.as_str());
        self.diff.as_ref()?.open.as_ref().filter(|job| job.in_flight() && Some(job.id.as_str()) != own)
    }

    /// The waiting cards the button badges, when a diff is loaded.
    fn changes(&self) -> Option<usize> {
        self.diff.as_ref().map(|diff| diff.map.rows().len())
    }

    /// The widget's rows, from the diff last read.
    pub fn rows(&self) -> Vec<Row> {
        self.diff.as_ref().map(|diff| diff.map.rows()).unwrap_or_default()
    }

    /// How many of `rows` go in the next Sync.
    fn included(&self, rows: &[Row]) -> usize {
        rows.iter().filter(|row| !row.left_out(&self.excluded)).count()
    }

    /// The card ids the next Sync leaves out: only those the diff still lists.
    fn excluded_ids(&self, rows: &[Row]) -> Vec<u64> {
        let mut ids: Vec<u64> = rows.iter().flat_map(|row| row.cards.iter().copied()).filter(|id| self.excluded.contains(id)).collect();
        ids.sort_unstable();
        ids
    }

    /// Whether a sync has something to do: an included card, a code change, or a note.
    fn has_work(&self) -> bool {
        if !self.note.trim().is_empty() {
            return true;
        }
        let Some(diff) = self.diff.as_ref() else { return false };
        let rows = diff.map.rows();
        if rows.is_empty() {
            return diff.has_work();
        }
        self.included(&rows) > 0
    }

    /// Whether the Sync button (and ⌘↵) can start a Sync now.
    fn can_sync(&self, compose: &crate::compose::State) -> bool {
        let ready = self.diff.as_ref().is_some_and(|diff| diff.ok);
        !self.locked() && compose.target.is_some() && ready && self.has_work()
    }

    /// Step the current row by `delta` (`[` is −1, `]` is +1), stopping at the ends. With no
    /// current row the first step lands on the first (or last) row. Returns the new current row.
    pub fn step(&mut self, rows: &[Row], delta: i32) -> Option<usize> {
        if rows.is_empty() {
            self.current = None;
            return None;
        }
        let last = rows.len() - 1;
        let index = match self.current.and_then(|key| rows.iter().position(|row| row.key() == key)) {
            Some(at) if delta < 0 => at.saturating_sub(1),
            Some(at) => (at + 1).min(last),
            None if delta < 0 => last,
            None => 0,
        };
        self.current = rows.get(index).map(Row::key);
        Some(index)
    }

    /// Tick or untick every card of `row`.
    fn toggle(&mut self, row: &Row) {
        if row.left_out(&self.excluded) {
            for id in &row.cards {
                self.excluded.remove(id);
            }
        } else {
            self.excluded.extend(row.cards.iter().copied());
        }
    }

    /// Forget everything about the previous workspace or document, keeping nothing in flight.
    fn reset(&mut self) {
        *self = State::default();
    }
}

/// The Burn sidebar toggle: in the canvas header while the sidebar is closed (a button with the
/// disc, `Burn` / `Burn 7` / `Burning…`, and the sidebar toggle at its right), and the open
/// sidebar's own header (the disc and `Burn`, the toggle at the far right). It says `Burning…`
/// while a Burn is in flight, open or closed, until the node reports done.
pub fn header_button(app: &mut FigApp, ui: &mut egui::Ui, ctx: &egui::Context) {
    let burning = app.sync.in_flight() || app.sync.starting;
    let enabled = app.active_workspace.is_some() || app.sync.open;
    let open = app.sync.open;
    let look = button_look(burning, enabled, app.sync.changes());
    let colors = c();
    let font = regular(if open { 14.0 } else { 13.0 });
    let disc = if open { 18.0 } else { 16.0 };
    let label_w = text_width(ui, &look.label, font.clone());
    let lead = if open { 0.0 } else { 8.0 };
    let width = if open { ui.available_width() } else { lead + disc + 7.0 + label_w + 12.0 + TOGGLE };
    let (_, rect) = ui.allocate_space(vec2(width, 30.0));
    let toggle_rect = Rect::from_min_max(pos2(rect.right() - TOGGLE, rect.top() + 1.0), pos2(rect.right(), rect.bottom() - 1.0));
    let label_rect = Rect::from_min_max(rect.min, pos2(toggle_rect.left(), rect.bottom()));
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let response = ui.interact(label_rect, ui.id().with("sync_label_toggle"), sense);
    let toggle = ui.interact(toggle_rect, ui.id().with("sync_sidebar_toggle"), sense);
    app.sync.button_rect = Some(rect);

    let mut painter = ui.painter().clone();
    if !enabled {
        painter.multiply_opacity(0.45);
    }
    if !open {
        let chip = rect.shrink2(vec2(0.0, 1.0));
        let fill = look.fill.unwrap_or(if response.hovered() || toggle.hovered() { colors.soft } else { colors.panel });
        painter.rect_filled(chip, CornerRadius::same(5), fill);
    }
    let disc_rect = Rect::from_center_size(pos2(rect.left() + lead + disc / 2.0, rect.center().y), vec2(disc, disc));
    let state = if burning { burn_disc::DiscState::Burning { t: ui.input(|input| input.time) } } else { burn_disc::DiscState::Blank };
    burn_disc::paint(&painter, disc_rect, state, &colors);
    let ink = if open { if enabled { colors.ink } else { colors.faint } } else { look.ink };
    let room = (toggle_rect.left() - disc_rect.right() - 12.0).max(20.0);
    let shown = widgets::elide_font(ui, &look.label, room, font.clone());
    painter.text(pos2(disc_rect.right() + 7.0, rect.center().y), Align2::LEFT_CENTER, shown, font, ink);
    let toggle_ink = if toggle.hovered() {
        colors.accent
    } else if open || look.fill.is_some() {
        colors.teal
    } else {
        colors.muted
    };
    if open {
        painter.rect_filled(toggle_rect.shrink(1.0), CornerRadius::same(5), colors.selected);
    }
    sidebar_mark(&painter, toggle_rect.center(), toggle_ink);

    let label = if open { "Hide Burn sidebar" } else { "Show Burn sidebar" };
    a11y::button(&response, label);
    a11y::button(&toggle, label);
    let response = response.on_hover_text(label);
    let toggle = toggle.on_hover_text(label);
    if enabled && (response.clicked() || toggle.clicked()) {
        if app.sync.open {
            app.sync.open = false;
            app.sync.menu_open = false;
            ctx.memory_mut(|memory| memory.surrender_focus(Id::new(NOTE_ID)));
            ctx.request_repaint();
        } else {
            open_panel(app, ctx);
        }
    }
}

/// The sidebar toggle's width at the button's right.
const TOGGLE: f32 = 28.0;

/// A right sidebar: an outline with its right edge filled, centred on `c`.
fn sidebar_mark(painter: &egui::Painter, c: egui::Pos2, ink: Color32) {
    let outline = Rect::from_center_size(c, vec2(16.0, 14.0));
    painter.rect_stroke(outline, CornerRadius::same(2), Stroke::new(1.3, ink), StrokeKind::Inside);
    let edge = Rect::from_min_max(pos2(outline.right() - 5.0, outline.top() + 1.0), pos2(outline.right() - 1.0, outline.bottom() - 1.0));
    painter.rect_filled(edge, CornerRadius::ZERO, ink);
}

/// What the header button shows in each designed state.
#[derive(Debug, PartialEq, Eq)]
struct ButtonLook {
    label: String,
    ink: Color32,
    /// The button's fill when it stands out (cards waiting); the panel otherwise.
    fill: Option<Color32>,
}

/// Idle `Burn`; Changes `Burn 7` on the teal; Burning `Burning…` (the disc spins) until the node
/// reports done, whether the sidebar is open or closed; Disabled greyed.
fn button_look(burning: bool, enabled: bool, changes: Option<usize>) -> ButtonLook {
    let colors = c();
    if burning {
        return ButtonLook { label: "Burning…".to_string(), ink: colors.muted, fill: None };
    }
    if !enabled {
        return ButtonLook { label: "Burn".to_string(), ink: colors.faint, fill: None };
    }
    match changes.filter(|n| *n > 0) {
        Some(n) => ButtonLook { label: format!("Burn {n}"), ink: colors.map_ink, fill: Some(colors.selected) },
        None => ButtonLook { label: "Burn".to_string(), ink: colors.muted, fill: None },
    }
}

/// Open the widget and read a fresh diff from the current map.
pub fn open_panel(app: &mut FigApp, ctx: &egui::Context) {
    app.sync.open = true;
    app.sync.menu_open = false;
    if app.sync.busy || app.sync.starting || app.sync.in_flight() {
        return;
    }
    let Some(workspace) = app.active_workspace.clone() else {
        app.toast("Open a workspace map to burn.");
        return;
    };
    match snapshot_now(app) {
        Ok((snapshot, page)) => start_diff(app, ctx, snapshot, page, workspace),
        Err(message) => {
            app.sync.open = false;
            app.toast(message);
        }
    }
}

/// Poll state and network replies every map frame, before the parent sizes the panels.
pub fn poll(app: &mut FigApp, ctx: &egui::Context) {
    let Some(workspace) = app.active_workspace.clone() else {
        if app.sync.workspace.is_some() {
            app.sync.reset();
        }
        return;
    };
    #[cfg(not(target_arch = "wasm32"))]
    let identity = workspaces::node_identity(app).map(|node| format!("{}:{}", node.address, node.session)).unwrap_or_else(|| workspace.clone());
    #[cfg(target_arch = "wasm32")]
    let identity = workspace.clone();
    if app.sync.workspace.as_deref() != Some(identity.as_str()) {
        app.sync.reset();
        app.sync.workspace = Some(identity);
    }
    let uid = app.session.ds().ok().map(|ds| ds.uid);
    if app.sync.doc != uid {
        // A different document: its diff is not this map's. A job already in flight belongs to
        // the workspace and keeps polling; only the diff is dropped.
        app.sync.doc = uid;
        app.sync.diff = None;
        app.sync.snapshot = None;
        app.sync.dirty = false;
        app.sync.error = None;
        app.sync.open = false;
        app.sync.menu_open = false;
    }

    // A page sync's diff belongs to the page it was read on and its subpages: moving to another
    // page reads that page's diff instead (a row opening one of its subpages keeps it). A job in
    // flight keeps polling.
    let current_page = app.session.page().ok().map(|page| page.0);
    let in_scope = match (app.sync.page, current_page, app.session.doc().ok()) {
        (Some(scope), Some(page), Some(doc)) => scope == page || crate::page_tree::descendants(doc, NodeId(scope)).contains(&NodeId(page)),
        _ => false,
    };
    if !app.sync.all_pages && !app.sync.busy && app.sync.page.is_some() && !in_scope {
        app.sync.diff = None;
        app.sync.snapshot = None;
        app.sync.page = None;
        app.sync.dirty = false;
        app.sync.error = None;
        app.sync.refresh = app.sync.open;
    }

    poll_diff(app);
    poll_job(app, ctx);
    poll_cancel(app);
    #[cfg(not(target_arch = "wasm32"))]
    if !app.sync.starting {
        let address = app.sync.node.as_ref().map(|node| node.address.clone());
        for event in std::mem::take(&mut app.sync.pushed_jobs) {
            if let Some(address) = address.as_ref() {
                node_event(app, ctx, address, event);
            }
        }
    }
    if app.sync.refresh && !app.sync.busy {
        app.sync.refresh = false;
        if let Ok((snapshot, page)) = snapshot_now(app) {
            start_diff(app, ctx, snapshot, page, workspace.clone());
        }
    }
    if app.sync.open {
        refresh_dirty(app);
    } else {
        app.sync.dirty = false;
    }
}

/// Fill the parent's margin-free right panel, including the window-height header and footer.
pub fn pane(app: &mut FigApp, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let area = ui.max_rect();
    ui.painter().rect_filled(area, CornerRadius::ZERO, c().panel);
    ui.painter().vline(area.left() + 0.5, area.y_range(), Stroke::new(1.0, c().border));
    let header_h = if app.ui.mac_inset() { crate::MAC_BAND } else { 44.0 };
    let top = Rect::from_min_size(area.min, vec2(area.width(), header_h));
    if app.ui.title_bar != crate::TitleBar::Native {
        crate::titlebar_drag(ui, top);
    }
    let controls = Rect::from_min_max(pos2(top.left() + 18.0, top.top()), pos2(top.right() - 10.0, top.bottom()));
    let mut head =
        ui.new_child(egui::UiBuilder::new().id_salt("sync_header").max_rect(controls).layout(egui::Layout::left_to_right(egui::Align::Center)));
    header_button(app, &mut head, &ctx);
    ui.painter().hline(top.x_range(), top.bottom() - 0.5, Stroke::new(1.0, c().border));
    if !app.sync.open {
        return;
    }
    let Some(workspace) = app.active_workspace.clone() else { return };

    let scope = scope_label(app);
    let rows = app.sync.rows();
    if app.sync.current.is_some_and(|key| !rows.iter().any(|row| row.key() == key)) {
        app.sync.current = None;
    }
    // The pane's keys, before the map's shortcuts see them: `[` and `]` step through the rows
    // (unless a field is typing), ⌘↵ syncs (also from the note).
    let mut keys = WidgetOut::default();
    // A focused button still allows navigation; only a text editor owns typing keys.
    let typing = ctx.text_edit_focused();
    let note_focused = ctx.memory(|memory| memory.has_focus(Id::new(NOTE_ID)));
    let hovered = ui.rect_contains_pointer(area);
    let owns_keys = !crate::map_actions::overlay_open(app, &ctx) && (note_focused || (hovered && !typing));
    if owns_keys && ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, Key::Enter)) {
        keys.sync = true;
    }
    if owns_keys && !typing {
        for (key, delta) in [(Key::OpenBracket, -1), (Key::CloseBracket, 1)] {
            if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, key))
                && let Some(index) = app.sync.step(&rows, delta)
            {
                keys.go = rows.get(index).map(Row::key);
            }
        }
    }
    let menu_was_open = app.sync.menu_open;
    let body_rect = Rect::from_min_max(pos2(area.left(), top.bottom()), area.max);
    let mut body = ui.new_child(egui::UiBuilder::new().id_salt("sync_body").max_rect(body_rect));
    body.set_clip_rect(body_rect.intersect(ui.clip_rect()));
    body.spacing_mut().item_spacing.y = 0.0;
    let mut out = widget_body(&mut body, &mut app.sync, &mut app.compose, &scope, &rows);
    out.sync |= keys.sync && app.sync.can_sync(&app.compose);
    out.go = out.go.or(keys.go);
    if let Some(key) = out.go
        && let Some(row) = rows.iter().find(|row| row.key() == key)
    {
        fly_to(app, row);
    }

    if menu_was_open && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, Key::Escape)) {
        app.sync.menu_open = false;
    }
    if let Some(all_pages) = out.scope
        && all_pages != app.sync.all_pages
        && !app.sync.busy
    {
        app.sync.all_pages = all_pages;
        app.sync.diff = None;
        app.sync.snapshot = None;
        app.sync.page = None;
        app.sync.dirty = false;
        app.sync.error = None;
        if let Ok((snapshot, page)) = snapshot_now(app) {
            start_diff(app, &ctx, snapshot, page, workspace.clone());
        }
    }
    if let Some(job) = out.cancel {
        start_cancel(app, &ctx, job);
    }
    if out.sync && app.sync.open {
        match snapshot_now(app) {
            Ok((snapshot, page)) => start_sync(app, &ctx, workspace, snapshot, page),
            Err(message) => app.toast(message),
        }
    }
}

/// The widget's scope line: the current page and how many subpages a page sync takes along.
fn scope_label(app: &FigApp) -> String {
    let Ok(doc) = app.session.doc() else { return String::new() };
    let Ok(page) = app.session.page() else { return String::new() };
    let name = doc.node(page).map(|node| node.name.clone()).unwrap_or_else(|| "this page".to_string());
    match crate::page_tree::descendants(doc, page).len() {
        0 => name,
        1 => format!("{name} + 1 subpage"),
        n => format!("{name} + {n} subpages"),
    }
}

/// What the widget body asked for.
#[derive(Default)]
struct WidgetOut {
    sync: bool,
    /// The scope the human picked: `Some(true)` for every page, `Some(false)` for this page.
    scope: Option<bool>,
    /// Cancel this open record.
    cancel: Option<String>,
    /// Show this row's card on the canvas (its key).
    go: Option<u64>,
}

/// The note field's id, so ⌘↵ syncs from it.
const NOTE_ID: &str = "sync_note";

/// The widget's frame contents: the header (scope, cards waiting, stepper), the cards and the
/// folded code in a scroll area, and the sticky footer (agent, note, action) that stays in view
/// while the list scrolls.
fn widget_body(ui: &mut Ui, state: &mut State, compose: &mut crate::compose::State, scope: &str, rows: &[Row]) -> WidgetOut {
    // Reserve the footer before laying out the list, so even an empty diff fills the pane.
    let sync = egui::Panel::bottom("sync_footer")
        .resizable(false)
        .default_size(170.0)
        .show_separator_line(false)
        .frame(egui::Frame::NONE.fill(c().panel).inner_margin(egui::Margin { left: 19, right: 19, top: 0, bottom: 17 }))
        .show(ui, |ui| footer(ui, state, compose, rows))
        .inner;
    let mut out = WidgetOut { sync, ..WidgetOut::default() };
    let content = ui.available_rect_before_wrap().shrink2(vec2(PAD, 0.0));
    let mut list = ui.new_child(egui::UiBuilder::new().id_salt("sync_list").max_rect(content));
    list.set_clip_rect(content.intersect(ui.clip_rect()));
    let ui = &mut list;
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 0.0);
        let enabled = !state.busy;
        if scope_chip(ui, "This page", !state.all_pages, enabled) {
            out.scope = Some(false);
        }
        if scope_chip(ui, "All pages", state.all_pages, enabled) {
            out.scope = Some(true);
        }
    });
    ui.add_space(12.0);
    if let Some(go) = header(ui, state, rows) {
        out.go = Some(go);
    }
    if state.dirty {
        ui.add_space(7.0);
        ui.label(egui::RichText::new("the map changed since this diff").font(mono(8.5)).color(c().accent));
    }
    let mut carded = false;
    if let Some(job) = state.job.as_ref() {
        if job.in_flight() {
            ui.add_space(7.0);
            flight_card(ui, job, compose);
            carded = true;
        } else if job.failed() {
            let reason = job.reason.clone().unwrap_or_else(|| "The Burn failed.".to_string());
            ui.add_space(7.0);
            failed_card(ui, &reason);
            carded = true;
        }
    }
    if let Some(open) = state.blocker() {
        ui.add_space(7.0);
        if open_card(ui, open, &compose.targets, state.cancelling, now_secs()) {
            out.cancel = Some(open.id.clone());
        }
        carded = true;
    }
    ui.add_space(if carded { 9.0 } else { 7.0 });
    divider(ui);

    // All remaining height belongs to the list; short content must not pull the footer up.
    let scroll_h = ui.available_height().max(0.0);
    egui::ScrollArea::vertical().id_salt("sync_widget_body").max_height(scroll_h).min_scrolled_height(0.0).auto_shrink([false, false]).show(
        ui,
        |ui| {
            ui.set_width(ui.available_width());
            ui.add_space(5.0);
            if let Some(error) = state.error.clone() {
                error_block(ui, &error);
                ui.add_space(10.0);
            }
            match state.diff.as_ref().map(|diff| diff.code.clone()) {
                Some(code) => {
                    let covers = if state.all_pages { "every page" } else { scope };
                    if let Some(include) = include_line(ui, covers, !rows.is_empty()) {
                        if include {
                            state.excluded.clear();
                        } else {
                            state.excluded.extend(rows.iter().flat_map(|row| row.cards.iter().copied()));
                        }
                    }
                    if rows.is_empty() {
                        ui.add_space(7.0);
                        ui.label(egui::RichText::new("No design changes since the last Burn.").font(regular(10.5)).color(c().muted));
                        ui.add_space(7.0);
                    }
                    let mut clicked = None;
                    let mut toggled = None;
                    for row in rows {
                        let current = state.current == Some(row.key());
                        let row_out = row_ui(ui, row, current, row.left_out(&state.excluded));
                        if row_out.toggle {
                            toggled = Some(row.clone());
                        } else if row_out.go {
                            clicked = Some(row.key());
                        }
                    }
                    if let Some(row) = toggled {
                        state.toggle(&row);
                    }
                    if let Some(key) = clicked {
                        state.current = Some(key);
                        out.go = Some(key);
                    }
                    ui.add_space(4.0);
                    divider(ui);
                    ui.add_space(9.0);
                    if code_line(ui, state.last.as_ref(), &code, state.code_open) {
                        state.code_open = !state.code_open;
                    }
                    if state.code_open {
                        ui.add_space(4.0);
                        code_section(ui, state.last.as_ref(), &code);
                    }
                    ui.add_space(9.0);
                }
                None => {
                    ui.add_space(5.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing = vec2(7.0, 0.0);
                        if state.busy {
                            ui.add(egui::Spinner::new().size(11.0).color(c().accent));
                            ui.label(egui::RichText::new("Reading the diff…").font(regular(11.0)).color(c().muted));
                        } else {
                            ui.label(egui::RichText::new("No diff read yet.").font(regular(11.0)).color(c().muted));
                        }
                    });
                    ui.add_space(9.0);
                }
            }
        },
    );
    out
}

/// `14 cards waiting`, `12 in this Sync · 2 keep waiting · since 519ac2f`, and the stepper
/// `‹ 3 / 14 ›` with its `[ ] step` hint. Returns the row a stepper arrow moved to.
fn header(ui: &mut Ui, state: &mut State, rows: &[Row]) -> Option<u64> {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::hover());
    let total = rows.len();
    let title = match (&state.diff, total) {
        (None, _) if state.busy => "Reading the diff…".to_string(),
        (None, _) => "Burn".to_string(),
        (Some(_), 0) => "Nothing waiting".to_string(),
        (Some(_), 1) => "1 card waiting".to_string(),
        (Some(_), n) => format!("{n} cards waiting"),
    };
    let stepper_w = 84.0;
    let room = (rect.width() - if total > 0 { stepper_w + 12.0 } else { 0.0 }).max(40.0);
    let shown = widgets::elide_font(ui, &title, room, semibold(16.0));
    ui.painter().text(pos2(rect.left(), rect.top() + 11.0), Align2::LEFT_CENTER, shown, semibold(16.0), c().ink);
    let line = widgets::elide_font(ui, &header_line(state, rows), room, mono(8.0));
    ui.painter().text(pos2(rect.left(), rect.top() + 33.0), Align2::LEFT_CENTER, line, mono(8.0), c().muted);
    if total == 0 {
        return None;
    }
    let pill = Rect::from_min_size(pos2(rect.right() - stepper_w, rect.top()), vec2(stepper_w, 24.0));
    ui.painter().rect_filled(pill, CornerRadius::same(12), c().panel);
    ui.painter().rect_stroke(pill, CornerRadius::same(12), Stroke::new(1.0, c().border), StrokeKind::Inside);
    let at = state.current.and_then(|key| rows.iter().position(|row| row.key() == key));
    let count = match at {
        Some(at) => format!("{} / {total}", at + 1),
        None => format!("– / {total}"),
    };
    ui.painter().text(pill.center(), Align2::CENTER_CENTER, count, regular(11.0), c().ink);
    ui.painter().text(pos2(pill.center().x, pill.bottom() + 11.0), Align2::CENTER_CENTER, "[ ]  step", mono(8.0), c().faint);
    let mut moved = None;
    for (delta, glyph, zone, label) in [
        (-1, "‹", Rect::from_min_size(pill.min, vec2(26.0, pill.height())), "Previous waiting card"),
        (1, "›", Rect::from_min_size(pos2(pill.right() - 26.0, pill.top()), vec2(26.0, pill.height())), "Next waiting card"),
    ] {
        let response = ui.interact(zone, Id::new(("sync_step", delta)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        a11y::button(&response, label);
        let response = response.on_hover_text(format!("{label}  {}", if delta < 0 { "[" } else { "]" }));
        ui.painter().text(zone.center(), Align2::CENTER_CENTER, glyph, regular(13.0), if response.hovered() { c().accent } else { c().muted });
        if response.clicked()
            && let Some(index) = state.step(rows, delta)
        {
            moved = rows.get(index).map(Row::key);
        }
    }
    moved
}

/// `12 in this Sync · 2 keep waiting · since 519ac2f`.
fn header_line(state: &State, rows: &[Row]) -> String {
    let since = match state.last.as_ref().map(|last| short_commit(&last.commit)).filter(|commit| !commit.is_empty()) {
        Some(commit) => format!("since {commit}"),
        None if state.last.is_some() => "since the last Burn".to_string(),
        None => "first Burn".to_string(),
    };
    if state.diff.is_none() || rows.is_empty() {
        return since;
    }
    let included = state.included(rows);
    let waiting = rows.len() - included;
    match waiting {
        0 => format!("{included} selected · {since}"),
        1 => format!("{included} selected · 1 keeps waiting · {since}"),
        n => format!("{included} selected · {n} keep waiting · {since}"),
    }
}

/// The line above the list: what the scope covers, and `include: all · none`. Returns `Some(true)`
/// for all, `Some(false)` for none.
fn include_line(ui: &mut Ui, covers: &str, rows: bool) -> Option<bool> {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::hover());
    let mut picked = None;
    let mut x = rect.right() - 4.0;
    if rows {
        // Right to left: `none`, ` · `, `all`, `include: `.
        for (label, value) in [("none", false), ("all", true)] {
            let w = text_width(ui, label, mono(8.5));
            let zone = Rect::from_min_size(pos2(x - w, rect.top()), vec2(w, rect.height()));
            let response =
                ui.interact(zone.expand(2.0), Id::new(("sync_include", value)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
            a11y::button(&response, if value { "Include every card" } else { "Leave every card out" });
            let ink = if response.hovered() { c().accent } else { c().muted };
            ui.painter().text(pos2(zone.left(), rect.center().y), Align2::LEFT_CENTER, label, mono(8.5), ink);
            if response.clicked() {
                picked = Some(value);
            }
            x = zone.left() - text_width(ui, " · ", mono(8.5));
            if value {
                x = zone.left();
            } else {
                ui.painter().text(pos2(x, rect.center().y), Align2::LEFT_CENTER, " · ", mono(8.5), c().faint);
            }
        }
        let label = "include: ";
        x -= text_width(ui, label, mono(8.5));
        ui.painter().text(pos2(x, rect.center().y), Align2::LEFT_CENTER, label, mono(8.5), c().faint);
    }
    let room = (x - rect.left() - 10.0).max(30.0);
    let covers = widgets::elide_font(ui, covers, room, mono(8.5));
    ui.painter().text(pos2(rect.left(), rect.center().y), Align2::LEFT_CENTER, covers, mono(8.5), c().faint);
    picked
}

/// What a row asked for.
#[derive(Default)]
struct RowOut {
    /// The row, or its →, was clicked: make it current and show its card.
    go: bool,
    /// Its checkbox was clicked.
    toggle: bool,
}

/// One waiting card: mark, name and sub line, the include checkbox and →; the current row is
/// filled, barred in amber and lists its changed layers.
fn row_ui(ui: &mut Ui, row: &Row, current: bool, left_out: bool) -> RowOut {
    let width = ui.available_width();
    let listed = if current { row.details.len().min(DETAILS) + usize::from(row.details.len() > DETAILS) } else { 0 };
    let height = ROW_H + if listed > 0 { 4.0 + listed as f32 * DETAIL_H } else { 0.0 };
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    let id = Id::new(("sync_row", row.key()));
    let arrow = Rect::from_min_size(pos2(rect.right() - 26.0, rect.top() + 9.0), vec2(26.0, 22.0));
    let check = Rect::from_min_size(pos2(arrow.left() - 16.0 - 18.0, rect.top() + 11.0), vec2(18.0, 18.0));
    let check_response = ui.interact(check.expand(3.0), id.with("check"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    let include = if left_out { format!("Include {}", row.name) } else { format!("Leave {} out", row.name) };
    a11y::button(&check_response, &include);
    let arrow_response = row.navigable().then(|| {
        let response = ui.interact(arrow, id.with("go"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        a11y::button(&response, &format!("Show {} on the map", row.name));
        response.on_hover_text("Show it on the map")
    });
    let response = if row.navigable() { response.on_hover_cursor(egui::CursorIcon::PointingHand) } else { response };
    a11y::button(&response, &row.name);
    let painter = ui.painter();
    // The current row reaches into the widget's padding, with an amber bar on its left.
    let band = Rect::from_min_max(pos2(rect.left() - 12.0, rect.top()), pos2(rect.right() + 6.0, rect.bottom()));
    if current {
        painter.rect_filled(band, CornerRadius::ZERO, row_hot());
        painter.rect_filled(Rect::from_min_size(band.min, vec2(3.0, band.height())), CornerRadius::ZERO, c().working);
    } else if response.hovered() {
        painter.rect_filled(band, CornerRadius::ZERO, c().soft);
    }
    let dim = |color: Color32| if left_out { color.gamma_multiply(0.55) } else { color };
    painter.text(pos2(rect.left() + 4.0, rect.top() + 15.0), Align2::CENTER_CENTER, row.mark.glyph(), mono(13.0), dim(row.mark.color()));
    let x = rect.left() + 18.0;
    let room = (check.left() - 10.0 - x).max(30.0);
    let removed = row.mark == Mark::Removed;
    let ink = if left_out {
        c().faint
    } else if removed {
        c().muted
    } else {
        c().ink
    };
    let name = widgets::elide_font(ui, &row.name, room, regular(12.0));
    let galley = painter.layout_no_wrap(name, regular(12.0), ink);
    let name_at = pos2(x, rect.top() + 15.0 - galley.size().y / 2.0);
    let name_w = galley.size().x;
    painter.galley(name_at, galley, ink);
    if removed {
        let y = rect.top() + 15.5;
        painter.line_segment([pos2(x, y), pos2(x + name_w, y)], Stroke::new(1.0, ink));
    }
    let (sub, sub_ink) = if left_out {
        ("left out · keeps waiting", c().working_ink)
    } else if removed {
        (row.sub.as_str(), row.mark.color())
    } else {
        (row.sub.as_str(), c().muted)
    };
    let sub = widgets::elide_font(ui, sub, room, mono(8.5));
    painter.text(pos2(x, rect.top() + 31.0), Align2::LEFT_CENTER, sub, mono(8.5), sub_ink);
    // The include checkbox: ticked in the accent, empty and outlined when left out.
    let hot = check_response.hovered();
    if left_out {
        painter.rect_filled(check, CornerRadius::same(4), c().panel);
        painter.rect_stroke(check, CornerRadius::same(4), Stroke::new(1.2, if hot { c().accent } else { c().faint }), StrokeKind::Inside);
    } else {
        painter.rect_filled(check, CornerRadius::same(4), if hot { c().accent_hover } else { c().accent });
        let tick = [
            pos2(check.left() + 4.5, check.center().y),
            pos2(check.left() + 7.8, check.bottom() - 5.0),
            pos2(check.right() - 4.0, check.top() + 5.0),
        ];
        painter.add(egui::Shape::line(tick.to_vec(), Stroke::new(1.6, c().on_accent)));
    }
    if let Some(arrow_response) = arrow_response.as_ref() {
        let fill = if arrow_response.hovered() { c().soft } else { c().panel };
        painter.rect_filled(arrow, CornerRadius::same(5), fill);
        painter.rect_stroke(arrow, CornerRadius::same(5), Stroke::new(1.0, c().border), StrokeKind::Inside);
        painter.text(arrow.center(), Align2::CENTER_CENTER, "→", regular(11.0), c().map_ink);
    }
    if listed > 0 {
        let column = x + 18.0;
        let fields_x = column + ((rect.right() - column) * 0.48).max(80.0);
        let shown = row.details.iter().take(DETAILS).map(|(layer, what)| (layer.clone(), what.clone()));
        let more = (row.details.len() > DETAILS).then(|| (format!("+{} more", row.details.len() - DETAILS), String::new()));
        for (index, (layer, what)) in shown.chain(more).enumerate() {
            let y = rect.top() + ROW_H + 2.0 + (index as f32 + 0.5) * DETAIL_H;
            let layer = widgets::elide_font(ui, &layer, (fields_x - column - 8.0).max(30.0), mono(9.0));
            painter.text(pos2(column, y), Align2::LEFT_CENTER, layer, mono(9.0), c().ink);
            let what = widgets::elide_font(ui, &what, (rect.right() - fields_x).max(30.0), mono(9.0));
            painter.text(pos2(fields_x, y), Align2::LEFT_CENTER, what, mono(9.0), c().muted);
        }
    }
    let toggle = check_response.clicked();
    let arrow_clicked = arrow_response.is_some_and(|response| response.clicked());
    RowOut { go: !toggle && (response.clicked() || arrow_clicked), toggle }
}

/// `▸  Code since 519ac2f   11 files · 5 commits`: the code section folded to one line. Returns
/// whether it was clicked (to fold or unfold).
fn code_line(ui: &mut Ui, last: Option<&Last>, code: &CodeDiff, open: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let title = match code_since(last, code) {
        Some(commit) => format!("Code since {}", short_commit(&commit)),
        None if last.is_none() => "Code · first Burn".to_string(),
        None => "Code since the last Burn".to_string(),
    };
    a11y::button(&response, &if open { format!("Fold {title}") } else { format!("Unfold {title}") });
    let hot = response.hovered();
    let glyph = if open { "▾" } else { "▸" };
    ui.painter().text(
        pos2(rect.left() + 4.0, rect.center().y),
        Align2::CENTER_CENTER,
        glyph,
        regular(10.0),
        if hot { c().accent } else { c().muted },
    );
    let x = rect.left() + 18.0;
    let galley = ui.painter().layout_no_wrap(title, regular(11.0), if hot { c().accent } else { c().ink });
    let title_w = galley.size().x;
    ui.painter().galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley, c().ink);
    let files = code.files.len();
    let commits = code.commits.len();
    let counts = if last.is_none() {
        "the map is the baseline".to_string()
    } else if files == 0 && commits == 0 {
        "no changes".to_string()
    } else {
        format!("{} · {}", plural(files, "file"), plural(commits, "commit"))
    };
    let room = (rect.right() - x - title_w - 12.0).max(30.0);
    let counts = widgets::elide_font(ui, &counts, room, mono(8.5));
    ui.painter().text(pos2(x + title_w + 12.0, rect.center().y), Align2::LEFT_CENTER, counts, mono(8.5), c().muted);
    response.clicked()
}

/// The commit the code is compared with: the last sync's, or the one the node names.
fn code_since(last: Option<&Last>, code: &CodeDiff) -> Option<String> {
    last.map(|last| last.commit.trim().to_string())
        .filter(|commit| !commit.is_empty())
        .or_else(|| code.from.as_deref().map(str::trim).filter(|commit| !commit.is_empty()).map(str::to_string))
}

/// The pinned footer on its white band: the agent button, the note, and the full-width action
/// `Sync 12 of 14 cards with codex  ⌘↵`. Returns whether the action was clicked.
fn footer(ui: &mut Ui, state: &mut State, compose: &mut crate::compose::State, rows: &[Row]) -> bool {
    let top = ui.cursor().top();
    ui.add_space(13.0);
    // The composer's agent button and menu: the same agent, chosen the same way.
    if let Some(target) = agent_button(ui, state, compose) {
        compose.choose(target);
    }
    ui.add_space(8.0);
    note_field(ui, &mut state.note);
    ui.add_space(10.0);
    let syncing = state.starting || state.in_flight();
    let target = compose.target.as_ref();
    let ready = state.diff.as_ref().is_some_and(|diff| diff.ok);
    let enabled = state.can_sync(compose);
    let label =
        if syncing { "Burning…".to_string() } else { sync_label(state.included(rows), rows.len(), target.map(|target| kind_label(target.kind()))) };
    let clicked = action_button(ui, &label, enabled, syncing);
    let hint = if state.blocker().is_some() {
        Some("Burn waits until the open record is committed or cancelled.")
    } else if target.is_none() {
        Some(if compose.targets.is_empty() { "No agent is available in this workspace yet." } else { "Choose an agent to burn." })
    } else if syncing {
        Some("The map stays editable while the Burn runs.")
    } else if state.busy {
        Some("Reading the diff…")
    } else if !ready {
        Some("Refresh the diff to burn.")
    } else if !state.has_work() {
        Some(if rows.is_empty() { "Change the map or add a note to burn." } else { "Include a card or add a note to burn." })
    } else {
        None
    };
    if let Some(hint) = hint {
        ui.add_space(7.0);
        ui.label(egui::RichText::new(hint).font(mono(8.5)).color(c().muted));
    }
    let max = ui.max_rect();
    ui.painter().hline((max.left() - PAD)..=(max.right() + PAD), top, Stroke::new(1.0, c().border));
    clicked
}

/// `Burn 12 cards with Codex` (the cards ticked), or `Burn with Codex` when no card waits.
fn sync_label(included: usize, total: usize, kind: Option<&str>) -> String {
    let cards = if total == 0 { String::new() } else { format!(" {}", plural(included, "card")) };
    match kind {
        Some(kind) => format!("Burn{cards} with {}", capitalized(kind)),
        None => format!("Burn{cards}"),
    }
}

/// `codex` → `Codex`.
fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The toast when a Burn is done: `Burned a1f9c2e` when the node names the commit, else
/// `Burned · 3 cards` (or `Burned`).
fn burned_toast(commit: Option<&str>, cards: usize) -> String {
    match commit.map(short_commit).filter(|commit| !commit.is_empty()) {
        Some(commit) => format!("Burned {commit}"),
        None if cards == 0 => "Burned".to_string(),
        None => format!("Burned · {}", plural(cards, "card")),
    }
}

/// The in-flight card: spinner, `Sync in flight · kind`, and when it started.
fn flight_card(ui: &mut Ui, job: &Job, compose: &crate::compose::State) {
    let kind =
        job.kind().or_else(|| compose.target.as_ref().map(|target| kind_label(target.kind()).to_string())).unwrap_or_else(|| "agent".to_string());
    egui::Frame::NONE
        .fill(c().soft)
        .stroke(Stroke::new(1.0, c().accent))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin { left: 13, right: 13, top: 10, bottom: 10 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.label(egui::RichText::new(format!("Burning · {kind}")).font(semibold(11.0)).color(c().accent));
            let started = job.started.trim();
            let line = if started.is_empty() { "started just now".to_string() } else { format!("started {started}") };
            ui.label(egui::RichText::new(line).font(mono(9.0)).color(c().muted));
        });
}

/// The open record (or another Sync) the node is waiting on: what it is, whose it is, how long it
/// has waited, and a Cancel button for a record. Returns whether Cancel was clicked.
fn open_card(ui: &mut Ui, job: &Job, targets: &[TargetInfo], cancelling: bool, now: Option<u64>) -> bool {
    let mut clicked = false;
    egui::Frame::NONE
        .fill(c().soft)
        .stroke(Stroke::new(1.0, c().accent))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin { left: 13, right: 13, top: 10, bottom: 10 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.label(egui::RichText::new(open_title(job)).font(semibold(11.0)).color(c().accent));
            let line = format!("by {} · {}", job_owner(job, targets), waited(&job.started, now));
            ui.label(egui::RichText::new(line).font(mono(9.0)).color(c().muted));
            if let Some(reason) = job.reason.as_deref().map(str::trim).filter(|reason| !reason.is_empty()) {
                ui.add(egui::Label::new(egui::RichText::new(reason).font(mono(9.0)).color(c().muted)).wrap());
            }
            if job.is_record() {
                ui.add_space(5.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = vec2(8.0, 0.0);
                    let label = if cancelling { "Cancelling…" } else { "Cancel record" };
                    clicked = secondary_button(ui, label, !cancelling);
                    ui.label(egui::RichText::new("commits nothing").font(mono(8.5)).color(c().faint));
                });
            }
        });
    clicked
}

/// `Explanation record open`, `Bug fix record open`, or another `Sync in flight`.
pub(crate) fn open_title(job: &Job) -> &'static str {
    match job.work.trim() {
        "explain" => "Explanation record open",
        "fix" => "Bug fix record open",
        _ => "Burn in flight",
    }
}

/// Whose job it is: the agent in its pane, by the name the agent menu uses.
pub(crate) fn job_owner(job: &Job, targets: &[TargetInfo]) -> String {
    let Some(pane) = job.pane() else { return "an agent outside Herdr".to_string() };
    targets
        .iter()
        .find_map(|info| match &info.target {
            Target::Existing { pane: p, kind, .. } if p == pane => Some(if info.label.trim().is_empty() {
                kind_label(kind).to_string()
            } else {
                format!("{} · {}", info.label.trim(), kind_label(kind))
            }),
            _ => None,
        })
        .unwrap_or_else(|| format!("pane {pane}"))
}

/// `waited 12 min` from the job's start (Unix seconds) to now.
pub(crate) fn waited(started: &str, now: Option<u64>) -> String {
    let (Some(start), Some(now)) = (started.trim().parse::<u64>().ok(), now) else { return "open".to_string() };
    let minutes = now.saturating_sub(start) / 60;
    match minutes {
        0 => "opened just now".to_string(),
        1..=59 => format!("waited {minutes} min"),
        _ => format!("waited {} h {} min", minutes / 60, minutes % 60),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn now_secs() -> Option<u64> {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs())
}

#[cfg(target_arch = "wasm32")]
fn now_secs() -> Option<u64> {
    None
}

/// A quiet outlined button for an action beside the main one.
fn secondary_button(ui: &mut Ui, label: &str, enabled: bool) -> bool {
    let width = text_width(ui, label, semibold(10.0)) + 22.0;
    let (rect, response) = ui.allocate_exact_size(vec2(width, 22.0), if enabled { Sense::click() } else { Sense::hover() });
    let response = response.on_hover_cursor(if enabled { egui::CursorIcon::PointingHand } else { egui::CursorIcon::Default });
    a11y::button(&response, label);
    let fill = if enabled && response.hovered() { c().panel } else { c().soft };
    ui.painter().rect_filled(rect, CornerRadius::same(5), fill);
    ui.painter().rect_stroke(rect, CornerRadius::same(5), Stroke::new(1.0, if enabled { c().accent } else { c().border }), StrokeKind::Inside);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, semibold(10.0), if enabled { c().accent } else { c().faint });
    response.clicked() && enabled
}

/// Cancel an open record on a thread; the diff is read again once the node answers.
#[cfg(not(target_arch = "wasm32"))]
fn start_cancel(app: &mut FigApp, ctx: &egui::Context, job: String) {
    if app.sync.cancelling {
        return;
    }
    let node = match workspaces::active_node(app) {
        Ok(node) => node,
        Err(error) => {
            app.sync.error = Some(error);
            return;
        }
    };
    let request = json!({"id": 1, "method": "sync.cancel", "job": job}).to_string();
    let (tx, rx) = mpsc::channel();
    app.sync.cancel_rx = Some(rx);
    app.sync.cancelling = true;
    let context = ctx.clone();
    let spawned = std::thread::Builder::new().name("figcraft-sync-cancel".into()).spawn(move || {
        let reply = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| ask(&node.address, &request, STATUS_TIMEOUT)))
            .unwrap_or_else(|_| Err("The cancel request failed unexpectedly.".to_string()))
            .and_then(|text| {
                let value: Value = serde_json::from_str(&text).map_err(|e| format!("The node sent an unreadable answer: {e}"))?;
                if value["ok"] == true { Ok(()) } else { Err(value["error"].as_str().unwrap_or("The node refused the cancel.").to_string()) }
            });
        let _ = tx.send(reply);
        context.request_repaint();
    });
    if let Err(e) = spawned {
        app.sync.cancel_rx = None;
        app.sync.cancelling = false;
        app.sync.error = Some(format!("Could not cancel the record: {e}"));
    }
}

#[cfg(target_arch = "wasm32")]
fn start_cancel(app: &mut FigApp, _ctx: &egui::Context, _job: String) {
    app.toast("The node is only available in the desktop app.");
}

/// Take in the cancel thread's answer.
fn poll_cancel(app: &mut FigApp) {
    let received = match app.sync.cancel_rx.as_ref().map(Receiver::try_recv) {
        None | Some(Err(mpsc::TryRecvError::Empty)) => return,
        Some(Ok(reply)) => reply,
        Some(Err(mpsc::TryRecvError::Disconnected)) => Err("The cancel request ended without an answer.".to_string()),
    };
    app.sync.cancel_rx = None;
    app.sync.cancelling = false;
    match received {
        Ok(()) => {
            app.sync.error = None;
            app.sync.refresh = true;
            app.toast("Record cancelled: nothing was committed.");
        }
        Err(e) => app.sync.error = Some(e),
    }
}

/// A failed job's reason, kept in the widget so the next sync can be set up.
fn failed_card(ui: &mut Ui, reason: &str) {
    egui::Frame::NONE
        .fill(c().soft)
        .stroke(Stroke::new(1.0, c().danger))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin { left: 13, right: 13, top: 10, bottom: 10 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.label(egui::RichText::new("Burn failed").font(semibold(11.0)).color(c().danger));
            ui.add(egui::Label::new(egui::RichText::new(reason).font(mono(9.0)).color(c().danger)).wrap());
        });
}

/// A request error (a refused diff, a lost status poll).
fn error_block(ui: &mut Ui, error: &str) {
    egui::Frame::NONE
        .fill(c().soft)
        .stroke(Stroke::new(1.0, c().border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            paragraph(ui, error, 10.5, c().danger);
        });
}

/// The unfolded code section: the scope it is limited to, file and commit counts, the files, and
/// the service's note.
fn code_section(ui: &mut Ui, last: Option<&Last>, code: &CodeDiff) {
    let first = last.is_none();
    if !code.scope.is_empty() {
        let shown: Vec<&str> = code.scope.iter().take(FILES).map(String::as_str).collect();
        let mut line = format!("in {}", shown.join(", "));
        if code.scope.len() > FILES {
            line.push_str(&format!(" +{} more", code.scope.len() - FILES));
        }
        ui.add_space(7.0);
        ui.add(egui::Label::new(egui::RichText::new(line).font(regular(10.0)).color(c().faint)).wrap());
    }
    ui.add_space(7.0);
    if first {
        ui.label(egui::RichText::new("first Burn: the map is the baseline.").font(regular(10.5)).color(c().muted));
    } else if code.files.is_empty() && code.commits.is_empty() && code.note.as_deref().unwrap_or("").trim().is_empty() {
        ui.label(egui::RichText::new("No code changes since the last Burn.").font(regular(10.5)).color(c().muted));
    }
    for file in code.files.iter().take(FILES) {
        ui.add_space(7.0);
        file_row(ui, file);
    }
    if code.files.len() > FILES {
        ui.add_space(7.0);
        ui.label(egui::RichText::new(format!("+{} more files", code.files.len() - FILES)).font(regular(10.0)).color(c().faint));
    }
    if let Some(note) = code.note.as_deref().map(str::trim).filter(|note| !note.is_empty()) {
        ui.add_space(7.0);
        paragraph(ui, note, 10.0, c().faint);
    }
}

/// One changed file: `service/src/sync.rs   +240/−3`, one line.
fn file_row(ui: &mut Ui, file: &FileDelta) {
    let stats = format!("   +{}/−{}", file.added, file.removed);
    let room = (ui.available_width() - text_width(ui, &stats, regular(10.0))).max(40.0);
    let path = widgets::elide_start_font(ui, &file.path, room, regular(10.0));
    ui.label(egui::RichText::new(format!("{path}{stats}")).font(regular(10.0)).color(c().ink));
}

/// The agent button and its menu, shared with the composer; the pick, if any.
fn agent_button(ui: &mut Ui, state: &mut State, compose: &crate::compose::State) -> Option<Target> {
    let label = agent_menu::button_label(&compose.targets, compose.target.as_ref(), "Choose an agent");
    // Full width in the footer, as the design draws it.
    let width = ui.available_width();
    let response =
        agent_menu::button(ui, Id::new("sync_agent_button"), &compose.targets, compose.target.as_ref(), &label, vec2(width, 28.0), state.menu_open);
    if response.clicked() {
        state.menu_open = !state.menu_open;
        if state.menu_open {
            state.menu.opened();
        }
    }
    let options =
        agent_menu::Options { loading: compose.loading_targets(), error: compose.targets_error.as_deref(), ..agent_menu::Options::default() };
    agent_menu::popup(
        ui,
        Id::new("sync_targets"),
        &response,
        &mut state.menu_open,
        &mut state.menu,
        &compose.targets,
        compose.target.as_ref(),
        options,
    )
}

/// One scope choice in the widget's header.
fn scope_chip(ui: &mut Ui, label: &str, selected: bool, enabled: bool) -> bool {
    let width = text_width(ui, label, semibold(9.5)) + 18.0;
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(vec2(width, 19.0), sense);
    let response = if enabled { response.on_hover_cursor(egui::CursorIcon::PointingHand) } else { response };
    a11y::button(&response, &format!("Burn {}", label.to_lowercase()));
    let hot = enabled && response.hovered();
    let fill = if selected {
        classic_or(Color32::from_rgb(0xF1, 0xEC, 0xE7), c().accent_tint)
    } else if hot {
        c().soft
    } else {
        c().panel
    };
    ui.painter().rect_filled(rect, CornerRadius::same(5), fill);
    let border = if selected { c().accent } else { c().border };
    ui.painter().rect_stroke(rect, CornerRadius::same(5), Stroke::new(1.0, border), StrokeKind::Inside);
    let ink = if selected || hot { c().accent } else { c().muted };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, semibold(9.5), ink);
    response.clicked() && enabled && !selected
}

/// The note field: multiline, capped, with the design's placeholder.
fn note_field(ui: &mut Ui, note: &mut String) {
    // One line tall until the note needs more, as the design draws it.
    egui::Frame::NONE
        .fill(classic_or(Color32::from_rgb(0xF1, 0xF0, 0xED), c().panel))
        .stroke(Stroke::new(1.0, c().border))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(11, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical().id_salt("sync_note_scroll").max_height(64.0).show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(note)
                        .id(Id::new(NOTE_ID))
                        .frame(egui::Frame::NONE)
                        .font(regular(10.5))
                        .desired_width(f32::INFINITY)
                        .desired_rows(1)
                        .hint_text(egui::RichText::new("Note for this Burn…").font(regular(10.5)).color(c().faint)),
                )
            });
        });
    if note.chars().count() > MAX_NOTE_CHARS {
        *note = note.chars().take(MAX_NOTE_CHARS).collect();
    }
}

/// The widget's one action, full width with its ⌘↵ at the right: accent when it can run, soft
/// while it cannot.
fn action_button(ui: &mut Ui, label: &str, enabled: bool, busy: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), if enabled { Sense::click() } else { Sense::hover() });
    let response = response.on_hover_cursor(if enabled { egui::CursorIcon::PointingHand } else { egui::CursorIcon::Default });
    a11y::button(&response, label);
    let fill = if enabled {
        if response.hovered() { c().accent_hover } else { c().accent }
    } else {
        classic_or(Color32::from_rgb(0xE8, 0xE5, 0xE0), c().disabled_fill)
    };
    ui.painter().rect_filled(rect, CornerRadius::same(7), fill);
    let ink = if enabled { c().on_accent } else { c().faint };
    // The disc at the left: blank, spinning while the Burn runs; faded while the action can't run.
    let disc = Rect::from_center_size(pos2(rect.left() + 14.0 + 12.0, rect.center().y), vec2(24.0, 24.0));
    let mut painter = ui.painter().clone();
    if !enabled && !busy {
        painter.multiply_opacity(0.5);
    }
    let state = if busy { burn_disc::DiscState::Burning { t: ui.input(|input| input.time) } } else { burn_disc::DiscState::Blank };
    burn_disc::paint(&painter, disc, state, &c());
    let room = (rect.right() - 60.0 - (disc.right() + 10.0)).max(40.0);
    let shown = widgets::elide_font(ui, label, room, regular(13.0));
    ui.painter().text(pos2(disc.right() + 10.0, rect.center().y), Align2::LEFT_CENTER, shown, regular(13.0), ink);
    if !busy {
        let hint = format!("{}↵", if crate::shortcuts::platform() == figcraft_engine::keymap::Platform::Mac { "⌘" } else { "Ctrl+" });
        ui.painter().text(pos2(rect.right() - 14.0, rect.center().y), Align2::RIGHT_CENTER, hint, mono(8.5), ink.gamma_multiply(0.7));
    }
    response.clicked() && enabled
}

/// Show `row`'s card: open its page if another one is open, then fit the card (and room for its
/// chip) in the view. A deleted card has nowhere to go.
fn fly_to(app: &mut FigApp, row: &Row) {
    let Some(page) = row.page else { return };
    if row.shown.is_empty() {
        return;
    }
    let exists = app.session.doc().is_ok_and(|doc| doc.node(NodeId(page)).is_some_and(|node| node.is_page()));
    if !exists {
        return;
    }
    if app.session.page().ok().map(|page| page.0) != Some(page) && app.run("page.select", json!({"id": page})).is_none() {
        return;
    }
    let ids: Vec<NodeId> = row.shown.iter().map(|id| NodeId(*id)).collect();
    let Ok(ds) = app.session.ds_mut() else { return };
    let Some(bounds) = ds.doc.bounds_of(&ids) else { return };
    // A margin around the card, more below it for its chip, so the cards beside it stay in view.
    let pad = (bounds.width().max(bounds.height()) * 0.15).max(24.0);
    let fit = figcraft_doc::geom::Rect::new(bounds.x0 - pad, bounds.y0 - pad, bounds.x1 + pad, bounds.y1 + pad * 2.0);
    ds.view.fit(fit, 1.0);
}

/// The canvas hook while the pane is open: each waiting card on the open page wears its `+` / `~`
/// badge at its top-right corner, and the current card an amber outline with a dark chip under it
/// (`~ 3 / 14 · tabs · tab Mindmap · 6 separators`). Drawn by `canvas::show` on the canvas painter.
pub(crate) fn canvas_marks(app: &FigApp, painter: &egui::Painter) {
    if !app.sync.open {
        return;
    }
    let rows = app.sync.rows();
    let (Ok(doc), Ok(page)) = (app.session.doc(), app.session.page()) else { return };
    let total = rows.len();
    for (index, row) in rows.iter().enumerate() {
        let current = app.sync.current == Some(row.key());
        let left_out = row.left_out(&app.sync.excluded);
        let mut union: Option<Rect> = None;
        for id in &row.shown {
            let id = NodeId(*id);
            if doc.page_of(id) != Some(page) {
                continue;
            }
            let Some(quad) = crate::canvas::node_quad(app, doc, id) else { continue };
            let bounds = Rect::from_points(&quad);
            if !bounds.is_finite() {
                continue;
            }
            union = Some(union.map_or(bounds, |u| u.union(bounds)));
            if current {
                let ring = bounds.expand(5.0);
                painter.rect_stroke(ring, CornerRadius::same(6), Stroke::new(7.0, c().working.gamma_multiply(0.28)), StrokeKind::Outside);
                painter.rect_stroke(ring, CornerRadius::same(4), Stroke::new(2.5, c().working), StrokeKind::Inside);
            }
            let color = if left_out { row.mark.fill().gamma_multiply(0.5) } else { row.mark.fill() };
            let at = bounds.right_top();
            painter.circle_filled(at, 8.0, color);
            painter.circle_stroke(at, 8.0, Stroke::new(1.5, c().panel));
            painter.text(at, Align2::CENTER_CENTER, row.mark.glyph(), mono(10.0), row.mark.on_fill());
        }
        if let (true, Some(union)) = (current, union) {
            let text = chip_text(row, index, total);
            let galley = painter.layout_no_wrap(text, mono(9.0), c().panel);
            let chip = Rect::from_min_size(pos2(union.left(), union.bottom() + 14.0), galley.size() + vec2(20.0, 12.0));
            painter.rect_filled(chip, CornerRadius::same(5), c().ink);
            painter.galley(chip.min + vec2(10.0, 6.0), galley, c().panel);
        }
    }
}

/// The current card's chip: `~ 3 / 14 · tabs · tab Mindmap · 6 separators`, or the row's sub line
/// when it lists no layers.
pub fn chip_text(row: &Row, index: usize, total: usize) -> String {
    let mut parts = vec![format!("{} {} / {total}", row.mark.glyph(), index + 1)];
    if row.details.is_empty() {
        parts.push(row.sub.clone());
    } else {
        parts.extend(row.details.iter().take(3).map(|(layer, _)| layer.trim_start_matches(['+', '−', ' ']).to_string()));
    }
    parts.join(" · ")
}

/// Read the current document's snapshot and the page it is limited to (`None` for every page). The
/// doc and its current page must both exist.
fn snapshot_now(app: &FigApp) -> Result<(Snapshot, Option<u64>), String> {
    let doc = app.session.doc().map_err(|_| "Open a map to burn.".to_string())?;
    let page = app.session.page().map_err(|_| "This map has no page to burn.".to_string())?;
    if doc.node(page).is_none() {
        return Err("This map's page is not available to burn.".to_string());
    }
    let scope = (!app.sync.all_pages).then_some(page);
    let snapshot = Snapshot::of_pages(doc, scope);
    if snapshot.pages.is_empty() {
        return Err("This map has no pages to burn.".to_string());
    }
    Ok((snapshot, scope.map(|page| page.0)))
}

/// Rebuild the current snapshot and compare it with the one the diff was read from.
fn refresh_dirty(app: &mut FigApp) {
    let scope = app.sync.page.map(NodeId);
    let current = app.session.doc().ok().map(|doc| Snapshot::of_pages(doc, scope));
    let dirty = match (app.sync.snapshot.as_ref(), current.as_ref()) {
        (Some(baseline), Some(current)) => current.differs(baseline),
        (None, _) => false,
        (Some(_), None) => true,
    };
    app.sync.dirty = dirty;
}

/// Ask for the diff of `snapshot`, on a thread of its own.
#[cfg(not(target_arch = "wasm32"))]
fn start_diff(app: &mut FigApp, ctx: &egui::Context, snapshot: Snapshot, page: Option<u64>, workspace: String) {
    if app.sync.busy {
        return;
    }
    let node = match workspaces::active_node(app) {
        Ok(node) => node,
        Err(error) => {
            app.sync.error = Some(error);
            return;
        }
    };
    let request = json!({"id": 1, "method": "diff", "workspace": workspace, "session": node.session, "page": page}).to_string();
    // Job events (a record opening or ending) arrive for the node the diff came from.
    if !app.sync.starting && !app.sync.in_flight() {
        app.sync.node = Some(node.clone());
    }
    let (tx, rx) = mpsc::channel();
    app.sync.rx = Some(rx);
    app.sync.busy = true;
    app.sync.error = None;
    app.sync.snapshot = Some(snapshot);
    app.sync.page = page;
    app.sync.dirty = false;
    let address = node.address;
    let context = ctx.clone();
    let spawned = std::thread::Builder::new().name("figcraft-sync-diff".into()).spawn(move || {
        let reply = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| diff_request(&address, &request)))
            .unwrap_or_else(|_| Err("The diff request failed unexpectedly.".to_string()));
        let _ = tx.send(reply);
        context.request_repaint();
    });
    if let Err(e) = spawned {
        app.sync.rx = None;
        app.sync.busy = false;
        app.sync.error = Some(format!("Could not ask for the diff: {e}"));
    }
}

#[cfg(target_arch = "wasm32")]
fn start_diff(app: &mut FigApp, _ctx: &egui::Context, _snapshot: Snapshot, _page: Option<u64>, _workspace: String) {
    app.sync.busy = false;
    app.sync.error = Some("The node is only available in the desktop app.".to_string());
}

/// Ask the service to sync the map with the chosen target.
#[cfg(not(target_arch = "wasm32"))]
fn start_sync(app: &mut FigApp, ctx: &egui::Context, workspace: String, _snapshot: Snapshot, page: Option<u64>) {
    if app.sync.busy || app.sync.starting || app.sync.in_flight() {
        return;
    }
    let Some(target) = app.compose.target.clone() else {
        app.toast("Choose an agent to burn with.");
        return;
    };
    let node = match workspaces::active_node(app) {
        Ok(node) => node,
        Err(error) => {
            app.sync.error = Some(error);
            return;
        }
    };
    let note = app.sync.note.chars().take(MAX_NOTE_CHARS).collect::<String>();
    app.sync.node = Some(node.clone());
    // The unticked cards stay out of the frozen map and the agent's work: they keep waiting.
    let rows = app.sync.rows();
    let request = json!({
        "id": 1,
        "method": "sync",
        "workspace": workspace,
        "target": target.line(),
        "prompt": note,
        "session": node.session,
        "page": page,
        "exclude": app.sync.excluded_ids(&rows),
    })
    .to_string();
    let (tx, rx) = mpsc::channel();
    app.sync.job_rx = Some(rx);
    app.sync.busy = true;
    app.sync.starting = true;
    app.sync.error = None;
    app.sync.applied = app.sync.included(&rows);
    app.sync.job_scope = Some(page);
    let address = node.address;
    let context = ctx.clone();
    let spawned = std::thread::Builder::new().name("figcraft-sync-start".into()).spawn(move || {
        let reply = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sync_request(&address, &request)))
            .unwrap_or_else(|_| Err("The sync request failed unexpectedly.".to_string()));
        let _ = tx.send(JobMsg::Started(reply));
        context.request_repaint();
    });
    if let Err(e) = spawned {
        app.sync.job_rx = None;
        app.sync.busy = false;
        app.sync.starting = false;
        app.sync.error = Some(format!("Could not start the Burn: {e}"));
    }
}

#[cfg(target_arch = "wasm32")]
fn start_sync(app: &mut FigApp, _ctx: &egui::Context, _workspace: String, _snapshot: Snapshot, _page: Option<u64>) {
    app.sync.busy = false;
    app.sync.starting = false;
    app.toast("The node is only available in the desktop app.");
}

/// Poll a job's status once a second on a named thread until it settles, however long the agent
/// works: a sync has no time limit. The thread ends when the widget stops listening.
#[cfg(not(target_arch = "wasm32"))]
fn start_poll(app: &mut FigApp, ctx: &egui::Context, job_id: String) {
    let (tx, rx) = mpsc::channel();
    app.sync.job_rx = Some(rx);
    let Some(node) = app.sync.node.clone() else { return };
    let address = node.address;
    let context = ctx.clone();
    let spawned = std::thread::Builder::new().name("figcraft-sync-status".into()).spawn(move || {
        let mut misses = 0u8;
        loop {
            std::thread::sleep(POLL_EVERY);
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| status_request(&address, &job_id, &node.session))) {
                Ok(Ok(reply)) => {
                    let settled = !reply.ok || reply.job.as_ref().is_none_or(|job| !job.in_flight());
                    if tx.send(JobMsg::Status(Ok(reply))).is_err() {
                        return;
                    }
                    context.request_repaint();
                    if settled {
                        return;
                    }
                    misses = 0;
                }
                Ok(Err(e)) => {
                    misses = misses.saturating_add(1);
                    if misses >= POLL_MISSES {
                        let _ = tx.send(JobMsg::Status(Err(e)));
                        context.request_repaint();
                        return;
                    }
                }
                Err(_) => {
                    let _ = tx.send(JobMsg::Status(Err("The sync status request failed unexpectedly.".to_string())));
                    context.request_repaint();
                    return;
                }
            }
        }
    });
    if let Err(e) = spawned {
        app.sync.job_rx = None;
        app.sync.error = Some(format!("Could not poll the Burn: {e}"));
        if let Some(job) = app.sync.job.as_mut() {
            job.state = "failed".to_string();
            job.reason = Some(format!("Could not poll the Burn: {e}"));
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn start_poll(app: &mut FigApp, _ctx: &egui::Context, _job_id: String) {
    app.sync.job_rx = None;
}

/// Take in the diff thread's answer.
fn poll_diff(app: &mut FigApp) {
    let received = app.sync.rx.as_ref().map(Receiver::try_recv);
    let received = match received {
        None => return,
        Some(Ok(reply)) => reply,
        Some(Err(mpsc::TryRecvError::Empty)) => return,
        Some(Err(mpsc::TryRecvError::Disconnected)) => {
            app.sync.rx = None;
            app.sync.busy = false;
            app.sync.error = Some("The diff request ended without an answer.".to_string());
            return;
        }
    };
    app.sync.rx = None;
    app.sync.busy = false;
    match received {
        Ok(reply) if reply.ok => {
            app.sync.last = reply.last.clone();
            app.sync.diff = Some(reply);
            app.sync.error = None;
            app.sync.dirty = false;
        }
        Ok(reply) => {
            app.sync.error = Some(reply.error.unwrap_or_else(|| "The node refused the diff.".to_string()));
        }
        Err(e) => app.sync.error = Some(e),
    }
}

/// Take in the sync-start or status thread's next answer.
fn poll_job(app: &mut FigApp, ctx: &egui::Context) {
    let received = app.sync.job_rx.as_ref().map(Receiver::try_recv);
    let received = match received {
        None => return,
        Some(Ok(message)) => message,
        Some(Err(mpsc::TryRecvError::Empty)) => return,
        Some(Err(mpsc::TryRecvError::Disconnected)) => {
            app.sync.job_rx = None;
            app.sync.busy = false;
            app.sync.starting = false;
            fail_flight(app, "The sync status request ended without an answer.".to_string());
            return;
        }
    };
    apply_job(app, ctx, received);
}

#[cfg(not(target_arch = "wasm32"))]
pub fn node_event(app: &mut FigApp, ctx: &egui::Context, address: &str, mut event: Value) {
    if app.sync.node.as_ref().is_none_or(|node| node.address != address) {
        return;
    }
    if app.sync.starting {
        if app.sync.pushed_jobs.len() < 32 {
            app.sync.pushed_jobs.push(event);
        }
        return;
    }
    if app.sync.job.as_ref().is_none_or(|job| event["job"]["id"].as_str() != Some(job.id.as_str()) || !job.in_flight()) {
        // Another job: a record opened or ended, or the one blocking Sync changed. Re-read.
        let blocking = app.sync.blocker().is_some_and(|open| event["job"]["id"].as_str() == Some(open.id.as_str()));
        let record = matches!(event["job"]["kind"].as_str(), Some("explain" | "fix"));
        if blocking || record {
            app.sync.refresh = true;
            ctx.request_repaint();
        }
        return;
    }
    event["ok"] = Value::Bool(true);
    match serde_json::from_value::<StatusReply>(event) {
        Ok(reply) => apply_job(app, ctx, JobMsg::Status(Ok(reply))),
        Err(_) => {
            app.sync.error = Some("The node sent an invalid job update.".into());
        }
    }
}

fn apply_job(app: &mut FigApp, ctx: &egui::Context, received: JobMsg) {
    match received {
        JobMsg::Started(result) => {
            app.sync.busy = false;
            app.sync.starting = false;
            match result {
                Ok(reply) if reply.ok => match reply.job.filter(|job| !job.id.trim().is_empty()) {
                    Some(job) => {
                        app.sync.error = None;
                        app.sync.note.clear();
                        app.sync.job = Some(job.clone());
                        start_poll(app, ctx, job.id);
                    }
                    None => app.sync.error = Some("The node started no sync job.".to_string()),
                },
                Ok(reply) => {
                    app.sync.error = Some(reply.error.unwrap_or_else(|| "The node refused the sync.".to_string()));
                    // A refusal is often a record or Sync already running: show it.
                    app.sync.refresh = true;
                }
                Err(e) => app.sync.error = Some(e),
            }
        }
        JobMsg::Status(result) => match result {
            Ok(reply) if reply.ok => {
                let Some(job) = reply.job else {
                    app.sync.job_rx = None;
                    fail_flight(app, "The node sent a status with no job.".to_string());
                    return;
                };
                if let Some(last) = reply.last {
                    app.sync.last = Some(last);
                }
                let state = job.state.trim().to_ascii_lowercase();
                if job.in_flight() {
                    app.sync.job = Some(job);
                    return;
                }
                app.sync.job_rx = None;
                app.sync.job = Some(job);
                match state.as_str() {
                    "done" => {
                        // The applied cards are gone from the new baseline: start the next diff
                        // from the map as it is now (the left-out cards still wait, still unticked).
                        app.sync.diff = None;
                        app.sync.snapshot = None;
                        app.sync.refresh = true;
                        app.sync.error = None;
                        app.sync.current = None;
                        let commit = app.sync.job.as_ref().and_then(|job| job.commit.clone());
                        app.toast_burned(burned_toast(commit.as_deref(), app.sync.applied));
                        // An exploration this Sync covered is built: the state of the art now.
                        if let Some(scope) = app.sync.job_scope
                            && !app.sync.job.as_ref().is_some_and(Job::is_record)
                        {
                            crate::page_tree::built(app, scope);
                        }
                    }
                    "failed" => {
                        let reason = app.sync.job.as_ref().and_then(|job| job.reason.clone()).unwrap_or_else(|| "The Burn failed.".to_string());
                        app.toast(format!("Burn failed: {reason}"));
                    }
                    other => {
                        let message = if other.is_empty() {
                            "The node sent a sync status this app can't read.".to_string()
                        } else {
                            format!("The node sent an unknown sync state ({other}).")
                        };
                        app.sync.error = Some(message);
                    }
                }
            }
            Ok(reply) => {
                app.sync.job_rx = None;
                let message = reply.error.unwrap_or_else(|| "The node refused the sync status.".to_string());
                fail_flight(app, message);
            }
            Err(e) => {
                app.sync.job_rx = None;
                fail_flight(app, e);
            }
        },
    }
}

/// Stop showing a flight as running when its status channel is gone.
fn fail_flight(app: &mut FigApp, reason: String) {
    let mut marked = false;
    if let Some(job) = app.sync.job.as_mut()
        && job.in_flight()
    {
        job.state = "failed".to_string();
        job.reason = Some(reason.clone());
        marked = true;
    }
    if !marked {
        app.sync.error = Some(reason);
    }
}

/// The `diff` request and its reply.
#[cfg(not(target_arch = "wasm32"))]
fn diff_request(address: &str, request: &str) -> Result<DiffReply, String> {
    let line = ask(address, request, DIFF_TIMEOUT)?;
    serde_json::from_str(&line).map_err(|e| format!("The node sent a diff this app can't read: {e}"))
}

/// The `sync` request and its reply.
#[cfg(not(target_arch = "wasm32"))]
fn sync_request(address: &str, request: &str) -> Result<SyncReply, String> {
    let line = ask(address, request, SYNC_TIMEOUT)?;
    serde_json::from_str(&line).map_err(|e| format!("The node sent a sync this app can't read: {e}"))
}

/// The `sync_status` request and its reply.
#[cfg(not(target_arch = "wasm32"))]
fn status_request(address: &str, job: &str, session: &str) -> Result<StatusReply, String> {
    let request = json!({"id": 1, "method": "sync_status", "session":session, "job": job}).to_string();
    let line = ask(address, &request, STATUS_TIMEOUT)?;
    serde_json::from_str(&line).map_err(|e| format!("The node sent a sync status this app can't read: {e}"))
}

/// Ask the node one newline-terminated JSON request and return its reply line.
#[cfg(not(target_arch = "wasm32"))]
fn ask(address: &str, request: &str, timeout: Duration) -> Result<String, String> {
    crate::node::ask(address, request, timeout)
}

/// The first seven characters of a commit hash.
fn short_commit(commit: &str) -> String {
    commit.trim().chars().take(7).collect()
}

fn kind_label(kind: &str) -> &str {
    let kind = kind.trim();
    if kind.is_empty() { "agent" } else { kind }
}

fn paragraph(ui: &mut Ui, text: &str, size: f32, color: Color32) {
    ui.add(egui::Label::new(egui::RichText::new(text).font(regular(size)).color(color)).wrap());
}

fn divider(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().line_segment([rect.left_center(), rect.right_center()], Stroke::new(1.0, c().border));
}

fn text_width(ui: &Ui, text: &str, font: FontId) -> f32 {
    ui.painter().layout_no_wrap(text.to_string(), font, Color32::WHITE).size().x
}

fn mono(size: f32) -> FontId {
    FontId::monospace(size)
}

#[cfg(test)]
#[path = "sync_cards_tests.rs"]
mod cards_tests;
