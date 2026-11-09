//! The colour themes: Afterburn (the default), Afterburn Night, Classic and Classic Dark.
//!
//! A theme is a family (`Prefs::color_theme`: "afterburn" or "classic") lit light or dark by
//! `Prefs::theme` ("light", "dark", or "system": Afterburn by day, Afterburn Night by night). It
//! fills one set of tokens: the map, Sync and the workspace screens read [`get`]; the design
//! editor (the IDE) keeps its layout and look and takes the theme's accents through
//! [`Theme::tokens`] (design: `Mindmap page · Afterburn`, `Colour map` and `Palette Colour
//! theme`).
//!
//! The current palette is set once a frame, before anything draws ([`set`]); it lives with the
//! UI thread, so tests on other threads keep their own.

use std::cell::Cell;

use egui::Color32;

use crate::theme::Tokens;

/// A colour theme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    Afterburn,
    AfterburnNight,
    Classic,
    ClassicDark,
}

impl Theme {
    /// Every theme, in the picker's order.
    pub const ALL: [Theme; 4] = [Theme::Afterburn, Theme::AfterburnNight, Theme::Classic, Theme::ClassicDark];

    /// The theme `family` ("afterburn" or "classic"; anything else is Afterburn) lit `dark`.
    pub fn of(family: &str, dark: bool) -> Theme {
        match (family.trim().eq_ignore_ascii_case("classic"), dark) {
            (false, false) => Theme::Afterburn,
            (false, true) => Theme::AfterburnNight,
            (true, false) => Theme::Classic,
            (true, true) => Theme::ClassicDark,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Theme::Afterburn => "Afterburn",
            Theme::AfterburnNight => "Afterburn Night",
            Theme::Classic => "Classic",
            Theme::ClassicDark => "Classic Dark",
        }
    }

    /// What the picker says under the name.
    pub fn note(self) -> &'static str {
        match self {
            Theme::Afterburn => "Light · the default",
            Theme::AfterburnNight | Theme::ClassicDark => "Dark",
            Theme::Classic => "Light · Mindmap until now",
        }
    }

    /// The family `Prefs::color_theme` stores.
    pub fn family(self) -> &'static str {
        match self {
            Theme::Afterburn | Theme::AfterburnNight => "afterburn",
            Theme::Classic | Theme::ClassicDark => "classic",
        }
    }

    pub fn dark(self) -> bool {
        matches!(self, Theme::AfterburnNight | Theme::ClassicDark)
    }

    /// The map's palette.
    pub fn colors(self) -> MapColors {
        match self {
            Theme::Afterburn => AFTERBURN,
            Theme::AfterburnNight => AFTERBURN_NIGHT,
            Theme::Classic => CLASSIC,
            Theme::ClassicDark => CLASSIC_DARK,
        }
    }

    /// The design editor's tokens: its own light or dark set, with this theme's accent. The IDE
    /// keeps its layout and look; only these tokens change.
    pub fn tokens(self) -> Tokens {
        let rgb = |r, g, b| Color32::from_rgb(r, g, b);
        match self {
            Theme::Classic => Tokens::light(),
            Theme::ClassicDark => Tokens::dark(),
            Theme::Afterburn => {
                let mut t = Tokens::light();
                t.accent = rgb(0xFF, 0x5A, 0x1F);
                t.accent_hover = rgb(0xE6, 0x4A, 0x12);
                t.accent_text = rgb(0xC2, 0x41, 0x0C);
                t.selected_row = rgb(0xFF, 0xED, 0xE3);
                t.focus = t.accent;
                t
            }
            Theme::AfterburnNight => {
                let mut t = Tokens::dark();
                t.accent = rgb(0xFF, 0x6A, 0x33);
                t.accent_hover = rgb(0xFF, 0x84, 0x52);
                t.accent_text = rgb(0xFF, 0x8A, 0x5C);
                t.selected_row = rgb(0x4A, 0x2C, 0x24);
                t.focus = t.accent;
                t
            }
        }
    }
}

/// One theme's map palette.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapColors {
    /// The map's paper, behind the cards.
    pub canvas: Color32,
    /// Bars, sidebars, cards, the composer.
    pub panel: Color32,
    /// Hover and soft fills.
    pub soft: Color32,
    pub border: Color32,
    pub ink: Color32,
    pub muted: Color32,
    pub faint: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    /// Text and marks on the accent.
    pub on_accent: Color32,
    /// The pale accent behind a comment saved on several cards and the `N SELECTED` chip.
    pub accent_tint: Color32,
    pub accent_tint_hover: Color32,
    /// The map's second ink: agents, links, live marks (readable as text).
    pub map_ink: Color32,
    /// The same hue as a fill: badges, toggles, the + on a waiting card.
    pub teal: Color32,
    /// The open page's row, and other selected rows.
    pub selected: Color32,
    pub danger: Color32,
    /// The draft open in the field, and that field.
    pub edit: Color32,
    pub edit_fill: Color32,
    /// A working agent, as a fill (the ~ badge) and as text.
    pub working: Color32,
    pub working_ink: Color32,
    /// Page tabs: the strip, an inactive tab, a hovered one.
    pub tab_row: Color32,
    pub tab_inactive: Color32,
    pub tab_hover: Color32,
    /// A button that can't act now.
    pub disabled_fill: Color32,
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

/// Classic: Mindmap's palette until now (compose.rs, sync.rs, agents.rs, map_tabs.rs).
pub const CLASSIC: MapColors = MapColors {
    canvas: rgb(0xF4, 0xF4, 0xF2),
    panel: Color32::WHITE,
    soft: rgb(0xF8, 0xF6, 0xF3),
    border: rgb(0xE3, 0xE2, 0xDF),
    ink: rgb(0x1A, 0x1A, 0x1A),
    muted: rgb(0x6E, 0x6C, 0x68),
    faint: rgb(0x9A, 0x97, 0x91),
    accent: rgb(0xB5, 0x45, 0x2F),
    accent_hover: rgb(0xC2, 0x55, 0x3E),
    on_accent: Color32::WHITE,
    accent_tint: rgb(0xFA, 0xE9, 0xE4),
    accent_tint_hover: rgb(0xF5, 0xDC, 0xD5),
    map_ink: rgb(0x2F, 0x6F, 0x6A),
    teal: rgb(0x2F, 0x6F, 0x6A),
    selected: rgb(0xEF, 0xEE, 0xEA),
    danger: rgb(0x9E, 0x3B, 0x2C),
    edit: rgb(0x2F, 0x6F, 0xDB),
    edit_fill: rgb(0xEA, 0xF0, 0xFB),
    working: rgb(0xA8, 0x6B, 0x12),
    working_ink: rgb(0xA8, 0x6B, 0x12),
    tab_row: rgb(0xF7, 0xF6, 0xF3),
    tab_inactive: rgb(0xF1, 0xF0, 0xED),
    tab_hover: rgb(0xEA, 0xE9, 0xE5),
    disabled_fill: rgb(0xE3, 0xE2, 0xDF),
};

/// Classic Dark: the dark workspace skin's surfaces with Classic's lightened inks.
pub const CLASSIC_DARK: MapColors = MapColors {
    canvas: rgb(0x1E, 0x1E, 0x1E),
    panel: rgb(0x2C, 0x2C, 0x2C),
    soft: rgb(0x38, 0x38, 0x38),
    border: rgb(0x44, 0x44, 0x44),
    ink: rgb(0xF5, 0xF5, 0xF5),
    muted: rgb(0xB3, 0xB3, 0xB3),
    faint: rgb(0x80, 0x80, 0x80),
    accent: rgb(0xD9, 0x7A, 0x5F),
    accent_hover: rgb(0xE8, 0x8E, 0x75),
    on_accent: rgb(0x1A, 0x1A, 0x1A),
    accent_tint: rgb(0x4A, 0x30, 0x2A),
    accent_tint_hover: rgb(0x5A, 0x3A, 0x32),
    map_ink: rgb(0x7F, 0xBD, 0xB5),
    teal: rgb(0x7F, 0xBD, 0xB5),
    selected: rgb(0x3A, 0x30, 0x2C),
    danger: rgb(0xE8, 0x9A, 0x8A),
    edit: rgb(0x8F, 0xB3, 0xF0),
    edit_fill: rgb(0x26, 0x32, 0x4A),
    working: rgb(0xE0, 0xA8, 0x5A),
    working_ink: rgb(0xE0, 0xA8, 0x5A),
    tab_row: rgb(0x23, 0x23, 0x23),
    tab_inactive: rgb(0x26, 0x26, 0x26),
    tab_hover: rgb(0x33, 0x33, 0x33),
    disabled_fill: rgb(0x44, 0x44, 0x44),
};

/// Afterburn: cream paper, ember accent, lagoon teal (design: `Colour map`).
pub const AFTERBURN: MapColors = MapColors {
    canvas: rgb(0xFF, 0xF4, 0xE6),
    panel: rgb(0xFF, 0xFA, 0xF2),
    soft: rgb(0xFB, 0xEE, 0xDC),
    border: rgb(0xEA, 0xD3, 0xB0),
    ink: rgb(0x0B, 0x0B, 0x0D),
    muted: rgb(0x6B, 0x5A, 0x48),
    faint: rgb(0xA8, 0x95, 0x7C),
    accent: rgb(0xFF, 0x5A, 0x1F),
    accent_hover: rgb(0xE6, 0x4A, 0x12),
    on_accent: Color32::WHITE,
    accent_tint: rgb(0xFF, 0xE3, 0xD3),
    accent_tint_hover: rgb(0xFF, 0xD3, 0xBD),
    map_ink: rgb(0x0E, 0x8F, 0x8C),
    teal: rgb(0x12, 0xC4, 0xC0),
    selected: rgb(0xDD, 0xF6, 0xF4),
    danger: rgb(0xFF, 0x2E, 0x63),
    edit: rgb(0x1B, 0x2B, 0x6B),
    edit_fill: rgb(0xE3, 0xE7, 0xF5),
    working: rgb(0xFF, 0xD2, 0x3F),
    working_ink: rgb(0x9A, 0x6A, 0x00),
    tab_row: rgb(0xFB, 0xEE, 0xDC),
    tab_inactive: rgb(0xF2, 0xDF, 0xC0),
    tab_hover: rgb(0xEA, 0xD3, 0xB0),
    disabled_fill: rgb(0xEA, 0xD3, 0xB0),
};

/// Afterburn Night (design: `Palette Colour theme`, previewing it).
pub const AFTERBURN_NIGHT: MapColors = MapColors {
    canvas: rgb(0x15, 0x12, 0x1A),
    panel: rgb(0x1E, 0x1A, 0x24),
    soft: rgb(0x2A, 0x24, 0x30),
    border: rgb(0x2E, 0x28, 0x36),
    ink: rgb(0xF5, 0xED, 0xE2),
    muted: rgb(0xA8, 0x9B, 0xB0),
    faint: rgb(0x7D, 0x70, 0x88),
    accent: rgb(0xFF, 0x6A, 0x33),
    accent_hover: rgb(0xFF, 0x84, 0x52),
    on_accent: Color32::WHITE,
    accent_tint: rgb(0x3A, 0x24, 0x20),
    accent_tint_hover: rgb(0x4A, 0x2C, 0x24),
    map_ink: rgb(0x3E, 0xD6, 0xD2),
    teal: rgb(0x3E, 0xD6, 0xD2),
    selected: rgb(0x1D, 0x34, 0x36),
    danger: rgb(0xFF, 0x5C, 0x82),
    edit: rgb(0x8F, 0xA4, 0xF0),
    edit_fill: rgb(0x23, 0x28, 0x42),
    working: rgb(0xFF, 0xD2, 0x3F),
    working_ink: rgb(0xFF, 0xD2, 0x3F),
    tab_row: rgb(0x18, 0x15, 0x1E),
    tab_inactive: rgb(0x1A, 0x17, 0x20),
    tab_hover: rgb(0x2A, 0x24, 0x30),
    disabled_fill: rgb(0x2E, 0x28, 0x36),
};

thread_local! {
    static CURRENT: Cell<Theme> = const { Cell::new(Theme::Afterburn) };
}

/// The theme the UI draws with now.
pub fn theme() -> Theme {
    CURRENT.with(Cell::get)
}

/// The palette the UI draws with now.
pub fn get() -> MapColors {
    theme().colors()
}

/// Draw with `theme` from now on (once a frame, before anything draws; the live preview sets the
/// hovered theme the same way).
pub fn set(theme: Theme) {
    CURRENT.with(|c| c.set(theme));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_family_lit_light_or_dark_is_one_theme_and_back() {
        for theme in Theme::ALL {
            assert_eq!(Theme::of(theme.family(), theme.dark()), theme);
        }
        assert_eq!(Theme::of("", false), Theme::Afterburn, "the default");
        assert_eq!(Theme::of("afterburn", true), Theme::AfterburnNight);
    }

    #[test]
    fn the_ide_keeps_its_look_and_takes_the_themes_accent() {
        let light = Tokens::light();
        let afterburn = Theme::Afterburn.tokens();
        assert_eq!(afterburn.accent, Color32::from_rgb(0xFF, 0x5A, 0x1F));
        assert_eq!((afterburn.panel, afterburn.text, afterburn.canvas), (light.panel, light.text, light.canvas));
        assert_eq!(Theme::Classic.tokens().accent, light.accent);
        assert!(Theme::AfterburnNight.tokens().dark);
    }
}
