use egui::Color32;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

/// Colour family; light/dark and high contrast remain independent preferences.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Palette {
    #[default]
    Classic,
    Siege,
}

impl Palette {
    pub const ALL: [Self; 2] = [Self::Classic, Self::Siege];

    pub fn label(self) -> &'static str {
        match self {
            Self::Classic => "Classic",
            Self::Siege => "Siege21",
        }
    }
}

static DARK_MODE: AtomicBool = AtomicBool::new(true);
static HIGH_CONTRAST: AtomicBool = AtomicBool::new(false);
static PALETTE: AtomicU8 = AtomicU8::new(0);

pub fn set_palette(palette: Palette) {
    PALETTE.store(palette as u8, Ordering::Relaxed);
}

fn siege() -> bool {
    PALETTE.load(Ordering::Relaxed) == Palette::Siege as u8
}

// Siege21's published identity, sampled from https://siege21.com/assets/css/styles.css.
// Keep the exact brand orange on primary buttons; brighter/deeper site variants
// are used for small text where its contrast against the surface is insufficient.
pub const SIEGE_ORANGE: Color32 = Color32::from_rgb(0xd5, 0x43, 0x29);
pub const SIEGE_ORANGE_LIGHT: Color32 = Color32::from_rgb(0xf2, 0x69, 0x49);
pub const SIEGE_ORANGE_DEEP: Color32 = Color32::from_rgb(0xad, 0x32, 0x1e);

pub mod siege_light {
    use egui::Color32;
    pub const BG_APP: Color32 = Color32::from_rgb(244, 243, 238);
    pub const BG_TITLEBAR: Color32 = Color32::from_rgb(235, 233, 224);
    pub const BG_TOOLBAR: Color32 = Color32::from_rgb(251, 250, 246);
    pub const BG_PANEL: Color32 = Color32::from_rgb(251, 250, 246);
    pub const BG_PANEL_ALT: Color32 = Color32::from_rgb(239, 237, 229);
    pub const BG_HOVER: Color32 = Color32::from_rgb(235, 227, 214);
    pub const BG_SELECTED: Color32 = Color32::from_rgb(246, 222, 207);
    pub const BG_ELEVATED: Color32 = Color32::from_rgb(255, 253, 248);
    pub const BORDER: Color32 = Color32::from_rgb(207, 203, 191);
    pub const BORDER_STRONG: Color32 = Color32::from_rgb(133, 125, 109);
    pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(36, 37, 32);
    pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(60, 62, 54);
    pub const TEXT_MUTED: Color32 = Color32::from_rgb(99, 97, 85);
    pub const BG_DIFF_ADD: Color32 = Color32::from_rgb(229, 240, 218);
    pub const BG_DIFF_DEL: Color32 = Color32::from_rgb(250, 228, 222);
    pub const BG_DIFF_HUNK: Color32 = Color32::from_rgb(237, 230, 214);
}

pub mod siege_dark {
    use egui::Color32;
    pub const BG_APP: Color32 = Color32::from_rgb(25, 26, 23);
    pub const BG_TITLEBAR: Color32 = Color32::from_rgb(20, 21, 19);
    pub const BG_TOOLBAR: Color32 = Color32::from_rgb(31, 32, 29);
    pub const BG_PANEL: Color32 = Color32::from_rgb(31, 32, 29);
    pub const BG_PANEL_ALT: Color32 = Color32::from_rgb(39, 40, 36);
    pub const BG_HOVER: Color32 = Color32::from_rgb(45, 45, 39);
    pub const BG_SELECTED: Color32 = Color32::from_rgb(60, 39, 32);
    pub const BG_ELEVATED: Color32 = Color32::from_rgb(44, 45, 39);
    pub const BORDER: Color32 = Color32::from_rgb(65, 66, 57);
    pub const BORDER_STRONG: Color32 = Color32::from_rgb(127, 126, 112);
    pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(251, 250, 246);
    pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(217, 215, 203);
    pub const TEXT_MUTED: Color32 = Color32::from_rgb(174, 172, 157);
    pub const BG_DIFF_ADD: Color32 = Color32::from_rgb(29, 47, 31);
    pub const BG_DIFF_DEL: Color32 = Color32::from_rgb(59, 30, 29);
    pub const BG_DIFF_HUNK: Color32 = Color32::from_rgb(49, 42, 30);
}

#[inline]
fn dark() -> bool {
    DARK_MODE.load(Ordering::Relaxed)
}
#[inline]
fn high_contrast() -> bool {
    HIGH_CONTRAST.load(Ordering::Relaxed)
}
pub fn set_dark(d: bool) {
    DARK_MODE.store(d, Ordering::Relaxed);
}
pub fn set_high_contrast(enabled: bool) {
    HIGH_CONTRAST.store(enabled, Ordering::Relaxed);
}

// ── Light mode palette ────────────────────────────────────────────────────────
pub mod light {
    use egui::Color32;
    pub const BG_APP: Color32 = Color32::from_rgb(247, 248, 251);
    pub const BG_TITLEBAR: Color32 = Color32::from_rgb(255, 255, 255);
    pub const BG_TOOLBAR: Color32 = Color32::from_rgb(255, 255, 255);
    pub const BG_PANEL: Color32 = Color32::from_rgb(255, 255, 255);
    pub const BG_PANEL_ALT: Color32 = Color32::from_rgb(241, 243, 248);
    pub const BG_HOVER: Color32 = Color32::from_rgb(234, 238, 247);
    pub const BG_SELECTED: Color32 = Color32::from_rgb(229, 237, 255);
    pub const BG_ELEVATED: Color32 = Color32::from_rgb(249, 250, 253);
    pub const BORDER: Color32 = Color32::from_rgb(221, 226, 236);
    pub const BORDER_STRONG: Color32 = Color32::from_rgb(152, 165, 188);
    pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(29, 36, 51);
    pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(57, 68, 86);
    pub const TEXT_MUTED: Color32 = Color32::from_rgb(78, 90, 109);
    pub const BG_DIFF_ADD: Color32 = Color32::from_rgb(230, 246, 237);
    pub const BG_DIFF_DEL: Color32 = Color32::from_rgb(253, 234, 237);
    pub const BG_DIFF_HUNK: Color32 = Color32::from_rgb(235, 240, 250);
}

pub mod high_contrast_light {
    use egui::Color32;
    pub const BG_APP: Color32 = Color32::from_rgb(255, 255, 255);
    pub const BG_TITLEBAR: Color32 = Color32::from_rgb(226, 229, 235);
    pub const BG_TOOLBAR: Color32 = Color32::from_rgb(238, 241, 247);
    pub const BG_PANEL: Color32 = Color32::from_rgb(255, 255, 255);
    pub const BG_PANEL_ALT: Color32 = Color32::from_rgb(236, 240, 247);
    pub const BG_HOVER: Color32 = Color32::from_rgb(196, 210, 234);
    pub const BG_SELECTED: Color32 = Color32::from_rgb(180, 204, 244);
    pub const BG_ELEVATED: Color32 = Color32::from_rgb(248, 250, 253);
    pub const BORDER: Color32 = Color32::from_rgb(128, 139, 158);
    pub const BORDER_STRONG: Color32 = Color32::from_rgb(48, 60, 82);
    pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(0, 0, 0);
    pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(18, 25, 38);
    pub const TEXT_MUTED: Color32 = Color32::from_rgb(48, 58, 78);
    pub const BG_DIFF_ADD: Color32 = Color32::from_rgb(188, 238, 198);
    pub const BG_DIFF_DEL: Color32 = Color32::from_rgb(250, 196, 196);
    pub const BG_DIFF_HUNK: Color32 = Color32::from_rgb(206, 220, 250);
}

pub mod high_contrast_dark {
    use egui::Color32;
    pub const BG_APP: Color32 = Color32::from_rgb(0, 0, 0);
    pub const BG_TITLEBAR: Color32 = Color32::from_rgb(0, 0, 0);
    pub const BG_TOOLBAR: Color32 = Color32::from_rgb(8, 10, 14);
    pub const BG_PANEL: Color32 = Color32::from_rgb(12, 15, 22);
    pub const BG_PANEL_ALT: Color32 = Color32::from_rgb(22, 27, 38);
    pub const BG_HOVER: Color32 = Color32::from_rgb(58, 72, 98);
    pub const BG_SELECTED: Color32 = Color32::from_rgb(38, 78, 126);
    pub const BG_ELEVATED: Color32 = Color32::from_rgb(26, 32, 44);
    pub const BORDER: Color32 = Color32::from_rgb(108, 124, 154);
    pub const BORDER_STRONG: Color32 = Color32::from_rgb(174, 194, 224);
    pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(255, 255, 255);
    pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(226, 234, 246);
    pub const TEXT_MUTED: Color32 = Color32::from_rgb(174, 188, 212);
    pub const BG_DIFF_ADD: Color32 = Color32::from_rgb(10, 58, 22);
    pub const BG_DIFF_DEL: Color32 = Color32::from_rgb(72, 12, 16);
    pub const BG_DIFF_HUNK: Color32 = Color32::from_rgb(22, 42, 82);
}

#[inline]
fn pick(
    normal_dark: Color32,
    normal_light: Color32,
    contrast_dark: Color32,
    contrast_light: Color32,
    siege_dark: Color32,
    siege_light: Color32,
) -> Color32 {
    match (dark(), high_contrast()) {
        (true, true) => contrast_dark,
        (true, false) if siege() => siege_dark,
        (true, false) => normal_dark,
        (false, true) => contrast_light,
        (false, false) if siege() => siege_light,
        (false, false) => normal_light,
    }
}

// ── Dynamic colour accessors — use these in UI paint calls ────────────────────
#[inline]
pub fn bg_app() -> Color32 {
    pick(
        BG_APP,
        light::BG_APP,
        high_contrast_dark::BG_APP,
        high_contrast_light::BG_APP,
        siege_dark::BG_APP,
        siege_light::BG_APP,
    )
}
#[inline]
pub fn bg_titlebar() -> Color32 {
    pick(
        BG_TITLEBAR,
        light::BG_TITLEBAR,
        high_contrast_dark::BG_TITLEBAR,
        high_contrast_light::BG_TITLEBAR,
        siege_dark::BG_TITLEBAR,
        siege_light::BG_TITLEBAR,
    )
}
#[inline]
pub fn bg_toolbar() -> Color32 {
    pick(
        BG_TOOLBAR,
        light::BG_TOOLBAR,
        high_contrast_dark::BG_TOOLBAR,
        high_contrast_light::BG_TOOLBAR,
        siege_dark::BG_TOOLBAR,
        siege_light::BG_TOOLBAR,
    )
}
#[inline]
pub fn bg_panel() -> Color32 {
    pick(
        BG_PANEL,
        light::BG_PANEL,
        high_contrast_dark::BG_PANEL,
        high_contrast_light::BG_PANEL,
        siege_dark::BG_PANEL,
        siege_light::BG_PANEL,
    )
}
#[inline]
pub fn bg_panel_alt() -> Color32 {
    pick(
        BG_PANEL_ALT,
        light::BG_PANEL_ALT,
        high_contrast_dark::BG_PANEL_ALT,
        high_contrast_light::BG_PANEL_ALT,
        siege_dark::BG_PANEL_ALT,
        siege_light::BG_PANEL_ALT,
    )
}
#[inline]
pub fn bg_hover() -> Color32 {
    pick(
        BG_HOVER,
        light::BG_HOVER,
        high_contrast_dark::BG_HOVER,
        high_contrast_light::BG_HOVER,
        siege_dark::BG_HOVER,
        siege_light::BG_HOVER,
    )
}
#[inline]
pub fn bg_selected() -> Color32 {
    pick(
        BG_SELECTED,
        light::BG_SELECTED,
        high_contrast_dark::BG_SELECTED,
        high_contrast_light::BG_SELECTED,
        siege_dark::BG_SELECTED,
        siege_light::BG_SELECTED,
    )
}
#[inline]
pub fn bg_elevated() -> Color32 {
    pick(
        BG_ELEVATED,
        light::BG_ELEVATED,
        high_contrast_dark::BG_ELEVATED,
        high_contrast_light::BG_ELEVATED,
        siege_dark::BG_ELEVATED,
        siege_light::BG_ELEVATED,
    )
}
#[inline]
pub fn bg_surface() -> Color32 {
    bg_panel()
}
#[inline]
pub fn bg_base() -> Color32 {
    bg_app()
}
#[inline]
pub fn border() -> Color32 {
    pick(
        BORDER,
        light::BORDER,
        high_contrast_dark::BORDER,
        high_contrast_light::BORDER,
        siege_dark::BORDER,
        siege_light::BORDER,
    )
}
#[inline]
pub fn border_strong() -> Color32 {
    pick(
        BORDER_STRONG,
        light::BORDER_STRONG,
        high_contrast_dark::BORDER_STRONG,
        high_contrast_light::BORDER_STRONG,
        siege_dark::BORDER_STRONG,
        siege_light::BORDER_STRONG,
    )
}
#[inline]
pub fn text_primary() -> Color32 {
    pick(
        TEXT_PRIMARY,
        light::TEXT_PRIMARY,
        high_contrast_dark::TEXT_PRIMARY,
        high_contrast_light::TEXT_PRIMARY,
        siege_dark::TEXT_PRIMARY,
        siege_light::TEXT_PRIMARY,
    )
}
#[inline]
pub fn text_secondary() -> Color32 {
    pick(
        TEXT_SECONDARY,
        light::TEXT_SECONDARY,
        high_contrast_dark::TEXT_SECONDARY,
        high_contrast_light::TEXT_SECONDARY,
        siege_dark::TEXT_SECONDARY,
        siege_light::TEXT_SECONDARY,
    )
}
#[inline]
pub fn text_muted() -> Color32 {
    pick(
        TEXT_MUTED,
        light::TEXT_MUTED,
        high_contrast_dark::TEXT_MUTED,
        high_contrast_light::TEXT_MUTED,
        siege_dark::TEXT_MUTED,
        siege_light::TEXT_MUTED,
    )
}
#[inline]
pub fn bg_diff_add() -> Color32 {
    pick(
        BG_DIFF_ADD,
        light::BG_DIFF_ADD,
        high_contrast_dark::BG_DIFF_ADD,
        high_contrast_light::BG_DIFF_ADD,
        siege_dark::BG_DIFF_ADD,
        siege_light::BG_DIFF_ADD,
    )
}
#[inline]
pub fn bg_diff_del() -> Color32 {
    pick(
        BG_DIFF_DEL,
        light::BG_DIFF_DEL,
        high_contrast_dark::BG_DIFF_DEL,
        high_contrast_light::BG_DIFF_DEL,
        siege_dark::BG_DIFF_DEL,
        siege_light::BG_DIFF_DEL,
    )
}
#[inline]
pub fn bg_diff_hunk() -> Color32 {
    pick(
        BG_DIFF_HUNK,
        light::BG_DIFF_HUNK,
        high_contrast_dark::BG_DIFF_HUNK,
        high_contrast_light::BG_DIFF_HUNK,
        siege_dark::BG_DIFF_HUNK,
        siege_light::BG_DIFF_HUNK,
    )
}

// ── Background layers ─────────────────────────────────────────────────────────
pub const BG_APP: Color32 = Color32::from_rgb(21, 23, 28);
pub const BG_TITLEBAR: Color32 = Color32::from_rgb(17, 19, 24);
pub const BG_TOOLBAR: Color32 = Color32::from_rgb(27, 30, 37);
pub const BG_PANEL: Color32 = Color32::from_rgb(27, 30, 37);
pub const BG_PANEL_ALT: Color32 = Color32::from_rgb(32, 36, 44);
pub const BG_HOVER: Color32 = Color32::from_rgb(39, 44, 55);
pub const BG_SELECTED: Color32 = Color32::from_rgb(35, 48, 72);
pub const BG_ELEVATED: Color32 = Color32::from_rgb(35, 39, 48);

// Diff-specific tints
pub const BG_DIFF_ADD: Color32 = Color32::from_rgb(24, 46, 37);
pub const BG_DIFF_DEL: Color32 = Color32::from_rgb(54, 30, 39);
pub const BG_DIFF_HUNK: Color32 = Color32::from_rgb(31, 39, 57);

// ── Borders ───────────────────────────────────────────────────────────────────
pub const BORDER: Color32 = Color32::from_rgb(47, 53, 65);
pub const BORDER_STRONG: Color32 = Color32::from_rgb(82, 94, 115);

// ── Text ──────────────────────────────────────────────────────────────────────
pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(236, 239, 246);
pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(187, 197, 214);
pub const TEXT_MUTED: Color32 = Color32::from_rgb(151, 163, 184);

// ── Accents ───────────────────────────────────────────────────────────────────
pub const ACCENT_BLUE: Color32 = Color32::from_rgb(125, 162, 255);
pub const ACCENT_TEAL: Color32 = Color32::from_rgb(96, 210, 180);
pub const ACCENT_PURPLE: Color32 = Color32::from_rgb(185, 158, 255);
pub const ACCENT_ORANGE: Color32 = Color32::from_rgb(248, 170, 104);
pub const ACCENT_GREEN: Color32 = Color32::from_rgb(104, 209, 147);
pub const ACCENT_RED: Color32 = Color32::from_rgb(244, 130, 140);
pub const ACCENT_YELLOW: Color32 = Color32::from_rgb(226, 197, 114);

// Semantic foreground colours are tuned independently for both themes.
fn siege_accent() -> Color32 {
    if dark() {
        if high_contrast() {
            Color32::WHITE
        } else {
            SIEGE_ORANGE_LIGHT
        }
    } else {
        SIEGE_ORANGE_DEEP
    }
}

pub fn accent_blue() -> Color32 {
    if siege() {
        siege_accent()
    } else if dark() {
        ACCENT_BLUE
    } else {
        Color32::from_rgb(49, 88, 198)
    }
}
pub fn accent_teal() -> Color32 {
    if siege() {
        siege_accent()
    } else if dark() {
        ACCENT_TEAL
    } else {
        Color32::from_rgb(13, 115, 94)
    }
}
pub fn accent_purple() -> Color32 {
    if dark() {
        ACCENT_PURPLE
    } else {
        Color32::from_rgb(121, 72, 176)
    }
}
pub fn accent_orange() -> Color32 {
    if siege() {
        siege_accent()
    } else if dark() {
        ACCENT_ORANGE
    } else {
        Color32::from_rgb(158, 81, 18)
    }
}
pub fn accent_green() -> Color32 {
    if dark() {
        ACCENT_GREEN
    } else {
        Color32::from_rgb(27, 119, 72)
    }
}
pub fn accent_red() -> Color32 {
    if dark() {
        ACCENT_RED
    } else {
        Color32::from_rgb(184, 49, 72)
    }
}
pub fn accent_yellow() -> Color32 {
    if dark() {
        ACCENT_YELLOW
    } else {
        Color32::from_rgb(143, 100, 22)
    }
}
pub fn primary_fill() -> Color32 {
    if siege() {
        SIEGE_ORANGE
    } else if dark() {
        Color32::from_rgb(66, 100, 206)
    } else {
        Color32::from_rgb(49, 88, 198)
    }
}
pub fn on_primary() -> Color32 {
    Color32::WHITE
}

/// Choose a readable label for opaque coloured badges and semantic buttons.
pub fn contrast_text(background: Color32) -> Color32 {
    if relative_luminance(background) > 0.179 {
        Color32::BLACK
    } else {
        Color32::WHITE
    }
}

fn relative_luminance(color: Color32) -> f32 {
    let linear = |channel: u8| {
        let value = channel as f32 / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
}
pub fn is_dark() -> bool {
    dark()
}

// Aliases kept for existing call-sites
pub const BG_BASE: Color32 = BG_APP;
pub const BG_SURFACE: Color32 = BG_PANEL;
pub const BG_DIFF_ADD_LINE: Color32 = Color32::from_rgb(171, 224, 171);
pub const BG_DIFF_DEL_LINE: Color32 = Color32::from_rgb(240, 160, 160);

// ── Font sizes ────────────────────────────────────────────────────────────────
pub mod font_size {
    pub const TITLE: f32 = 18.0;
    pub const SECTION: f32 = 12.0;
    pub const BODY: f32 = 14.0;
    pub const SMALL: f32 = 12.0;
    pub const CODE: f32 = 13.0;
}

// ── Graph lane colours ────────────────────────────────────────────────────────
pub const LANE_COLORS: &[Color32] = &[
    Color32::from_rgb(0, 229, 255),   // electric cyan
    Color32::from_rgb(255, 70, 203),  // hot pink
    Color32::from_rgb(177, 105, 255), // violet
    Color32::from_rgb(81, 255, 137),  // neon green
    Color32::from_rgb(255, 197, 53),  // amber
    Color32::from_rgb(87, 136, 255),  // electric blue
    Color32::from_rgb(255, 103, 72),  // coral
    Color32::from_rgb(196, 246, 45),  // lime
];

// Saturated counterparts with enough contrast against the light graph surface.
const LANE_COLORS_LIGHT: &[Color32] = &[
    Color32::from_rgb(0, 124, 151),
    Color32::from_rgb(210, 24, 148),
    Color32::from_rgb(124, 52, 224),
    Color32::from_rgb(0, 143, 82),
    Color32::from_rgb(166, 105, 0),
    Color32::from_rgb(44, 96, 237),
    Color32::from_rgb(207, 65, 36),
    Color32::from_rgb(105, 130, 0),
];

/// Stable repository-specific shuffle, without filesystem access or saved state.
/// Use the full path so repositories with identical names can have different orders.
pub fn graph_color_order(repo: Option<&std::path::Path>) -> [usize; 8] {
    use sha2::{Digest, Sha256};
    let mut order = [0, 1, 2, 3, 4, 5, 6, 7];
    if let Some(repo) = repo {
        let seed = Sha256::digest(repo.as_os_str().as_encoded_bytes());
        for i in (1..order.len()).rev() {
            order.swap(i, seed[i] as usize % (i + 1));
        }
    }
    order
}

/// Optimised for deuteranopia/protanopia — no red/green confusion.
pub const LANE_COLORS_COLORBLIND: &[Color32] = &[
    Color32::from_rgb(0, 114, 178),   // blue
    Color32::from_rgb(230, 159, 0),   // orange
    Color32::from_rgb(86, 180, 233),  // sky blue
    Color32::from_rgb(240, 228, 66),  // yellow
    Color32::from_rgb(0, 158, 115),   // bluish-green
    Color32::from_rgb(213, 94, 0),    // vermilion
    Color32::from_rgb(204, 121, 167), // reddish-purple
    Color32::from_rgb(255, 255, 255), // white (for dark bg)
];

pub fn lane_color(lane: usize) -> Color32 {
    let colors = [
        accent_blue(),
        accent_teal(),
        accent_purple(),
        accent_orange(),
        accent_green(),
        accent_red(),
        accent_yellow(),
        text_secondary(),
    ];
    colors[lane % colors.len()]
}

pub fn lane_color_for_palette(lane: usize, colorblind: bool) -> Color32 {
    if colorblind {
        let color = LANE_COLORS_COLORBLIND[lane % LANE_COLORS_COLORBLIND.len()];
        if !dark() && lane % LANE_COLORS_COLORBLIND.len() == 7 {
            text_primary()
        } else if !dark() {
            color.gamma_multiply(0.65)
        } else {
            color
        }
    } else {
        let colors = if dark() {
            LANE_COLORS
        } else {
            LANE_COLORS_LIGHT
        };
        colors[lane % colors.len()]
    }
}

pub fn author_color(name: &str) -> Color32 {
    let h = name
        .bytes()
        .fold(5381u32, |a, b| a.wrapping_mul(33).wrapping_add(b as u32));
    lane_color(h as usize)
}

pub fn author_initials(name: &str) -> String {
    let mut w = name.split_whitespace();
    let f = w.next().and_then(|s| s.chars().next());
    let l = w.last().and_then(|s| s.chars().next());
    match (f, l) {
        (Some(a), Some(b)) => format!("{}{}", a.to_uppercase(), b.to_uppercase()),
        (Some(a), None) => a.to_uppercase().to_string(),
        _ => "?".into(),
    }
}

/// Format a Unix timestamp as an absolute date string, e.g. "Mon 2 Jun 2026 14:32".
pub fn absolute_time(ts: i64) -> String {
    let secs = ts;
    // Days since Unix epoch
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hh = time_of_day / 3600;
    let mm = (time_of_day % 3600) / 60;
    // Compute year/month/day via a simple algorithm
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    // Day of week (0 = Sun)
    let dow = ((days % 7 + 4).rem_euclid(7)) as usize;
    let day_names = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    let month_names = [
        "", "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    format!(
        "{} {} {} {}  {:02}:{:02}",
        day_names[dow], d, month_names[m as usize], y, hh, mm
    )
}

pub fn relative_time(ts: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let d = (now - ts).max(0);
    if d < 60 {
        "just now".into()
    } else if d < 3600 {
        format!("{}m", d / 60)
    } else if d < 86_400 {
        format!("{}h", d / 3600)
    } else if d < 86_400 * 7 {
        format!("{}d", d / 86_400)
    } else if d < 86_400 * 30 {
        format!("{}w", d / (86_400 * 7))
    } else if d < 86_400 * 365 {
        format!("{}mo", d / (86_400 * 30))
    } else {
        format!("{}y", d / (86_400 * 365))
    }
}

// ── Apply to egui context ─────────────────────────────────────────────────────
pub fn apply(ctx: &egui::Context, dark: bool, contrast: bool) {
    apply_palette(ctx, dark, contrast, Palette::Classic);
}

pub fn apply_palette(ctx: &egui::Context, dark: bool, contrast: bool, palette: Palette) {
    set_palette(palette);
    set_dark(dark);
    set_high_contrast(contrast);
    let mut v = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    v.panel_fill = bg_panel();
    v.window_fill = bg_panel();
    v.faint_bg_color = bg_panel_alt();
    v.extreme_bg_color = bg_app();
    v.override_text_color = Some(text_primary());
    v.hyperlink_color = accent_blue();
    v.widgets.noninteractive.bg_fill = bg_panel_alt();
    v.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, text_secondary());
    v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, border());
    v.widgets.inactive.bg_fill = bg_panel_alt();
    v.widgets.inactive.weak_bg_fill = bg_panel_alt();
    v.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, text_secondary());
    v.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, border());
    v.widgets.hovered.bg_fill = bg_hover();
    v.widgets.hovered.weak_bg_fill = bg_hover();
    v.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, text_primary());
    v.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, border_strong());
    v.widgets.active.bg_fill = bg_selected();
    v.widgets.active.weak_bg_fill = bg_selected();
    v.widgets.active.fg_stroke = egui::Stroke::new(1.0, text_primary());
    v.widgets.active.bg_stroke = egui::Stroke::new(1.5, accent_blue());
    v.widgets.open = v.widgets.active;
    for widget in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        widget.corner_radius = egui::CornerRadius::same(7);
    }
    v.selection.bg_fill = bg_selected();
    v.selection.stroke = egui::Stroke::new(1.0, text_primary());
    v.window_stroke = egui::Stroke::new(1.0, border());
    v.window_corner_radius = egui::CornerRadius::same(12);
    v.window_shadow = egui::Shadow {
        offset: [0, 8],
        blur: 28,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 90 } else { 28 }),
    };
    v.popup_shadow = egui::Shadow {
        offset: [0, 4],
        blur: 16,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 80 } else { 24 }),
    };
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.window_margin = egui::Margin::same(16);
        s.spacing.button_padding = egui::vec2(12.0, 7.0);
        s.spacing.interact_size.y = 32.0;
        s.text_styles
            .insert(egui::TextStyle::Heading, egui::FontId::proportional(22.0));
        s.text_styles.insert(
            egui::TextStyle::Body,
            egui::FontId::proportional(font_size::BODY),
        );
        s.text_styles.insert(
            egui::TextStyle::Button,
            egui::FontId::proportional(font_size::BODY),
        );
        s.text_styles.insert(
            egui::TextStyle::Small,
            egui::FontId::proportional(font_size::SMALL),
        );
        s.text_styles.insert(
            egui::TextStyle::Monospace,
            egui::FontId::monospace(font_size::CODE),
        );
        kill_debug(s);
    });
    ctx.options_mut(|o| o.warn_on_id_clash = false);
}

pub fn kill_debug(_style: &mut egui::Style) {
    #[cfg(debug_assertions)]
    {
        _style.debug.show_expand_width = false;
        _style.debug.show_expand_height = false;
        _style.debug.show_resize = false;
        _style.debug.show_interactive_widgets = false;
        _style.debug.show_widget_hits = false;
        _style.debug.debug_on_hover = false;
        _style.debug.debug_on_hover_with_all_modifiers = false;
        _style.debug.hover_shows_next = false;
    }
}

/// UI zoom is relative to the monitor's native pixel density.
pub fn apply_ui_scale(ctx: &egui::Context, scale: f32) {
    ctx.set_zoom_factor(if scale.is_finite() {
        scale.clamp(0.75, 2.0)
    } else {
        1.0
    });
}

#[cfg(test)]
mod typography_tests {
    fn contrast(a: egui::Color32, b: egui::Color32) -> f32 {
        let a = super::relative_luminance(a);
        let b = super::relative_luminance(b);
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn siege_palettes_keep_small_text_readable_on_interactive_surfaces() {
        use super::{siege_dark as d, siege_light as l};
        for (foregrounds, surfaces) in [
            (
                [
                    l::TEXT_PRIMARY,
                    l::TEXT_SECONDARY,
                    l::TEXT_MUTED,
                    super::SIEGE_ORANGE_DEEP,
                ],
                [
                    l::BG_APP,
                    l::BG_PANEL,
                    l::BG_PANEL_ALT,
                    l::BG_HOVER,
                    l::BG_SELECTED,
                    l::BG_ELEVATED,
                ],
            ),
            (
                [
                    d::TEXT_PRIMARY,
                    d::TEXT_SECONDARY,
                    d::TEXT_MUTED,
                    super::SIEGE_ORANGE_LIGHT,
                ],
                [
                    d::BG_APP,
                    d::BG_PANEL,
                    d::BG_PANEL_ALT,
                    d::BG_HOVER,
                    d::BG_SELECTED,
                    d::BG_ELEVATED,
                ],
            ),
        ] {
            for foreground in foregrounds {
                for surface in surfaces {
                    assert!(
                        contrast(foreground, surface) >= 4.5,
                        "Text {foreground:?} on {surface:?} must meet WCAG AA contrast"
                    );
                }
            }
        }
        assert!(contrast(super::SIEGE_ORANGE, super::on_primary()) >= 4.5);
        assert_eq!(
            super::SIEGE_ORANGE,
            egui::Color32::from_rgb(0xd5, 0x43, 0x29)
        );
    }

    #[test]
    fn badge_labels_choose_readable_foregrounds() {
        for background in super::LANE_COLORS
            .iter()
            .chain(super::LANE_COLORS_LIGHT)
            .copied()
            .chain([
                super::ACCENT_BLUE,
                super::ACCENT_TEAL,
                super::ACCENT_PURPLE,
                super::ACCENT_ORANGE,
                super::ACCENT_GREEN,
                super::ACCENT_RED,
                super::ACCENT_YELLOW,
                super::SIEGE_ORANGE,
            ])
        {
            assert!(contrast(background, super::contrast_text(background)) >= 4.5);
        }
    }

    #[test]
    fn repository_graph_colours_are_stable_varied_permutations() {
        use std::{collections::HashSet, path::Path};
        let mut first_colours = HashSet::new();
        for parent in 0..32 {
            // Deliberately reuse the basename: identity must use the full path.
            let repo = format!("/projects/{parent}/recorde");
            let order = super::graph_color_order(Some(Path::new(&repo)));
            assert_eq!(order, super::graph_color_order(Some(Path::new(&repo))));
            first_colours.insert(order[0]);
            let mut sorted = order;
            sorted.sort_unstable();
            assert_eq!(sorted, [0, 1, 2, 3, 4, 5, 6, 7]);
        }
        assert!(
            first_colours.len() >= 6,
            "Repositories should vary their starting hue"
        );
        assert_eq!(super::LANE_COLORS.len(), super::LANE_COLORS_LIGHT.len());
        assert_eq!(
            super::LANE_COLORS.len(),
            super::LANE_COLORS_COLORBLIND.len()
        );
    }

    #[test]
    fn ui_zoom_preserves_native_density_and_monitor_changes() {
        let ctx = egui::Context::default();
        let frame = |native_scale| {
            let mut input = egui::RawInput::default();
            input
                .viewports
                .get_mut(&egui::ViewportId::ROOT)
                .unwrap()
                .native_pixels_per_point = Some(native_scale);
            ctx.begin_pass(input);
            let density = ctx.pixels_per_point();
            ctx.end_pass().textures_delta.clear();
            density
        };
        frame(1.5);
        super::apply_ui_scale(&ctx, 1.25);
        assert!((frame(1.5) - 1.875).abs() < 0.001);
        assert!((frame(2.0) - 2.5).abs() < 0.001);
        super::apply_ui_scale(&ctx, 1.0);
        assert!((frame(2.0) - 2.0).abs() < 0.001);
        super::apply_ui_scale(&ctx, f32::NAN);
        assert!((frame(1.5) - 1.5).abs() < 0.001);
    }
}
