//! The comments pane at the right of the map: the open page's comments grouped by what they are
//! about, the page first (`Page comment · 3`, in the accent), then each card (`Overview · 1`).
//! Each comment shows its text and `author · age`. A card's comments take the canvas to the card;
//! `Open composer →` gives the composer the keyboard.
//!
//! The comments are the composer's saved comments (Enter in the composer keeps one on the
//! selected card or the page until a send carries it to an agent) and the file's open comment
//! threads. View ▸ Comments (Shift+C) shows or hides the pane with the comment marks on the
//! canvas; its header folds it to a rail.

use egui::{Align2, Color32, FontId, Id, Rect, Sense, Stroke, Ui, pos2, vec2};
use figcraft_doc::NodeId;
use figcraft_engine::cmd::comment as cc;
use serde_json::{Value, json};

use crate::FigApp;
use crate::compose::Draft;
use crate::theme::regular;

const INK: Color32 = Color32::from_rgb(0x1A, 0x1A, 0x1A);
const MUTED: Color32 = Color32::from_rgb(0x6E, 0x6C, 0x68);
const FAINT: Color32 = Color32::from_rgb(0x9A, 0x97, 0x91);
const ACCENT: Color32 = Color32::from_rgb(0xB5, 0x45, 0x2F);
const RULE: Color32 = Color32::from_rgb(0xE3, 0xE2, 0xDF);
/// The pane's inset.
const PAD: f32 = 14.0;

fn mono(size: f32) -> FontId {
    FontId::monospace(size)
}

/// When each saved comment was first seen, for its age (the composer keeps no clock).
#[derive(Default)]
pub struct State {
    seen: Vec<(Draft, u64)>,
}

/// Note the composer's saved comments: new ones are stamped now, sent or removed ones forgotten.
pub fn note(app: &mut FigApp) {
    let drafts = &app.compose.drafts;
    let seen = &mut app.map_comments.seen;
    seen.retain(|(draft, _)| drafts.contains(draft));
    let now = cc::now_secs();
    for draft in drafts {
        if !seen.iter().any(|(known, _)| known == draft) {
            seen.push((draft.clone(), now));
        }
    }
}

/// One comment in the pane.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub text: String,
    pub author: String,
    /// `2m`, `now`…; empty when unknown.
    pub age: String,
}

/// The comments about one thing: the page (`card: None`) or a card.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub card: Option<NodeId>,
    pub title: String,
    pub entries: Vec<Entry>,
}

impl Group {
    /// `Page comment · 3`, `Overview · 1`.
    pub fn heading(&self) -> String {
        format!("{} · {}", self.title, self.entries.len())
    }
}

/// The open page's comments by target: the page's first, then the cards' in the order their
/// first comment was written.
pub fn groups(app: &FigApp) -> Vec<Group> {
    let (Ok(doc), Ok(page)) = (app.session.doc(), app.session.page()) else { return Vec::new() };
    let mut items: Vec<(Option<NodeId>, Entry)> = Vec::new();
    for comment in doc.comments.iter().filter(|c| c.page == page && !c.resolved) {
        let card = comment.anchor.filter(|id| *id != page && doc.node(*id).is_some());
        let entry =
            Entry { text: comment.first.text.clone(), author: comment.first.author.clone(), age: crate::comments::ago(comment.first.created) };
        items.push((card, entry));
    }
    for draft in app.compose.drafts.iter().filter(|d| d.page_id == Some(page)) {
        let card = draft.node_id.filter(|id| *id != page);
        let at = app.map_comments.seen.iter().find(|(known, _)| known == draft).map_or(0, |(_, at)| *at);
        items.push((card, Entry { text: draft.text.clone(), author: "You".to_string(), age: crate::comments::ago(at) }));
    }
    let mut groups: Vec<Group> = Vec::new();
    for (card, entry) in items {
        if let Some(group) = groups.iter_mut().find(|g| g.card == card) {
            group.entries.push(entry);
            continue;
        }
        let title = match card {
            None => "Page comment".to_string(),
            Some(id) => doc.node(id).map_or_else(|| "Card".to_string(), |node| node.name.clone()),
        };
        groups.push(Group { card, title, entries: vec![entry] });
    }
    // The page's own comments lead.
    groups.sort_by_key(|g| g.card.is_some());
    groups
}

/// The pane's comments as `ui.inspect` reports them.
pub fn inspect(app: &FigApp) -> Value {
    json!({
        "open": crate::map_panes::comments_open(app),
        "collapsed": app.session.prefs.map_comments_collapsed,
        "groups": groups(app).iter().map(|g| json!({"card": g.card.map(|id| id.0), "heading": g.heading(), "count": g.entries.len()})).collect::<Vec<_>>(),
    })
}

enum Act {
    Card(NodeId),
    Composer,
    Fold(bool),
}

/// A text button drawn flat; returns its response.
fn link(ui: &mut Ui, id: Id, at: egui::Pos2, text: &str, font: FontId, color: Color32, hover: Color32) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, color);
    let rect = Rect::from_min_size(at, galley.size());
    let resp = ui.interact(rect, id, Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    ui.painter().galley(rect.min, galley, if resp.hovered() { hover } else { color });
    resp
}

/// The pane, filling `ui` (its left rule is the splitter's line).
pub fn show(app: &mut FigApp, ui: &mut Ui) {
    note(app);
    let rect = ui.max_rect();
    ui.painter().vline(rect.left(), rect.y_range(), Stroke::new(1.0, RULE));
    let mut act = None;
    if app.session.prefs.map_comments_collapsed {
        // The rail: unfold, then how many comments wait.
        let count: usize = groups(app).iter().map(|g| g.entries.len()).sum();
        let button = Rect::from_center_size(pos2(rect.center().x, rect.top() + 22.0), vec2(22.0, 22.0));
        let resp = ui.interact(button, Id::new("map_comments_unfold"), Sense::click());
        ui.painter().text(button.center(), Align2::CENTER_CENTER, "‹", regular(14.0), if resp.hovered() { ACCENT } else { MUTED });
        crate::a11y::button(&resp, "Show comments");
        if resp.on_hover_text("Show comments").clicked() {
            act = Some(Act::Fold(false));
        }
        if count > 0 {
            ui.painter().text(pos2(rect.center().x, button.bottom() + 14.0), Align2::CENTER_CENTER, count.to_string(), mono(9.0), ACCENT);
        }
    } else {
        let left = rect.left() + PAD;
        let width = (rect.width() - 2.0 * PAD).max(40.0);
        let top = rect.top() + 22.0;
        ui.painter().text(pos2(left, top), Align2::LEFT_CENTER, "COMMENTS", mono(9.0), FAINT);
        let fold = Rect::from_center_size(pos2(rect.right() - PAD - 6.0, top), vec2(22.0, 22.0));
        let resp = ui.interact(fold, Id::new("map_comments_fold"), Sense::click());
        ui.painter().text(fold.center(), Align2::CENTER_CENTER, "›", regular(14.0), if resp.hovered() { ACCENT } else { FAINT });
        crate::a11y::button(&resp, "Collapse comments");
        if resp.on_hover_text("Collapse comments").clicked() {
            act = Some(Act::Fold(true));
        }

        // The composer link stays at the bottom; the threads scroll above it.
        let footer = 44.0;
        let list = Rect::from_min_max(pos2(rect.left(), top + 18.0), pos2(rect.right(), rect.bottom() - footer));
        let groups = groups(app);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(list).layout(egui::Layout::top_down(egui::Align::Min)));
        egui::ScrollArea::vertical().id_salt("map_comments_list").auto_shrink([false, false]).show(&mut child, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            if groups.is_empty() {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.add_space(PAD);
                    ui.label(egui::RichText::new("No comments on this page.").font(regular(11.0)).color(FAINT));
                });
            }
            for (index, group) in groups.iter().enumerate() {
                ui.add_space(if index == 0 { 8.0 } else { 22.0 });
                let color = if group.card.is_none() { ACCENT } else { INK };
                let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::hover());
                let at = pos2(row.left() + PAD, row.top());
                let heading = group.heading();
                if let Some(card) = group.card {
                    let resp = link(ui, Id::new(("map_comments_card", card.0)), at, &heading, regular(12.0), color, ACCENT);
                    crate::a11y::button(&resp, &format!("Go to {}", group.title));
                    if resp.on_hover_text("Go to the card").clicked() {
                        act = Some(Act::Card(card));
                    }
                } else {
                    ui.painter().text(at, Align2::LEFT_TOP, heading, regular(12.0), color);
                }
                for entry in &group.entries {
                    ui.add_space(10.0);
                    let galley = ui.painter().layout(entry.text.clone(), regular(11.0), MUTED, width);
                    let (text_rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), galley.size().y), Sense::click());
                    ui.painter().galley(pos2(text_rect.left() + PAD, text_rect.top()), galley, MUTED);
                    if let Some(card) = group.card
                        && resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
                    {
                        act = Some(Act::Card(card));
                    }
                    ui.add_space(8.0);
                    let meta = if entry.age.is_empty() { entry.author.clone() } else { format!("{} · {}", entry.author, entry.age) };
                    let (meta_rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 12.0), Sense::hover());
                    ui.painter().text(pos2(meta_rect.left() + PAD, meta_rect.center().y), Align2::LEFT_CENTER, meta, mono(8.5), FAINT);
                }
            }
            ui.add_space(12.0);
        });
        let at = pos2(left, rect.bottom() - footer + 12.0);
        let resp = link(ui, Id::new("map_comments_composer"), at, "Open composer  →", regular(11.0), ACCENT, Color32::from_rgb(0x9E, 0x3B, 0x2C));
        crate::a11y::button(&resp, "Open composer");
        if resp.clicked() {
            act = Some(Act::Composer);
        }
    }
    let ctx = ui.ctx().clone();
    match act {
        Some(Act::Card(card)) => {
            app.session.set_selection(vec![card]);
            app.run("view.zoomSelection", json!({}));
        }
        Some(Act::Composer) => {
            crate::map_actions::run(app, &ctx, "map.focusComposer");
        }
        Some(Act::Fold(fold)) => {
            app.session.prefs.map_comments_collapsed = fold;
            app.services.save_prefs(&app.session.prefs);
        }
        None => {}
    }
    if act.is_some() {
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use figcraft_engine::Session;

    fn app() -> (FigApp, NodeId, NodeId) {
        let mut s = Session::new();
        let card = NodeId(
            s.execute("node.create", &json!({"type": "frame", "name": "Overview", "x": 0, "y": 0, "width": 200, "height": 100})).unwrap()["id"]
                .as_u64()
                .unwrap(),
        );
        let page = s.page().unwrap();
        (FigApp::new(s, Box::new(crate::NoServices)), page, card)
    }

    fn draft(text: &str, page: NodeId, card: Option<NodeId>) -> Draft {
        Draft { text: text.into(), node: None, page: None, node_id: card, page_id: Some(page) }
    }

    #[test]
    fn comments_group_by_target_page_first() {
        let (mut app, page, card) = app();
        app.compose.drafts.push(draft("This card is the entry point; keep it first.", page, Some(card)));
        app.compose.drafts.push(draft("The empty state should explain Init.", page, None));
        app.compose.drafts.push(draft("Name the agent too.", page, None));
        app.compose.drafts.push(draft("Elsewhere", NodeId(999_999), None));
        app.session.execute("comment.add", &json!({"x": 10, "y": 10, "text": "From the file"})).unwrap();
        note(&mut app);
        let groups = groups(&app);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].card, None);
        assert_eq!(groups[0].heading(), "Page comment · 3");
        assert_eq!(groups[0].entries[0].text, "From the file");
        assert_eq!(groups[1].card, Some(card));
        assert_eq!(groups[1].heading(), "Overview · 1");
        let mine = &groups[1].entries[0];
        assert_eq!(mine.author, "You");
        assert_eq!(mine.age, "now", "a saved comment is aged from when it was saved");
    }

    #[test]
    fn sent_comments_leave_the_pane() {
        let (mut app, page, _) = app();
        app.compose.drafts.push(draft("Gone soon", page, None));
        note(&mut app);
        assert_eq!(groups(&app).len(), 1);
        app.compose.drafts.clear();
        note(&mut app);
        assert!(groups(&app).is_empty());
        assert!(app.map_comments.seen.is_empty());
    }
}
