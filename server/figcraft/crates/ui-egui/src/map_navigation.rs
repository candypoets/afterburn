//! Read-only map navigation. The caller owns shortcut/text-focus and overlay routing.
//! Search state belongs to the egui context; no document or composer state is used for it.

use std::collections::HashSet;

use egui::{Id, Key, Modifiers};
use figcraft_doc::{Document, Node, NodeId, NodeKind};

use crate::{FigApp, page_tree};

#[derive(Clone, Default)]
struct State {
    open: bool,
    document: Option<u64>,
    query: String,
    selected: Option<NodeId>,
    focus: bool,
}

fn state_id() -> Id {
    Id::new("map_navigation_search")
}

fn input_id() -> Id {
    state_id().with("query")
}

fn state(ctx: &egui::Context) -> State {
    ctx.data(|data| data.get_temp::<State>(state_id())).unwrap_or_default()
}

/// Whether the map search overlay currently owns keyboard routing.
pub fn search_open(ctx: &egui::Context) -> bool {
    state(ctx).open
}

/// Close search, returning whether it consumed the cancellation. Does not clear selection.
/// Route Escape here before dispatching `map.selectNone`.
pub fn close_search(ctx: &egui::Context) -> bool {
    let mut search = state(ctx);
    if !search.open {
        return false;
    }
    search.open = false;
    search.focus = false;
    ctx.data_mut(|data| data.insert_temp(state_id(), search));
    ctx.memory_mut(|memory| memory.surrender_focus(input_id()));
    ctx.request_repaint();
    true
}

fn visible(node: &Node) -> bool {
    node.visible && node.opacity.is_finite() && node.opacity > 0.0 && !matches!(node.kind, NodeKind::Section { contents_hidden: true })
}

fn decoration(node: &Node) -> bool {
    let name = node.name.trim().to_ascii_lowercase();
    matches!(name.as_str(), "page bounds" | "page background" | "background" | "decoration" | "divider" | "separator")
        || matches!(node.kind, NodeKind::Line | NodeKind::Slice)
}

/// Visible descendants in reading order, with cycle protection for imported documents.
fn content_nodes(doc: &Document, root: NodeId) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Some(node) = doc.node(id) else { continue };
        if !visible(node) || decoration(node) {
            continue;
        }
        out.push(id);
        stack.extend(node.children.iter().rev().copied());
    }
    reading_order(doc, &mut out);
    out
}

fn reading_order(doc: &Document, ids: &mut [NodeId]) {
    // Stable sort: coincident layers retain their document order. Non-finite imported positions
    // sort last, rather than entering a partial float comparison.
    ids.sort_by_cached_key(|id| {
        let bounds = doc.world_bounds(*id);
        let coord = |value: f64| if value.is_finite() { value } else { f64::MAX };
        bounds.map(|rect| (Ordered(coord(rect.y0)), Ordered(coord(rect.x0)))).unwrap_or((Ordered(f64::MAX), Ordered(f64::MAX)))
    });
}

#[derive(Clone, Copy)]
struct Ordered(f64);

impl PartialEq for Ordered {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}
impl Eq for Ordered {}
impl PartialOrd for Ordered {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Ordered {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

fn has_content(doc: &Document, id: NodeId) -> bool {
    content_nodes(doc, id).iter().any(|id| {
        doc.node(*id).is_some_and(|node| {
            node.kind.text().is_some_and(|text| !text.characters.trim().is_empty()) || node.plugin_data.contains_key(crate::new_page::DETAIL_PAGE_KEY)
        })
    })
}

fn container(node: &Node) -> bool {
    matches!(node.kind, NodeKind::Frame | NodeKind::Group | NodeKind::Component { .. } | NodeKind::Instance { .. })
}

/// The boxes under `point`: the topmost card first, then each nested content box (container
/// or text) down to the innermost. Clicks select along this chain; it never edits.
pub(crate) fn boxes_at(doc: &Document, page: NodeId, point: figcraft_geom::Point) -> Vec<NodeId> {
    let under = |id: NodeId| doc.world_bounds(id).is_some_and(|b| b.contains(point));
    let Some(card) = cards(doc, page).into_iter().rev().find(|id| under(*id)) else { return Vec::new() };
    let mut chain = vec![card];
    while let Some(next) = chain.last().and_then(|id| doc.node(*id)).and_then(|node| {
        node.children.iter().rev().copied().find(|id| {
            doc.node(*id).is_some_and(|child| visible(child) && !decoration(child) && (container(child) || child.kind.text().is_some()))
                && has_content(doc, *id)
                && under(*id)
        })
    }) {
        chain.push(next);
    }
    chain
}

/// The box a click on `chain` selects: the innermost when `deep` (Cmd/Ctrl), else the one beside
/// the current selection (a sibling, so once inside a card its boxes stay one click away), else
/// the card itself.
pub(crate) fn click_target(doc: &Document, chain: &[NodeId], selection: &[NodeId], deep: bool) -> Option<NodeId> {
    if deep {
        return chain.last().copied();
    }
    let parent = |id: NodeId| doc.node(id).and_then(|node| node.parent);
    chain.iter().rev().copied().find(|id| selection.iter().any(|selected| parent(*selected) == parent(*id))).or_else(|| chain.first().copied())
}

/// The box one level inside `current` along `chain`: what a double-click selects.
pub(crate) fn inner(chain: &[NodeId], current: NodeId) -> Option<NodeId> {
    let at = chain.iter().position(|id| *id == current)?;
    chain.get(at + 1).copied()
}

/// Meaningful visible cards on a page, top-to-bottom then left-to-right. A content container
/// is one card, not a separate stop for every label inside it. Sections are layout containers;
/// bare text is a card except for conventional page headings/footers. Locked content is readable.
pub fn cards(doc: &Document, page: NodeId) -> Vec<NodeId> {
    let Some(root) = doc.node(page).filter(|node| node.is_page() && visible(node)) else { return Vec::new() };
    let mut stack: Vec<_> = root.children.iter().rev().copied().collect();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Some(node) = doc.node(id) else { continue };
        if !visible(node) || decoration(node) {
            continue;
        }
        let container = container(node);
        let page_label = matches!(node.name.trim().to_ascii_lowercase().as_str(), "title" | "page title" | "lede" | "foot" | "footer" | "eyebrow");
        let linked = node.plugin_data.contains_key(crate::new_page::DETAIL_PAGE_KEY);
        if (container || linked || (node.kind.text().is_some() && !page_label)) && has_content(doc, id) {
            out.push(id);
        } else {
            stack.extend(node.children.iter().rev().copied());
        }
    }
    reading_order(doc, &mut out);
    out
}

/// Shared pointer/F8 marquee semantics: intersect readable cards, including locked cards and
/// cards inside sections. Shift toggles against the selection captured before the gesture.
pub(crate) fn box_selection(doc: &Document, page: NodeId, rect: figcraft_geom::Rect, base: &[NodeId], toggle: bool) -> Vec<NodeId> {
    if ![rect.x0, rect.y0, rect.x1, rect.y1].iter().all(|value| value.is_finite()) {
        return Vec::new();
    }
    cards(doc, page)
        .into_iter()
        .filter(|id| {
            let hit = doc.world_bounds(*id).is_some_and(|b| {
                [b.x0, b.y0, b.x1, b.y1].iter().all(|value| value.is_finite())
                    && b.x0 <= rect.x1
                    && b.x1 >= rect.x0
                    && b.y0 <= rect.y1
                    && b.y1 >= rect.y0
            });
            if toggle { hit != base.contains(id) } else { hit }
        })
        .collect()
}

/// Mirrors the actual row order in page_tree::show, including its first-level stack order.
fn pages(doc: &Document) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for root in page_tree::roots(doc) {
        if !seen.insert(root) {
            continue;
        }
        if openable(doc, root) {
            out.push(root);
        }
        let mut stack = page_tree::children(doc, root);
        while let Some(page) = stack.pop() {
            if !seen.insert(page) {
                continue;
            }
            if openable(doc, page) {
                out.push(page);
            }
            stack.extend(page_tree::children(doc, page).into_iter().rev());
        }
    }
    out
}

fn openable(doc: &Document, page: NodeId) -> bool {
    doc.pages.contains(&page) && !doc.is_divider(page) && doc.node(page).is_some_and(|node| node.is_page() && visible(node))
}

fn adjacent(ids: &[NodeId], current: Option<NodeId>, forward: bool, wrap: bool) -> Option<NodeId> {
    let Some(index) = current.and_then(|id| ids.iter().position(|candidate| *candidate == id)) else {
        return if forward { ids.first() } else { ids.last() }.copied();
    };
    if forward {
        ids.get(index + 1).or_else(|| if wrap { ids.first() } else { None }).copied()
    } else {
        index.checked_sub(1).and_then(|index| ids.get(index)).or_else(|| if wrap { ids.last() } else { None }).copied()
    }
}

fn selected_cards(doc: &Document, page: NodeId, selection: &[NodeId]) -> Vec<NodeId> {
    cards(doc, page).into_iter().filter(|card| selection.iter().any(|id| card == id || doc.is_ancestor(*card, *id))).collect()
}

pub(crate) fn detail(doc: &Document, page: NodeId, selection: &[NodeId]) -> Option<NodeId> {
    let selected = selected_cards(doc, page, selection);
    // Enter is unambiguous only for one selected card.
    let [card] = selected.as_slice() else { return None };
    let linked = doc.node(*card)?.plugin_data.get(crate::new_page::DETAIL_PAGE_KEY).and_then(|value| value.parse::<u64>().ok()).map(NodeId);
    linked
        .filter(|id| openable(doc, *id))
        .or_else(|| page_tree::children(doc, page).into_iter().find(|child| page_tree::source(doc, *child) == Some(*card) && openable(doc, *child)))
}

fn page_label(doc: &Document, page: NodeId) -> String {
    page_tree::path(doc, page).iter().filter_map(|id| doc.node(*id).map(|node| node.name.as_str())).collect::<Vec<_>>().join(" / ")
}

fn card_text(doc: &Document, card: NodeId) -> String {
    content_nodes(doc, card)
        .iter()
        .filter_map(|id| doc.node(*id)?.kind.text().map(|text| text.characters.trim()))
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn copy_text(doc: &Document, page: NodeId, selection: &[NodeId]) -> String {
    selected_cards(doc, page, selection)
        .iter()
        .filter_map(|card| {
            let node = doc.node(*card)?;
            let text = card_text(doc, *card);
            let mut block = format!("{}\nMap: {}\nPage: {}\nCard: {}", node.name.trim(), doc.name, page_label(doc, page), card.0);
            // Sources are usually text layers, already included below. Preserve explicit source
            // metadata too, without leaking unrelated plugin state into the clipboard.
            for (key, value) in &node.plugin_data {
                if matches!(
                    key.as_str(),
                    "source" | "context" | "afterburn.source" | "afterburn.source_path" | "afterburn.source_url" | "afterburn.context"
                ) && !value.trim().is_empty()
                {
                    block.push_str(&format!("\n{key}: {value}"));
                }
            }
            if !text.is_empty() && text != node.name.trim() {
                block.push_str("\n\n");
                block.push_str(&text);
            }
            Some(block)
        })
        .collect::<Vec<_>>()
        .join("\n\n---\n\n")
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SearchResult {
    page: NodeId,
    card: NodeId,
    name: String,
    path: String,
    preview: String,
}

fn search(doc: &Document, query: &str) -> Vec<SearchResult> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    pages(doc)
        .into_iter()
        .flat_map(|page| {
            let query = &query;
            cards(doc, page).into_iter().filter_map(move |card| {
                let node = doc.node(card)?;
                let text = card_text(doc, card);
                let matched = node.name.to_lowercase().contains(query) || text.to_lowercase().contains(query);
                matched.then(|| SearchResult {
                    page,
                    card,
                    name: node.name.clone(),
                    path: page_label(doc, page),
                    preview: text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(180).collect(),
                })
            })
        })
        .collect()
}

fn open_result(app: &mut FigApp, result: &SearchResult) -> bool {
    let valid = app.session.doc().is_ok_and(|doc| openable(doc, result.page) && cards(doc, result.page).contains(&result.card));
    if !valid {
        return false;
    }
    if app.session.page().ok() != Some(result.page) && app.run("page.select", serde_json::json!({"id": result.page.0})).is_none() {
        return false;
    }
    app.session.set_selection(vec![result.card]);
    true
}

/// Open map search with `query` already typed (the Actions palette's "Search this map for …").
/// Like `map.search`, does nothing without an open map; returns whether search opened.
pub fn open_search(app: &mut FigApp, ctx: &egui::Context, query: &str) -> bool {
    if app.session.ds().is_err() {
        return false;
    }
    run(app, ctx, "map.search");
    let mut search = state(ctx);
    if !search.open {
        return false;
    }
    search.query = query.trim().to_string();
    search.selected = None;
    ctx.data_mut(|data| data.insert_temp(state_id(), search));
    true
}

/// Dispatch the map navigation slice. `true` means the ID is owned, even if there is no
/// available target (for example Parent page on a root). Focus policy belongs to the caller.
pub fn run(app: &mut FigApp, ctx: &egui::Context, id: &str) -> bool {
    match id {
        "map.search" => {
            let document = app.session.ds().ok().map(|ds| ds.uid);
            if document.is_none() {
                return true;
            }
            let mut search = state(ctx);
            if search.document != document {
                search = State::default();
            }
            search.document = document;
            search.open = true;
            search.focus = true;
            ctx.data_mut(|data| data.insert_temp(state_id(), search));
        }
        "map.pageTree" => {
            app.page_tree.open = !app.page_tree.open;
            app.page_tree.just_opened = app.page_tree.open;
        }
        "map.selectNone" => app.session.set_selection(Vec::new()),
        "map.selectAll" | "map.nextCard" | "map.previousCard" => {
            if let (Ok(doc), Ok(page)) = (app.session.doc(), app.session.page()) {
                let available = cards(doc, page);
                let selected = selected_cards(doc, page, &app.session.selection());
                let ids = if id == "map.selectAll" {
                    available
                } else {
                    let forward = id == "map.nextCard";
                    let anchor = if forward { selected.last() } else { selected.first() }.copied();
                    adjacent(&available, anchor, forward, true).into_iter().collect()
                };
                app.session.set_selection(ids);
            }
        }
        "map.previousPage" | "map.nextPage" | "map.openDetail" | "map.parentPage" => {
            let target = app.session.doc().ok().and_then(|doc| {
                let page = app.session.page().ok()?;
                match id {
                    "map.openDetail" => detail(doc, page, &app.session.selection()),
                    "map.parentPage" => page_tree::parent(doc, page).filter(|id| openable(doc, *id)),
                    _ => adjacent(&pages(doc), Some(page), id == "map.nextPage", false),
                }
            });
            if let Some(page) = target {
                app.run("page.select", serde_json::json!({"id": page.0}));
            }
        }
        "map.copy" => {
            if let (Ok(doc), Ok(page)) = (app.session.doc(), app.session.page()) {
                let text = copy_text(doc, page, &app.session.selection());
                if !text.is_empty() {
                    crate::egui_copy_text(ctx, &text);
                }
            }
        }
        _ => return false,
    }
    ctx.request_repaint();
    true
}

/// Render search once per frame. The parent must suppress canvas shortcuts while it is open,
/// and give it priority over canvas selection clearing when routing Escape.
pub fn show(app: &mut FigApp, ctx: &egui::Context) {
    let mut state = state(ctx);
    if !state.open {
        return;
    }
    if state.document != app.session.ds().ok().map(|ds| ds.uid) {
        close_search(ctx);
        return;
    }
    if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape)) {
        close_search(ctx);
        return;
    }
    // Consume these before TextEdit gets them: otherwise it handles Enter/vertical arrows
    // as field editing and the highlighted result never moves or opens.
    let down = ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowDown));
    let up = ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowUp));
    let enter = ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter));
    let tokens = crate::theme::Tokens::get(ctx);
    let mut close = false;
    let mut pick = None;
    let just_opened = state.focus;
    let area = egui::Area::new(state_id().with("overlay"))
        .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 90.0))
        .order(egui::Order::Foreground)
        .constrain(true)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).fill(tokens.panel).show(ui, |ui| {
                ui.set_width(480.0);
                ui.horizontal(|ui| {
                    ui.heading("Search this map");
                    close = ui.button("Close").clicked();
                });
                let input = ui
                    .add(egui::TextEdit::singleline(&mut state.query).id(input_id()).hint_text("Card names and text…").desired_width(f32::INFINITY));
                if state.focus {
                    input.request_focus();
                    state.focus = false;
                }
                if input.changed() {
                    state.selected = None;
                }
                let results = app.session.doc().map(|doc| search(doc, &state.query)).unwrap_or_default();
                if !results.iter().any(|result| Some(result.card) == state.selected) {
                    state.selected = results.first().map(|result| result.card);
                }
                let ids: Vec<_> = results.iter().map(|result| result.card).collect();
                let mut moved = false;
                for (pressed, forward) in [(down, true), (up, false)] {
                    if pressed {
                        state.selected = adjacent(&ids, state.selected, forward, true);
                        moved = true;
                    }
                }
                if enter {
                    pick = results.iter().find(|result| Some(result.card) == state.selected).cloned();
                }
                if state.query.trim().is_empty() {
                    ui.label("Search card names and text across this map’s pages.");
                } else if results.is_empty() {
                    ui.label("No matching cards.");
                } else {
                    ui.label(format!("{} matching cards · ↑/↓ to choose · Enter to open", results.len()));
                }
                egui::ScrollArea::vertical()
                    .id_salt(state_id().with("results"))
                    .max_height((ctx.content_rect().height() - 240.0).clamp(100.0, 360.0))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for result in &results {
                            ui.push_id((result.page.0, result.card.0), |ui| {
                                let selected = state.selected == Some(result.card);
                                let label = format!("{}\n{}\n{}", result.name, result.path, result.preview);
                                let response = ui.selectable_label(selected, label);
                                if selected && moved {
                                    response.scroll_to_me(Some(egui::Align::Center));
                                }
                                if response.clicked() {
                                    state.selected = Some(result.card);
                                    pick = Some(result.clone());
                                }
                            });
                        }
                    });
            });
        });
    let outside = !just_opened && ctx.input(|input| input.pointer.any_pressed()) && !area.response.contains_pointer();
    ctx.data_mut(|data| data.insert_temp(state_id(), state));
    if let Some(result) = pick {
        close |= open_result(app, &result);
    }
    if close || outside {
        close_search(ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use figcraft_doc::TextData;
    use figcraft_engine::Session;

    fn app() -> FigApp {
        FigApp::new(Session::new(), Box::new(crate::NoServices))
    }

    fn card(doc: &mut Document, page: NodeId, name: &str, text: &str, x: f64, y: f64) -> NodeId {
        let card = doc.create(page, NodeKind::Frame, name, x, y, 180.0, 100.0).unwrap();
        doc.create(card, NodeKind::Text(TextData::new(text)), "body", 8.0, 8.0, 160.0, 80.0).unwrap();
        card
    }

    fn subpage(doc: &mut Document, parent: NodeId, name: &str) -> NodeId {
        let child = doc.add_page(name);
        doc.node_mut(child).unwrap().plugin_data.insert(page_tree::PARENT_KEY.into(), parent.0.to_string());
        child
    }

    #[test]
    fn cards_skip_decoration_and_hidden_subtrees_and_use_reading_order() {
        let mut doc = Document::new("Map");
        let page = doc.pages[0];
        doc.create(page, NodeKind::Rectangle, "Page bounds", 0.0, 0.0, 900.0, 900.0).unwrap();
        doc.create(page, NodeKind::Line, "Connector", 0.0, 0.0, 100.0, 1.0).unwrap();
        doc.create(page, NodeKind::Text(TextData::new("Heading")), "title", 0.0, 0.0, 100.0, 30.0).unwrap();
        doc.create(page, NodeKind::Frame, "Empty decoration", 0.0, 0.0, 100.0, 30.0).unwrap();
        let bottom = card(&mut doc, page, "Bottom", "Read last", 0.0, 300.0);
        let right = card(&mut doc, page, "Right", "Read second", 250.0, 100.0);
        let left = card(&mut doc, page, "Left", "Read first", 0.0, 100.0);
        doc.node_mut(left).unwrap().locked = true;
        let hidden = card(&mut doc, page, "Hidden", "Do not select", 0.0, 50.0);
        doc.node_mut(hidden).unwrap().visible = false;
        let transparent = card(&mut doc, page, "Transparent", "Do not select", 0.0, 60.0);
        doc.node_mut(transparent).unwrap().opacity = 0.0;
        let section = doc.create(page, NodeKind::Section { contents_hidden: true }, "Collapsed", 0.0, 0.0, 300.0, 300.0).unwrap();
        card(&mut doc, section, "In collapsed section", "Hidden too", 0.0, 0.0);
        assert_eq!(cards(&doc, page), vec![left, right, bottom]);
        assert!(cards(&doc, left).is_empty());
    }

    #[test]
    fn section_contents_are_cards_and_equal_positions_keep_document_order() {
        let mut doc = Document::new("Map");
        let page = doc.pages[0];
        let section = doc.create(page, NodeKind::Section { contents_hidden: false }, "Layout", 200.0, 100.0, 300.0, 300.0).unwrap();
        let first = card(&mut doc, section, "First", "A", 0.0, 0.0);
        let second = card(&mut doc, section, "Second", "B", 0.0, 0.0);
        assert_eq!(cards(&doc, page), vec![first, second]);
    }

    #[test]
    fn page_order_matches_visible_tree_and_omits_dividers() {
        let mut doc = Document::new("Map");
        let root = doc.pages[0];
        let a = subpage(&mut doc, root, "A");
        let b = subpage(&mut doc, root, "B");
        let b1 = subpage(&mut doc, b, "B1");
        let b2 = subpage(&mut doc, b, "B2");
        doc.add_page("- Divider");
        let next_root = doc.add_page("Next root");
        assert_eq!(pages(&doc), vec![root, b, b1, b2, a, next_root]);
        assert_eq!(adjacent(&pages(&doc), Some(root), false, false), None);
        assert_eq!(adjacent(&pages(&doc), Some(next_root), true, false), None);
    }

    #[test]
    fn card_traversal_wraps_and_empty_lists_are_safe() {
        let ids = [NodeId(1), NodeId(2), NodeId(3)];
        assert_eq!(adjacent(&ids, None, true, true), Some(ids[0]));
        assert_eq!(adjacent(&ids, None, false, true), Some(ids[2]));
        assert_eq!(adjacent(&ids, Some(ids[2]), true, true), Some(ids[0]));
        assert_eq!(adjacent(&ids, Some(ids[0]), false, true), Some(ids[2]));
        assert_eq!(adjacent(&[], None, false, true), None);
    }

    #[test]
    fn search_matches_names_and_visible_unicode_text_across_pages() {
        let mut doc = Document::new("Map");
        let page = doc.pages[0];
        let named = card(&mut doc, page, "Résumé", "First body", 0.0, 0.0);
        let child = subpage(&mut doc, page, "Detail");
        let textual = card(&mut doc, child, "Other", "A RÉSUMÉ in the body", 0.0, 0.0);
        let hidden = card(&mut doc, child, "Résumé hidden", "RÉSUMÉ", 0.0, 0.0);
        doc.node_mut(hidden).unwrap().visible = false;
        let results = search(&doc, "  résumé  ");
        assert_eq!(results.iter().map(|result| (result.page, result.card)).collect::<Vec<_>>(), vec![(page, named), (child, textual)]);
        assert!(results[1].path.ends_with(" / Detail"));
        assert!(search(&doc, " ").is_empty());
        assert!(search(&doc, "missing").is_empty());
    }

    #[test]
    fn copy_deduplicates_card_and_descendant_and_preserves_source_context() {
        let mut doc = Document::new("Example map");
        let page = doc.pages[0];
        let first = card(&mut doc, page, "Evidence", "Readable body", 0.0, 0.0);
        let child = doc.node(first).unwrap().children[0];
        doc.node_mut(first).unwrap().plugin_data.insert("afterburn.source".into(), "src/example.rs:12".into());
        let hidden = doc.create(first, NodeKind::Text(TextData::new("Hidden body")), "hidden", 0.0, 0.0, 80.0, 20.0).unwrap();
        doc.node_mut(hidden).unwrap().visible = false;
        let text = copy_text(&doc, page, &[child, first, child]);
        assert_eq!(text.matches("Readable body").count(), 1);
        assert!(text.contains("Map: Example map"));
        assert!(text.contains(&format!("Page: {}", doc.node(page).unwrap().name)));
        assert!(text.contains("afterburn.source: src/example.rs:12"));
        assert!(!text.contains("Hidden body"));
        assert!(copy_text(&doc, page, &[]).is_empty());
    }

    #[test]
    fn selection_dispatch_clears_directly_and_does_not_edit_document_or_composer() {
        let mut app = app();
        let ctx = egui::Context::default();
        let page = app.session.page().unwrap();
        let first = card(app.session.doc_mut().unwrap(), page, "First", "A", 0.0, 0.0);
        let second = card(app.session.doc_mut().unwrap(), page, "Second", "B", 250.0, 0.0);
        app.compose.input = "Unsent comment".into();
        let before = app.session.doc().unwrap().clone();
        assert!(run(&mut app, &ctx, "map.selectAll"));
        assert_eq!(app.session.selection(), vec![first, second]);
        run(&mut app, &ctx, "map.nextCard");
        assert_eq!(app.session.selection(), vec![first]);
        run(&mut app, &ctx, "map.previousCard");
        assert_eq!(app.session.selection(), vec![second]);
        run(&mut app, &ctx, "map.selectNone");
        assert!(app.session.selection().is_empty());
        assert_eq!(app.compose.input, "Unsent comment");
        assert_eq!(app.session.doc().unwrap(), &before);
        assert!(!run(&mut app, &ctx, "map.unowned"));
    }

    #[test]
    fn clicks_reach_boxes_inside_boxes_one_level_at_a_time() {
        let mut app = app();
        let page = app.session.page().unwrap();
        let doc = app.session.doc_mut().unwrap();
        let outer = doc.create(page, NodeKind::Frame, "Outer", 0.0, 0.0, 400.0, 300.0).unwrap();
        doc.create(outer, NodeKind::Text(TextData::new("Outer title")), "title", 10.0, 10.0, 200.0, 20.0).unwrap();
        let left = card(doc, outer, "Left", "Left body", 20.0, 60.0);
        let right = card(doc, outer, "Right", "Right body", 210.0, 60.0);
        let doc = app.session.doc().unwrap();
        let in_left = figcraft_geom::Point::new(100.0, 140.0);
        let chain = boxes_at(doc, page, in_left);
        assert_eq!(chain[..2], [outer, left]);
        let left_body = chain[2];
        assert_eq!(doc.node(left_body).unwrap().name, "body");
        // A first click selects the card; a double-click goes one box deeper each time.
        assert_eq!(click_target(doc, &chain, &[], false), Some(outer));
        assert_eq!(inner(&chain, outer), Some(left));
        assert_eq!(inner(&chain, left), Some(left_body));
        assert_eq!(inner(&chain, left_body), None);
        // Once inside, a click selects the neighbouring box at the same depth.
        let in_right = boxes_at(doc, page, figcraft_geom::Point::new(300.0, 140.0));
        assert_eq!(click_target(doc, &in_right, &[left], false), Some(right));
        // Cmd/Ctrl+click goes straight to the innermost box.
        assert_eq!(click_target(doc, &chain, &[], true), Some(left_body));
        // Clicking another card leaves the nested level.
        let other = card(app.session.doc_mut().unwrap(), page, "Other", "Elsewhere", 600.0, 0.0);
        let doc = app.session.doc().unwrap();
        let elsewhere = boxes_at(doc, page, figcraft_geom::Point::new(650.0, 50.0));
        assert_eq!(click_target(doc, &elsewhere, &[left], false), Some(other));
    }

    #[test]
    fn detail_and_parent_dispatch_use_page_metadata_and_ignore_invalid_links() {
        let mut app = app();
        let ctx = egui::Context::default();
        let page = app.session.page().unwrap();
        let card = card(app.session.doc_mut().unwrap(), page, "Source", "Read me", 0.0, 0.0);
        let child = subpage(app.session.doc_mut().unwrap(), page, "Detail");
        app.session.doc_mut().unwrap().node_mut(card).unwrap().plugin_data.insert(crate::new_page::DETAIL_PAGE_KEY.into(), child.0.to_string());
        app.session.set_selection(vec![card]);
        let before = app.session.doc().unwrap().clone();
        run(&mut app, &ctx, "map.openDetail");
        assert_eq!(app.session.page().unwrap(), child);
        run(&mut app, &ctx, "map.parentPage");
        assert_eq!(app.session.page().unwrap(), page);
        run(&mut app, &ctx, "map.previousPage");
        assert_eq!(app.session.page().unwrap(), page);
        run(&mut app, &ctx, "map.nextPage");
        assert_eq!(app.session.page().unwrap(), child);
        assert_eq!(app.session.doc().unwrap(), &before);
        let doc = app.session.doc_mut().unwrap();
        doc.node_mut(card).unwrap().plugin_data.insert(crate::new_page::DETAIL_PAGE_KEY.into(), card.0.to_string());
        assert_eq!(detail(doc, page, &[card]), None);
        doc.node_mut(child).unwrap().plugin_data.insert(page_tree::SOURCE_KEY.into(), card.0.to_string());
        assert_eq!(detail(doc, page, &[card]), Some(child));
    }

    #[test]
    fn opening_search_result_switches_page_before_selecting_and_rejects_stale_results() {
        let mut app = app();
        let root = app.session.page().unwrap();
        let child = subpage(app.session.doc_mut().unwrap(), root, "Elsewhere");
        let card = card(app.session.doc_mut().unwrap(), child, "Needle", "Body", 0.0, 0.0);
        let result = search(app.session.doc().unwrap(), "needle").remove(0);
        let before = app.session.doc().unwrap().clone();
        assert!(open_result(&mut app, &result));
        assert_eq!(app.session.page().unwrap(), child);
        assert_eq!(app.session.selection(), vec![card]);
        assert_eq!(app.session.doc().unwrap(), &before);
        app.session.doc_mut().unwrap().remove(card).unwrap();
        assert!(!open_result(&mut app, &result));
    }

    #[test]
    fn open_search_prefills_the_query() {
        let mut app = app();
        let ctx = egui::Context::default();
        run(&mut app, &ctx, "map.search");
        let mut search = state(&ctx);
        search.query = "older".into();
        ctx.data_mut(|data| data.insert_temp(state_id(), search));
        close_search(&ctx);
        assert!(open_search(&mut app, &ctx, "  retry button "));
        assert!(search_open(&ctx));
        assert_eq!(state(&ctx).query, "retry button");
        assert!(state(&ctx).focus);
    }

    #[test]
    fn search_cancel_preserves_selection_and_query_but_document_switch_closes_it() {
        let mut app = app();
        let ctx = egui::Context::default();
        let page = app.session.page().unwrap();
        let card = card(app.session.doc_mut().unwrap(), page, "Card", "Body", 0.0, 0.0);
        app.session.set_selection(vec![card]);
        assert!(!close_search(&ctx));
        run(&mut app, &ctx, "map.search");
        assert!(search_open(&ctx));
        let mut search = state(&ctx);
        search.query = "retained query".into();
        ctx.data_mut(|data| data.insert_temp(state_id(), search));
        assert!(close_search(&ctx));
        assert!(!close_search(&ctx));
        assert_eq!(app.session.selection(), vec![card]);
        run(&mut app, &ctx, "map.search");
        assert_eq!(state(&ctx).query, "retained query");
        app.session.ds_mut().unwrap().uid += 1;
        show(&mut app, &ctx);
        assert!(!search_open(&ctx));
        run(&mut app, &ctx, "map.search");
        assert!(state(&ctx).query.is_empty());
    }

    #[test]
    fn page_tree_dispatch_toggles_existing_state() {
        let mut app = app();
        let ctx = egui::Context::default();
        run(&mut app, &ctx, "map.pageTree");
        assert!(app.page_tree.open && app.page_tree.just_opened);
        run(&mut app, &ctx, "map.pageTree");
        assert!(!app.page_tree.open && !app.page_tree.just_opened);
    }

    #[test]
    fn search_keyboard_chooses_and_opens_a_result_while_query_has_focus() {
        let mut app = app();
        let ctx = egui::Context::default();
        let page = app.session.page().unwrap();
        let first = card(app.session.doc_mut().unwrap(), page, "Match one", "First", 0.0, 0.0);
        let child = subpage(app.session.doc_mut().unwrap(), page, "Detail");
        let second = card(app.session.doc_mut().unwrap(), child, "Match two", "Second", 0.0, 0.0);
        run(&mut app, &ctx, "map.search");
        let mut search = state(&ctx);
        search.query = "match".into();
        ctx.data_mut(|data| data.insert_temp(state_id(), search));

        let frame = |app: &mut FigApp, key: Option<Key>| {
            let events = key
                .map(|key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE })
                .into_iter()
                .collect();
            let mut output = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| show(app, ui.ctx()));
            output.textures_delta.clear();
        };
        frame(&mut app, None);
        assert_eq!(state(&ctx).selected, Some(first));
        assert!(ctx.memory(|memory| memory.has_focus(input_id())));
        frame(&mut app, Some(Key::ArrowDown));
        assert_eq!(state(&ctx).selected, Some(second));
        frame(&mut app, Some(Key::Enter));
        assert_eq!(app.session.page().unwrap(), child);
        assert_eq!(app.session.selection(), vec![second]);
        assert!(!search_open(&ctx));
    }

    #[test]
    fn copy_dispatch_outputs_readable_text_without_mutating_the_document() {
        let mut app = app();
        let ctx = egui::Context::default();
        let page = app.session.page().unwrap();
        let card = card(app.session.doc_mut().unwrap(), page, "Card", "Copied content", 0.0, 0.0);
        app.session.set_selection(vec![card]);
        let before = app.session.doc().unwrap().clone();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            assert!(run(&mut app, ui.ctx(), "map.copy"));
        });
        output.textures_delta.clear();
        assert!(output.platform_output.commands.iter().any(|command| {
            matches!(command, egui::OutputCommand::CopyText(text) if text.contains("Copied content") && text.contains("Map:") && text.contains("Page:"))
        }));
        assert_eq!(app.session.doc().unwrap(), &before);
    }
}
