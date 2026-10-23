//! Delete-page confirmation. A page's cards and comments are not recoverable from the map UI.

use egui::{Align2, Color32, CornerRadius, FontId, Id, Margin, RichText, Stroke, vec2};

use crate::FigApp;
use crate::theme::{regular, semibold};

const PANEL: Color32 = Color32::WHITE;
const BORDER: Color32 = Color32::from_rgb(0xE3, 0xE2, 0xDF);
const TEXT: Color32 = Color32::from_rgb(0x1A, 0x1A, 0x1A);
const MUTED: Color32 = Color32::from_rgb(0x6E, 0x6C, 0x68);
const DANGER: Color32 = Color32::from_rgb(0x9E, 0x3B, 0x2C);

/// Draw the confirmation when a page deletion is pending.
pub fn show(app: &mut FigApp, ctx: &egui::Context) {
    let Some(id) = app.page_delete else { return };
    let name = app.session.doc().ok().and_then(|doc| doc.node(id)).map(|node| node.name.clone()).unwrap_or_else(|| "this page".to_string());
    let mut cancel = false;
    let mut delete = false;
    egui::Area::new(Id::new("page_delete_modal")).anchor(Align2::CENTER_CENTER, vec2(0.0, -20.0)).order(egui::Order::Foreground).show(ctx, |ui| {
        egui::Frame::NONE
            .fill(PANEL)
            .stroke(Stroke::new(1.0, BORDER))
            .corner_radius(CornerRadius::same(10))
            .shadow(egui::epaint::Shadow { offset: [0, 10], blur: 30, spread: 0, color: Color32::from_black_alpha(46) })
            .inner_margin(Margin::symmetric(20, 16))
            .show(ui, |ui| {
                ui.set_width(380.0);
                ui.label(RichText::new("DELETE PAGE").font(FontId::monospace(9.5)).color(DANGER));
                ui.add_space(8.0);
                ui.label(RichText::new(format!("Delete “{name}”?")).font(semibold(17.0)).color(TEXT));
                ui.add_space(5.0);
                ui.label(
                    RichText::new("Its cards, comments and subpages are removed from the live map. This cannot be undone from the app.")
                        .font(regular(11.5))
                        .color(MUTED),
                );
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    if ui.add(egui::Button::new(RichText::new("Cancel").font(regular(11.5)).color(MUTED)).frame(false)).clicked() {
                        cancel = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let button = egui::Button::new(RichText::new("Delete page").font(semibold(11.5)).color(Color32::WHITE))
                            .fill(DANGER)
                            .corner_radius(CornerRadius::same(6))
                            .min_size(vec2(110.0, 32.0));
                        if ui.add(button).clicked() {
                            delete = true;
                        }
                    });
                });
            });
    });
    if delete {
        let page_name = name.clone();
        match crate::page_tree::delete_tree(app, id) {
            Ok(count) if count > 1 => app.toast(format!("Deleted {page_name} and {} subpage(s)", count - 1)),
            Ok(_) => app.toast(format!("Deleted {page_name}")),
            Err(error) => app.toast(format!("Could not delete {page_name}: {error}")),
        }
        app.page_delete = None;
    } else if cancel || ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        app.page_delete = None;
    }
}
