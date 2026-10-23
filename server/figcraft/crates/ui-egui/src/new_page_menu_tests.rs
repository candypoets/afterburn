//! The map's right-click menu: "New page from card" opens the New page dialog.
//!
//! Driven through the same frame order the app uses (`new_page::show` before the menu), on a raw
//! egui context, so it needs no `MINDMAP_CLIENT` and runs in the normal test suite.

use egui::{Modifiers, PointerButton, Pos2, RawInput, Rect, vec2};
use figcraft_engine::Session;
use serde_json::json;

use crate::{FigApp, NoServices};

fn texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<(String, egui::Rect)> {
    fn walk(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
        match shape {
            egui::Shape::Text(text) => out.push((text.galley.text().to_string(), egui::Rect::from_min_size(text.pos, text.galley.size()))),
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    walk(shape, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for clipped in shapes {
        walk(&clipped.shape, &mut out);
    }
    out
}

#[test]
fn right_click_new_page_from_card_opens_the_dialog() {
    let mut session = Session::new();
    let frame = session.execute("frame.create", &json!({"x": 100, "y": 100, "width": 300, "height": 200, "name": "Card One"})).unwrap();
    let card = figcraft_engine::params::id_of(&frame["id"]).unwrap();
    session.execute("select.set", &json!({"ids": [card]})).unwrap();
    let mut app = FigApp::new(session, Box::new(NoServices));

    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx, false, 0);
    let mut time = 0.0f64;
    let mut run = |app: &mut FigApp, events: Vec<egui::Event>| {
        time += 1.0 / 60.0;
        let input =
            RawInput { events, screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0))), time: Some(time), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| {
            let ctx = ui.ctx().clone();
            crate::new_page::show(app, &ctx);
            crate::compose::context_menu(app, &ctx, Pos2::new(200.0, 200.0));
        });
        out.textures_delta.clear();
        out
    };
    // Let the fonts and the first layout settle before reading the menu's text.
    run(&mut app, vec![]);
    run(&mut app, vec![]);

    let anchor = Pos2::new(200.0, 200.0);
    let out = run(&mut app, vec![egui::Event::PointerMoved(anchor)]);
    let item = texts(&out.shapes).into_iter().find(|(text, _)| text == "New page from card").expect("the card's menu offers a new page").1;
    let item = item.center();
    let modifiers = Modifiers::default();
    for pressed in [true, false] {
        run(
            &mut app,
            vec![egui::Event::PointerMoved(item), egui::Event::PointerButton { pos: item, button: PointerButton::Primary, pressed, modifiers }],
        );
    }

    run(&mut app, vec![]);
    assert!(app.new_page.open, "the dialog opens from the card's menu");
}
