use egui::Color32;
use std::sync::atomic::{AtomicBool, Ordering};

static DARK_MODE: AtomicBool = AtomicBool::new(true);
static HIGH_CONTRAST: AtomicBool = AtomicBool::new(false);

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
    pub const BG_APP: Color32 = Color32::from_rgb(248, 249, 251);
    pub const BG_TITLEBAR: Color32 = Color32::from_rgb(235, 238, 244);
    pub const BG_TOOLBAR: Color32 = Color32::from_rgb(240, 242, 247);
    pub const BG_PANEL: Color32 = Color32::from_rgb(255, 255, 255);
    pub const BG_PANEL_ALT: Color32 = Color32::from_rgb(245, 247, 250);
    pub const BG_HOVER: Color32 = Color32::from_rgb(216, 223, 238);
    pub const BG_SELECTED: Color32 = Color32::from_rgb(210, 222, 245);
    pub const BG_ELEVATED: Color32 = Color32::from_rgb(250, 251, 253);
    pub const BORDER: Color32 = Color32::from_rgb(210, 215, 226);
    pub const BORDER_STRONG: Color32 = Color32::from_rgb(168, 178, 200);
    pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(18, 23, 34);
    pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(42, 50, 68);
    pub const TEXT_MUTED: Color32 = Color32::from_rgb(82, 94, 118);
    pub const BG_DIFF_ADD: Color32 = Color32::from_rgb(205, 242, 213);
    pub const BG_DIFF_DEL: Color32 = Color32::from_rgb(252, 210, 210);
    pub const BG_DIFF_HUNK: Color32 = Color32::from_rgb(222, 230, 250);
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
) -> Color32 {
    match (dark(), high_contrast()) {
        (true, true) => contrast_dark,
        (true, false) => normal_dark,
        (false, true) => contrast_light,
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
    )
}
#[inline]
pub fn bg_titlebar() -> Color32 {
    pick(
        BG_TITLEBAR,
        light::BG_TITLEBAR,
        high_contrast_dark::BG_TITLEBAR,
        high_contrast_light::BG_TITLEBAR,
    )
}
#[inline]
pub fn bg_toolbar() -> Color32 {
    pick(
        BG_TOOLBAR,
        light::BG_TOOLBAR,
        high_contrast_dark::BG_TOOLBAR,
        high_contrast_light::BG_TOOLBAR,
    )
}
#[inline]
pub fn bg_panel() -> Color32 {
    pick(
        BG_PANEL,
        light::BG_PANEL,
        high_contrast_dark::BG_PANEL,
        high_contrast_light::BG_PANEL,
    )
}
#[inline]
pub fn bg_panel_alt() -> Color32 {
    pick(
        BG_PANEL_ALT,
        light::BG_PANEL_ALT,
        high_contrast_dark::BG_PANEL_ALT,
        high_contrast_light::BG_PANEL_ALT,
    )
}
#[inline]
pub fn bg_hover() -> Color32 {
    pick(
        BG_HOVER,
        light::BG_HOVER,
        high_contrast_dark::BG_HOVER,
        high_contrast_light::BG_HOVER,
    )
}
#[inline]
pub fn bg_selected() -> Color32 {
    pick(
        BG_SELECTED,
        light::BG_SELECTED,
        high_contrast_dark::BG_SELECTED,
        high_contrast_light::BG_SELECTED,
    )
}
#[inline]
pub fn bg_elevated() -> Color32 {
    pick(
        BG_ELEVATED,
        light::BG_ELEVATED,
        high_contrast_dark::BG_ELEVATED,
        high_contrast_light::BG_ELEVATED,
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
    )
}
#[inline]
pub fn border_strong() -> Color32 {
    pick(
        BORDER_STRONG,
        light::BORDER_STRONG,
        high_contrast_dark::BORDER_STRONG,
        high_contrast_light::BORDER_STRONG,
    )
}
#[inline]
pub fn text_primary() -> Color32 {
    pick(
        TEXT_PRIMARY,
        light::TEXT_PRIMARY,
        high_contrast_dark::TEXT_PRIMARY,
        high_contrast_light::TEXT_PRIMARY,
    )
}
#[inline]
pub fn text_secondary() -> Color32 {
    pick(
        TEXT_SECONDARY,
        light::TEXT_SECONDARY,
        high_contrast_dark::TEXT_SECONDARY,
        high_contrast_light::TEXT_SECONDARY,
    )
}
#[inline]
pub fn text_muted() -> Color32 {
    pick(
        TEXT_MUTED,
        light::TEXT_MUTED,
        high_contrast_dark::TEXT_MUTED,
        high_contrast_light::TEXT_MUTED,
    )
}
#[inline]
pub fn bg_diff_add() -> Color32 {
    pick(
        BG_DIFF_ADD,
        light::BG_DIFF_ADD,
        high_contrast_dark::BG_DIFF_ADD,
        high_contrast_light::BG_DIFF_ADD,
    )
}
#[inline]
pub fn bg_diff_del() -> Color32 {
    pick(
        BG_DIFF_DEL,
        light::BG_DIFF_DEL,
        high_contrast_dark::BG_DIFF_DEL,
        high_contrast_light::BG_DIFF_DEL,
    )
}
#[inline]
pub fn bg_diff_hunk() -> Color32 {
    pick(
        BG_DIFF_HUNK,
        light::BG_DIFF_HUNK,
        high_contrast_dark::BG_DIFF_HUNK,
        high_contrast_light::BG_DIFF_HUNK,
    )
}

// ── Background layers ─────────────────────────────────────────────────────────
pub const BG_APP: Color32 = Color32::from_rgb(18, 23, 32); // #121720 window/app
pub const BG_TITLEBAR: Color32 = Color32::from_rgb(13, 17, 23); // #0D1117 title strip
pub const BG_TOOLBAR: Color32 = Color32::from_rgb(20, 26, 36); // #141A24 toolbar
pub const BG_PANEL: Color32 = Color32::from_rgb(25, 31, 42); // #191F2A panels
pub const BG_PANEL_ALT: Color32 = Color32::from_rgb(30, 37, 49); // #1E2531 alt surface
pub const BG_HOVER: Color32 = Color32::from_rgb(48, 59, 78); // #303B4E hover
pub const BG_SELECTED: Color32 = Color32::from_rgb(47, 57, 75); // #2F394B selected row
pub const BG_ELEVATED: Color32 = Color32::from_rgb(34, 42, 56); // #222A38 elevated card

// Diff-specific tints
pub const BG_DIFF_ADD: Color32 = Color32::from_rgb(14, 38, 20);
pub const BG_DIFF_DEL: Color32 = Color32::from_rgb(42, 14, 14);
pub const BG_DIFF_HUNK: Color32 = Color32::from_rgb(20, 28, 46);

// ── Borders ───────────────────────────────────────────────────────────────────
pub const BORDER: Color32 = Color32::from_rgb(54, 64, 82); // subtle
pub const BORDER_STRONG: Color32 = Color32::from_rgb(86, 98, 122); // prominent

// ── Text ──────────────────────────────────────────────────────────────────────
pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(222, 227, 235);
pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(170, 178, 193);
pub const TEXT_MUTED: Color32 = Color32::from_rgb(108, 119, 139);

// ── Accents ───────────────────────────────────────────────────────────────────
pub const ACCENT_BLUE: Color32 = Color32::from_rgb(82, 150, 255);
pub const ACCENT_TEAL: Color32 = Color32::from_rgb(83, 199, 184); // current branch, active tabs
pub const ACCENT_PURPLE: Color32 = Color32::from_rgb(165, 103, 232);
pub const ACCENT_ORANGE: Color32 = Color32::from_rgb(255, 139, 55);
pub const ACCENT_GREEN: Color32 = Color32::from_rgb(74, 208, 128);
pub const ACCENT_RED: Color32 = Color32::from_rgb(239, 93, 93);
pub const ACCENT_YELLOW: Color32 = Color32::from_rgb(245, 190, 69);

// Aliases kept for existing call-sites
pub const BG_BASE: Color32 = BG_APP;
pub const BG_SURFACE: Color32 = BG_PANEL;
pub const BG_DIFF_ADD_LINE: Color32 = Color32::from_rgb(171, 224, 171);
pub const BG_DIFF_DEL_LINE: Color32 = Color32::from_rgb(240, 160, 160);

// ── Font sizes ────────────────────────────────────────────────────────────────
pub mod font_size {
    pub const TITLE: f32 = 18.0;
    pub const SECTION: f32 = 11.5;
    pub const BODY: f32 = 13.0;
    pub const SMALL: f32 = 11.5;
    pub const CODE: f32 = 12.5;
}

// ── Graph lane colours ────────────────────────────────────────────────────────
pub const LANE_COLORS: &[Color32] = &[
    Color32::from_rgb(82, 150, 255),  // blue
    Color32::from_rgb(245, 190, 69),  // amber
    Color32::from_rgb(165, 103, 232), // purple
    Color32::from_rgb(255, 139, 55),  // orange
    Color32::from_rgb(74, 208, 128),  // green
    Color32::from_rgb(83, 199, 184),  // teal
    Color32::from_rgb(239, 93, 93),   // red
    Color32::from_rgb(200, 140, 255), // lavender
];

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
    LANE_COLORS[lane % LANE_COLORS.len()]
}

pub fn lane_color_for_palette(lane: usize, colorblind: bool) -> Color32 {
    if colorblind {
        LANE_COLORS_COLORBLIND[lane % LANE_COLORS_COLORBLIND.len()]
    } else {
        LANE_COLORS[lane % LANE_COLORS.len()]
    }
}

pub fn author_color(name: &str) -> Color32 {
    let h = name
        .bytes()
        .fold(5381u32, |a, b| a.wrapping_mul(33).wrapping_add(b as u32));
    LANE_COLORS[h as usize % LANE_COLORS.len()]
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
    set_dark(dark);
    set_high_contrast(contrast);
    if !dark {
        let mut v = egui::Visuals::light();
        v.panel_fill = bg_panel();
        v.window_fill = bg_app();
        v.faint_bg_color = bg_panel_alt();
        v.extreme_bg_color = bg_app();
        v.override_text_color = Some(text_primary());

        v.widgets.noninteractive.bg_fill = bg_panel();
        v.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, text_secondary());
        v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, border());
        v.widgets.noninteractive.rounding = egui::Rounding::same(5.0);

        v.widgets.inactive.bg_fill = bg_panel_alt();
        v.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, text_secondary());
        v.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, border());
        v.widgets.inactive.rounding = egui::Rounding::same(5.0);

        v.widgets.hovered.bg_fill = bg_hover();
        v.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, text_primary());
        v.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, border_strong());
        v.widgets.hovered.rounding = egui::Rounding::same(5.0);

        v.widgets.active.bg_fill = bg_selected();
        v.widgets.active.fg_stroke = egui::Stroke::new(1.0, text_primary());
        v.widgets.active.bg_stroke = egui::Stroke::new(1.0, border_strong());
        v.widgets.active.rounding = egui::Rounding::same(5.0);

        v.widgets.open.bg_fill = bg_selected();
        v.widgets.open.fg_stroke = egui::Stroke::new(1.0, text_primary());

        v.selection.bg_fill = Color32::from_rgba_premultiplied(82, 150, 255, 80);
        v.selection.stroke = egui::Stroke::new(1.0, Color32::from_rgb(82, 150, 255));

        v.window_stroke = egui::Stroke::new(1.0, border());
        v.popup_shadow = egui::Shadow::NONE;
        v.window_shadow = egui::Shadow::NONE;
        v.window_rounding = egui::Rounding::same(8.0);

        ctx.set_visuals(v);
        ctx.style_mut(|s| {
            s.spacing.item_spacing = egui::Vec2::new(8.0, 5.0);
            s.spacing.window_margin = egui::Margin::same(0.0);
            s.spacing.button_padding = egui::Vec2::new(12.0, 7.0);
            s.spacing.interact_size.y = 28.0;
            s.visuals.widgets.inactive.rounding = egui::Rounding::same(5.0);
            s.visuals.widgets.hovered.rounding = egui::Rounding::same(5.0);
            s.visuals.widgets.active.rounding = egui::Rounding::same(5.0);
            kill_debug(s);
        });
        ctx.options_mut(|o| o.warn_on_id_clash = false);
        return;
    }

    let mut v = egui::Visuals::dark();
    v.panel_fill = bg_panel();
    v.window_fill = bg_app();
    v.faint_bg_color = bg_panel_alt();
    v.extreme_bg_color = bg_app();
    v.override_text_color = Some(text_primary());

    v.widgets.noninteractive.bg_fill = bg_panel();
    v.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, text_secondary());
    v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, border());
    v.widgets.noninteractive.rounding = egui::Rounding::same(5.0);

    v.widgets.inactive.bg_fill = bg_panel_alt();
    v.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, text_secondary());
    v.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, border());
    v.widgets.inactive.rounding = egui::Rounding::same(5.0);

    v.widgets.hovered.bg_fill = bg_hover();
    v.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, text_primary());
    v.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, border_strong());
    v.widgets.hovered.rounding = egui::Rounding::same(5.0);

    v.widgets.active.bg_fill = bg_selected();
    v.widgets.active.fg_stroke = egui::Stroke::new(1.0, text_primary());
    v.widgets.active.bg_stroke = egui::Stroke::new(1.0, ACCENT_TEAL);
    v.widgets.active.rounding = egui::Rounding::same(5.0);

    v.widgets.open.bg_fill = bg_selected();
    v.widgets.open.fg_stroke = egui::Stroke::new(1.0, text_primary());

    v.selection.bg_fill = Color32::from_rgba_premultiplied(82, 150, 255, 55);
    v.selection.stroke = egui::Stroke::new(1.0, ACCENT_BLUE);

    v.window_stroke = egui::Stroke::new(1.0, border());
    v.popup_shadow = egui::Shadow::NONE;
    v.window_shadow = egui::Shadow::NONE;
    v.window_rounding = egui::Rounding::same(8.0);

    ctx.set_visuals(v);
    ctx.style_mut(|s| {
        s.spacing.item_spacing = egui::Vec2::new(8.0, 5.0);
        s.spacing.window_margin = egui::Margin::same(0.0);
        s.spacing.button_padding = egui::Vec2::new(12.0, 7.0);
        s.spacing.interact_size.y = 28.0;
        s.visuals.widgets.inactive.rounding = egui::Rounding::same(5.0);
        s.visuals.widgets.hovered.rounding = egui::Rounding::same(5.0);
        s.visuals.widgets.active.rounding = egui::Rounding::same(5.0);
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
