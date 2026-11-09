//! The Burn mark: a CD-R written from the hub outward (design: `Burn loading`).
//!
//! A blank disc is pale and iridescent. While a Burn runs, the written band grows from the hub to
//! the rim in the theme's ember-to-flare colours, one turn of the spiral at a time, with faint
//! track rings inside it and a bright laser dot riding its outer edge. Sync reports no percent,
//! so each pass loops: when the band reaches the rim it fades and the laser starts at the hub
//! again. Done fills the disc to the rim and stamps a ✓.
//!
//! The same disc is drawn at 16 px (the header toggle, the page tree), 24 px (the Burn button)
//! and 40 px (a toast). It is drawn in code from rings and fans; there is no image.

use std::f32::consts::{FRAC_PI_2, TAU};

use egui::{Color32, Mesh, Painter, Pos2, Rect, Shape, Stroke, pos2, vec2};

use crate::map_colors::MapColors;

/// What the disc shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DiscState {
    /// Nothing written yet.
    Blank,
    /// A Burn in flight, `t` seconds into it (any clock: the pass loops).
    Burning { t: f64 },
    /// Written to the rim.
    Done,
}

/// One pass, in seconds: the band writes from the hub to the rim in [`WRITE`], then fades.
pub const PASS: f64 = 1.45;
/// How long the band takes from the hub to the rim.
pub const WRITE: f64 = 1.2;
/// The hub's radius, as a share of the disc's: the band starts here.
pub const HUB: f32 = 0.3;
/// The centre hole's radius, as a share of the disc's.
const HOLE: f32 = 0.11;
/// Turns of the spiral in one pass: the laser goes round this many times on its way out.
pub const TURNS: f32 = 5.0;
/// The track rings' radii (shares of the disc's), drawn once the band has passed them.
const TRACKS: [f32; 5] = [0.4, 0.525, 0.65, 0.775, 0.9];
/// Where the laser starts each turn: the top.
const START: f32 = -FRAC_PI_2;

/// The blank disc's iridescence, round the disc from the top.
const IRIDESCENT: [(f32, [u8; 3]); 9] = [
    (0.0, [0xE9, 0xE9, 0xF0]),
    (0.125, [0xBF, 0xE7, 0xFF]),
    (0.25, [0xFF, 0xD3, 0xEA]),
    (0.375, [0xFF, 0xF1, 0xB5]),
    (0.5, [0xC6, 0xF4, 0xE2]),
    (0.625, [0xE9, 0xE9, 0xF0]),
    (0.75, [0xD5, 0xCE, 0xFF]),
    (0.875, [0xFF, 0xDD, 0xC2]),
    (1.0, [0xE9, 0xE9, 0xF0]),
];

/// The written band at one moment, in shares of the disc's radius.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Written {
    /// Written all the way round, from the hub out to here.
    pub full: f32,
    /// The turn being written: from `full` out to `full + turn`, over `sweep` radians from the top.
    pub turn: f32,
    pub sweep: f32,
    /// Where the laser is on its way out: grows from [`HUB`] to 1 over a pass.
    pub radius: f32,
    /// The band's opacity: it fades once the rim is reached.
    pub alpha: f32,
    /// The laser's angle while it writes.
    pub laser: Option<f32>,
}

/// The written band for `state`.
pub fn written(state: DiscState) -> Written {
    let width = (1.0 - HUB) / TURNS;
    match state {
        DiscState::Blank => Written { full: HUB, turn: 0.0, sweep: 0.0, radius: HUB, alpha: 0.0, laser: None },
        DiscState::Done => Written { full: 1.0, turn: 0.0, sweep: 0.0, radius: 1.0, alpha: 1.0, laser: None },
        DiscState::Burning { t } => {
            let t = if t.is_finite() { t.rem_euclid(PASS) } else { 0.0 };
            if t >= WRITE {
                let fade = ((t - WRITE) / (PASS - WRITE)) as f32;
                return Written { full: 1.0, turn: 0.0, sweep: 0.0, radius: 1.0, alpha: (1.0 - fade).clamp(0.0, 1.0), laser: None };
            }
            let share = (t / WRITE) as f32;
            let turns = (share * TURNS).clamp(0.0, TURNS);
            let done = turns.floor();
            let sweep = (turns - done) * TAU;
            Written { full: HUB + done * width, turn: width, sweep, radius: HUB + share * (1.0 - HUB), alpha: 1.0, laser: Some(START + sweep) }
        }
    }
}

/// Paint the disc in `rect` (centred, as large as fits).
pub fn paint(painter: &Painter, rect: Rect, state: DiscState, colors: &MapColors) {
    let r = rect.width().min(rect.height()) / 2.0;
    if !r.is_finite() || r < 1.0 {
        return;
    }
    let c = rect.center();
    let band = written(state);

    // The blank disc: iridescent, pale.
    blank(painter, c, r);

    // The written band, ember at the hub to flare at the rim.
    let alpha = band.alpha * 0.92;
    if alpha > 0.0 {
        let ember = colors.accent;
        let flare = colors.danger;
        let ink = |share: f32| {
            let k = ((share - HUB) / (1.0 - HUB)).clamp(0.0, 1.0);
            lerp(ember, flare, k).gamma_multiply(alpha)
        };
        if band.full > HUB {
            radial_band(painter, c, r * HUB, r * band.full, 0.0, TAU, |radius| ink(radius / r));
        }
        if band.sweep > 0.0 && band.turn > 0.0 {
            let outer = (band.full + band.turn).min(1.0);
            radial_band(painter, c, r * band.full, r * outer, START, START + band.sweep, |radius| ink(radius / r));
        }
        if r >= 10.0 {
            let width = (r / 160.0).max(0.5);
            for track in TRACKS.iter().filter(|track| **track < band.full) {
                painter.circle_stroke(c, r * track, Stroke::new(width, Color32::WHITE.gamma_multiply(0.35 * band.alpha)));
            }
        }
    }

    // The rim.
    painter.circle_stroke(c, r - 0.4, Stroke::new((r / 107.0).max(0.75), colors.ink.gamma_multiply(0.22)));

    // The hub and its hole.
    painter.circle_filled(c, r * HUB, Color32::WHITE.gamma_multiply(0.6));
    painter.circle_stroke(c, r * HUB, Stroke::new(0.5, colors.ink.gamma_multiply(0.18)));
    painter.circle_filled(c, (r * HOLE).max(0.8), colors.canvas);
    painter.circle_stroke(c, (r * HOLE).max(0.8), Stroke::new(0.5, colors.ink.gamma_multiply(0.3)));

    // The laser on the band's outer edge, with an ember glow.
    if let Some(angle) = band.laser {
        let at = c + vec2(angle.cos(), angle.sin()) * r * (band.full + band.turn).min(1.0);
        let dot = (r * 0.056).max(1.1);
        painter.circle_filled(at, dot * 2.2, colors.accent.gamma_multiply(0.35));
        painter.circle_filled(at, dot, Color32::WHITE);
    }

    // Done: a ✓ badge at the lower right (from the toast's 40 px up; too small to read below).
    if state == DiscState::Done && r >= 16.0 {
        let at = c + vec2(r * 0.74, r * 0.74);
        let radius = r * 0.21;
        painter.circle_filled(at, radius, colors.ink);
        let s = radius * 0.5;
        let check = vec![pos2(at.x - s, at.y), pos2(at.x - s * 0.25, at.y + s * 0.7), pos2(at.x + s, at.y - s * 0.65)];
        painter.add(Shape::line(check, Stroke::new((radius * 0.2).max(1.0), colors.canvas)));
    }

    if matches!(state, DiscState::Burning { .. }) {
        painter.ctx().request_repaint_after(std::time::Duration::from_millis(33));
    }
}

/// The blank disc: a fan whose colours turn round the iridescence.
fn blank(painter: &Painter, c: Pos2, r: f32) {
    const STEPS: u32 = 48;
    let mut mesh = Mesh::default();
    let hub = Color32::from_rgb(0xF2, 0xF0, 0xF4);
    for i in 0..=STEPS {
        let share = i as f32 / STEPS as f32;
        let a = START + share * TAU;
        let d = vec2(a.cos(), a.sin());
        mesh.colored_vertex(c + d * r * HUB * 0.9, hub);
        mesh.colored_vertex(c + d * r, iridescent(share));
        if i > 0 {
            let k = 2 * i;
            mesh.add_triangle(k - 2, k - 1, k);
            mesh.add_triangle(k - 1, k + 1, k);
        }
    }
    painter.add(Shape::mesh(mesh));
}

/// The iridescence `share` of the way round.
fn iridescent(share: f32) -> Color32 {
    let share = share.clamp(0.0, 1.0);
    let mut prev = IRIDESCENT[0];
    for stop in IRIDESCENT {
        if share <= stop.0 {
            let span = (stop.0 - prev.0).max(f32::EPSILON);
            let k = ((share - prev.0) / span).clamp(0.0, 1.0);
            let [r0, g0, b0] = prev.1;
            let [r1, g1, b1] = stop.1;
            return lerp(Color32::from_rgb(r0, g0, b0), Color32::from_rgb(r1, g1, b1), k);
        }
        prev = stop;
    }
    let [r, g, b] = prev.1;
    Color32::from_rgb(r, g, b)
}

/// A ring from `inner` to `outer` around `c`, from angle `a0` to `a1`, coloured by radius.
fn radial_band(painter: &Painter, c: Pos2, inner: f32, outer: f32, a0: f32, a1: f32, color: impl Fn(f32) -> Color32) {
    if outer <= inner {
        return;
    }
    let steps = ((a1 - a0).abs() / TAU * 64.0).ceil().clamp(2.0, 64.0) as u32;
    let rings = 4u32;
    let mut mesh = Mesh::default();
    for j in 0..=rings {
        let radius = inner + (outer - inner) * j as f32 / rings as f32;
        let ink = color(radius);
        for i in 0..=steps {
            let a = a0 + (a1 - a0) * i as f32 / steps as f32;
            mesh.colored_vertex(c + vec2(a.cos(), a.sin()) * radius, ink);
        }
    }
    let row = steps + 1;
    for j in 0..rings {
        for i in 0..steps {
            let k = j * row + i;
            mesh.add_triangle(k, k + 1, k + row);
            mesh.add_triangle(k + 1, k + row + 1, k + row);
        }
    }
    painter.add(Shape::mesh(mesh));
}

fn lerp(a: Color32, b: Color32, k: f32) -> Color32 {
    let k = k.clamp(0.0, 1.0);
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * k).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgba_unmultiplied(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()), mix(a.a(), b.a()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(t: f64) -> Written {
        written(DiscState::Burning { t })
    }

    #[test]
    fn the_written_band_grows_from_the_hub_to_the_rim_then_loops() {
        let start = at(0.0);
        assert_eq!(start.radius, HUB, "a pass starts at the hub");
        assert_eq!(start.full, HUB);
        let mut last = start.radius;
        let mut t = 0.03;
        while t < WRITE {
            let w = at(t);
            assert!(w.radius >= last, "the band only grows while it writes ({t} s)");
            assert!(w.laser.is_some(), "the laser writes ({t} s)");
            assert!(w.full <= w.radius && w.radius <= w.full + w.turn + 1e-5, "the laser rides the band's edge");
            last = w.radius;
            t += 0.03;
        }
        assert!(at(WRITE - 0.01).radius > 0.95, "it nearly reaches the rim");
        let fading = at(WRITE + (PASS - WRITE) / 2.0);
        assert_eq!(fading.full, 1.0, "rim reached");
        assert!(fading.laser.is_none() && fading.alpha < 1.0 && fading.alpha > 0.0, "the band fades");
        let again = at(PASS + 0.01);
        assert!(again.radius < 0.35 && again.laser.is_some(), "the laser starts at the hub again");
        assert_eq!(at(3.0 * PASS + 0.4), at(0.4), "every pass is the same");
    }

    #[test]
    fn the_laser_goes_round_as_it_writes() {
        let turn = WRITE / TURNS as f64;
        let a = at(turn * 0.25).laser.unwrap();
        let b = at(turn * 0.75).laser.unwrap();
        assert!(b > a, "it turns clockwise from the top");
        assert!(at(turn * 1.5).full > at(turn * 0.5).full, "a finished turn is written all round");
    }

    #[test]
    fn done_fills_to_the_rim_and_blank_holds_nothing() {
        let done = written(DiscState::Done);
        assert_eq!((done.full, done.alpha, done.laser), (1.0, 1.0, None));
        let blank = written(DiscState::Blank);
        assert_eq!((blank.full, blank.alpha), (HUB, 0.0));
    }

    #[test]
    fn junk_time_does_not_break_the_disc() {
        for t in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0, 1e300] {
            let w = at(t);
            assert!(w.radius.is_finite() && w.full.is_finite() && w.alpha.is_finite(), "{t}");
            assert!((HUB..=1.0).contains(&w.radius), "{t}");
        }
    }

    #[test]
    fn every_size_paints_without_trouble() {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let painter = ui.painter().clone();
            for size in [0.0, 16.0, 24.0, 40.0, 160.0, f32::NAN] {
                let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(size, size));
                for state in [DiscState::Blank, DiscState::Burning { t: 0.5 }, DiscState::Burning { t: 1.3 }, DiscState::Done] {
                    for theme in crate::map_colors::Theme::ALL {
                        paint(&painter, rect, state, &theme.colors());
                    }
                }
            }
        });
        out.textures_delta.clear();
    }

    #[test]
    fn the_iridescence_wraps_round() {
        assert_eq!(iridescent(0.0), iridescent(1.0));
        assert_eq!(iridescent(0.25), Color32::from_rgb(0xFF, 0xD3, 0xEA));
    }
}
