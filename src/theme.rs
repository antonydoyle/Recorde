use egui::Color32;
use std::sync::atomic::{AtomicBool, Ordering};

static DARK_MODE: AtomicBool = AtomicBool::new(true);

#[inline] fn dark() -> bool { DARK_MODE.load(Ordering::Relaxed) }
pub fn set_dark(d: bool) { DARK_MODE.store(d, Ordering::Relaxed); }

// ── Light mode palette ────────────────────────────────────────────────────────
pub mod light {
    use egui::Color32;
    pub const BG_APP:        Color32 = Color32::from_rgb(248, 249, 251);
    pub const BG_TITLEBAR:   Color32 = Color32::from_rgb(235, 238, 244);
    pub const BG_TOOLBAR:    Color32 = Color32::from_rgb(240, 242, 247);
    pub const BG_PANEL:      Color32 = Color32::from_rgb(255, 255, 255);
    pub const BG_PANEL_ALT:  Color32 = Color32::from_rgb(245, 247, 250);
    pub const BG_HOVER:      Color32 = Color32::from_rgb(228, 232, 242);
    pub const BG_SELECTED:   Color32 = Color32::from_rgb(210, 222, 245);
    pub const BG_ELEVATED:   Color32 = Color32::from_rgb(250, 251, 253);
    pub const BORDER:        Color32 = Color32::from_rgb(210, 215, 226);
    pub const BORDER_STRONG: Color32 = Color32::from_rgb(168, 178, 200);
    pub const TEXT_PRIMARY:  Color32 = Color32::from_rgb(22,  28,  42);
    pub const TEXT_SECONDARY:Color32 = Color32::from_rgb(66,  76,  98);
    pub const TEXT_MUTED:    Color32 = Color32::from_rgb(122, 134, 158);
    pub const BG_DIFF_ADD:   Color32 = Color32::from_rgb(205, 242, 213);
    pub const BG_DIFF_DEL:   Color32 = Color32::from_rgb(252, 210, 210);
    pub const BG_DIFF_HUNK:  Color32 = Color32::from_rgb(222, 230, 250);
}

// ── Dynamic colour accessors — use these in UI paint calls ────────────────────
#[inline] pub fn bg_app()        -> Color32 { if dark() { BG_APP       } else { light::BG_APP       } }
#[inline] pub fn bg_titlebar()   -> Color32 { if dark() { BG_TITLEBAR  } else { light::BG_TITLEBAR  } }
#[inline] pub fn bg_toolbar()    -> Color32 { if dark() { BG_TOOLBAR   } else { light::BG_TOOLBAR   } }
#[inline] pub fn bg_panel()      -> Color32 { if dark() { BG_PANEL     } else { light::BG_PANEL     } }
#[inline] pub fn bg_panel_alt()  -> Color32 { if dark() { BG_PANEL_ALT } else { light::BG_PANEL_ALT } }
#[inline] pub fn bg_hover()      -> Color32 { if dark() { BG_HOVER     } else { light::BG_HOVER     } }
#[inline] pub fn bg_selected()   -> Color32 { if dark() { BG_SELECTED  } else { light::BG_SELECTED  } }
#[inline] pub fn bg_elevated()   -> Color32 { if dark() { BG_ELEVATED  } else { light::BG_ELEVATED  } }
#[inline] pub fn bg_surface()    -> Color32 { bg_panel() }
#[inline] pub fn bg_base()       -> Color32 { bg_app()   }
#[inline] pub fn border()        -> Color32 { if dark() { BORDER       } else { light::BORDER       } }
#[inline] pub fn border_strong() -> Color32 { if dark() { BORDER_STRONG} else { light::BORDER_STRONG} }
#[inline] pub fn text_primary()  -> Color32 { if dark() { TEXT_PRIMARY  } else { light::TEXT_PRIMARY  } }
#[inline] pub fn text_secondary()-> Color32 { if dark() { TEXT_SECONDARY} else { light::TEXT_SECONDARY} }
#[inline] pub fn text_muted()    -> Color32 { if dark() { TEXT_MUTED    } else { light::TEXT_MUTED    } }
#[inline] pub fn bg_diff_add()   -> Color32 { if dark() { BG_DIFF_ADD  } else { light::BG_DIFF_ADD  } }
#[inline] pub fn bg_diff_del()   -> Color32 { if dark() { BG_DIFF_DEL  } else { light::BG_DIFF_DEL  } }
#[inline] pub fn bg_diff_hunk()  -> Color32 { if dark() { BG_DIFF_HUNK } else { light::BG_DIFF_HUNK } }

// ── Background layers ─────────────────────────────────────────────────────────
pub const BG_APP:        Color32 = Color32::from_rgb(18,  23,  32);   // #121720 window/app
pub const BG_TITLEBAR:   Color32 = Color32::from_rgb(13,  17,  23);   // #0D1117 title strip
pub const BG_TOOLBAR:    Color32 = Color32::from_rgb(20,  26,  36);   // #141A24 toolbar
pub const BG_PANEL:      Color32 = Color32::from_rgb(25,  31,  42);   // #191F2A panels
pub const BG_PANEL_ALT:  Color32 = Color32::from_rgb(30,  37,  49);   // #1E2531 alt surface
pub const BG_HOVER:      Color32 = Color32::from_rgb(38,  47,  62);   // #262F3E hover
pub const BG_SELECTED:   Color32 = Color32::from_rgb(47,  57,  75);   // #2F394B selected row
pub const BG_ELEVATED:   Color32 = Color32::from_rgb(34,  42,  56);   // #222A38 elevated card

// Diff-specific tints
pub const BG_DIFF_ADD:   Color32 = Color32::from_rgb(14,  38,  20);
pub const BG_DIFF_DEL:   Color32 = Color32::from_rgb(42,  14,  14);
pub const BG_DIFF_HUNK:  Color32 = Color32::from_rgb(20,  28,  46);

// ── Borders ───────────────────────────────────────────────────────────────────
pub const BORDER:        Color32 = Color32::from_rgb(54,  64,  82);   // subtle
pub const BORDER_STRONG: Color32 = Color32::from_rgb(86,  98, 122);   // prominent

// ── Text ──────────────────────────────────────────────────────────────────────
pub const TEXT_PRIMARY:   Color32 = Color32::from_rgb(222, 227, 235);
pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(170, 178, 193);
pub const TEXT_MUTED:     Color32 = Color32::from_rgb(108, 119, 139);

// ── Accents ───────────────────────────────────────────────────────────────────
pub const ACCENT_BLUE:   Color32 = Color32::from_rgb(82,  150, 255);
pub const ACCENT_TEAL:   Color32 = Color32::from_rgb(83,  199, 184);  // current branch, active tabs
pub const ACCENT_PURPLE: Color32 = Color32::from_rgb(165, 103, 232);
pub const ACCENT_ORANGE: Color32 = Color32::from_rgb(255, 139,  55);
pub const ACCENT_GREEN:  Color32 = Color32::from_rgb(74,  208, 128);
pub const ACCENT_RED:    Color32 = Color32::from_rgb(239,  93,  93);
pub const ACCENT_YELLOW: Color32 = Color32::from_rgb(245, 190,  69);

// Aliases kept for existing call-sites
pub const BG_BASE:     Color32 = BG_APP;
pub const BG_SURFACE:  Color32 = BG_PANEL;
pub const BG_DIFF_ADD_LINE:  Color32 = Color32::from_rgb(171, 224, 171);
pub const BG_DIFF_DEL_LINE:  Color32 = Color32::from_rgb(240, 160, 160);

// ── Font sizes ────────────────────────────────────────────────────────────────
pub mod font_size {
    pub const TITLE:   f32 = 18.0;
    pub const SECTION: f32 = 11.5;
    pub const BODY:    f32 = 13.0;
    pub const SMALL:   f32 = 11.5;
    pub const CODE:    f32 = 12.5;
}

// ── Graph lane colours ────────────────────────────────────────────────────────
pub const LANE_COLORS: &[Color32] = &[
    Color32::from_rgb(82,  150, 255), // blue
    Color32::from_rgb(245, 190,  69), // amber
    Color32::from_rgb(165, 103, 232), // purple
    Color32::from_rgb(255, 139,  55), // orange
    Color32::from_rgb(74,  208, 128), // green
    Color32::from_rgb(83,  199, 184), // teal
    Color32::from_rgb(239,  93,  93), // red
    Color32::from_rgb(200, 140, 255), // lavender
];

/// Optimised for deuteranopia/protanopia — no red/green confusion.
pub const LANE_COLORS_COLORBLIND: &[Color32] = &[
    Color32::from_rgb(  0, 114, 178), // blue
    Color32::from_rgb(230, 159,   0), // orange
    Color32::from_rgb( 86, 180, 233), // sky blue
    Color32::from_rgb(240, 228,  66), // yellow
    Color32::from_rgb(  0, 158, 115), // bluish-green
    Color32::from_rgb(213,  94,   0), // vermilion
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
    let h = name.bytes().fold(5381u32, |a, b| a.wrapping_mul(33).wrapping_add(b as u32));
    LANE_COLORS[h as usize % LANE_COLORS.len()]
}

pub fn author_initials(name: &str) -> String {
    let mut w = name.split_whitespace();
    let f = w.next().and_then(|s| s.chars().next());
    let l = w.last().and_then(|s| s.chars().next());
    match (f, l) {
        (Some(a), Some(b)) => format!("{}{}", a.to_uppercase(), b.to_uppercase()),
        (Some(a), None)    => a.to_uppercase().to_string(),
        _                  => "?".into(),
    }
}

pub fn relative_time(ts: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let d = (now - ts).max(0);
    if d < 60            { "just now".into() }
    else if d < 3600     { format!("{}m", d / 60) }
    else if d < 86_400   { format!("{}h", d / 3600) }
    else if d < 86_400*7 { format!("{}d", d / 86_400) }
    else if d < 86_400*30{ format!("{}w", d / (86_400*7)) }
    else if d < 86_400*365{ format!("{}mo", d / (86_400*30)) }
    else                  { format!("{}y", d / (86_400*365)) }
}

// ── Apply to egui context ─────────────────────────────────────────────────────
pub fn apply(ctx: &egui::Context, dark: bool) {
    set_dark(dark);
    if !dark {
        let mut v = egui::Visuals::light();
        v.panel_fill          = light::BG_PANEL;
        v.window_fill         = light::BG_APP;
        v.faint_bg_color      = light::BG_PANEL_ALT;
        v.extreme_bg_color    = light::BG_APP;
        v.override_text_color = Some(light::TEXT_PRIMARY);

        v.widgets.noninteractive.bg_fill   = light::BG_PANEL;
        v.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, light::TEXT_SECONDARY);
        v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, light::BORDER);
        v.widgets.noninteractive.rounding  = egui::Rounding::same(5.0);

        v.widgets.inactive.bg_fill         = light::BG_PANEL_ALT;
        v.widgets.inactive.fg_stroke       = egui::Stroke::new(1.0, light::TEXT_SECONDARY);
        v.widgets.inactive.bg_stroke       = egui::Stroke::new(1.0, light::BORDER);
        v.widgets.inactive.rounding        = egui::Rounding::same(5.0);

        v.widgets.hovered.bg_fill          = light::BG_HOVER;
        v.widgets.hovered.fg_stroke        = egui::Stroke::new(1.0, light::TEXT_PRIMARY);
        v.widgets.hovered.bg_stroke        = egui::Stroke::new(1.0, light::BORDER_STRONG);
        v.widgets.hovered.rounding         = egui::Rounding::same(5.0);

        v.widgets.active.bg_fill           = light::BG_SELECTED;
        v.widgets.active.fg_stroke         = egui::Stroke::new(1.0, light::TEXT_PRIMARY);
        v.widgets.active.bg_stroke         = egui::Stroke::new(1.0, light::BORDER_STRONG);
        v.widgets.active.rounding          = egui::Rounding::same(5.0);

        v.widgets.open.bg_fill             = light::BG_SELECTED;
        v.widgets.open.fg_stroke           = egui::Stroke::new(1.0, light::TEXT_PRIMARY);

        v.selection.bg_fill = Color32::from_rgba_premultiplied(82, 150, 255, 80);
        v.selection.stroke  = egui::Stroke::new(1.0, Color32::from_rgb(82, 150, 255));

        v.window_stroke  = egui::Stroke::new(1.0, light::BORDER);
        v.popup_shadow   = egui::Shadow::NONE;
        v.window_shadow  = egui::Shadow::NONE;
        v.window_rounding = egui::Rounding::same(8.0);

        ctx.set_visuals(v);
        ctx.style_mut(|s| {
            s.spacing.item_spacing    = egui::Vec2::new(8.0, 5.0);
            s.spacing.window_margin   = egui::Margin::same(0.0);
            s.spacing.button_padding  = egui::Vec2::new(12.0, 7.0);
            s.spacing.interact_size.y = 26.0;
            s.visuals.widgets.inactive.rounding = egui::Rounding::same(5.0);
            s.visuals.widgets.hovered.rounding  = egui::Rounding::same(5.0);
            s.visuals.widgets.active.rounding   = egui::Rounding::same(5.0);
            kill_debug(s);
        });
        ctx.options_mut(|o| o.warn_on_id_clash = false);
        return;
    }

    let mut v = egui::Visuals::dark();
    v.panel_fill               = BG_PANEL;
    v.window_fill              = BG_APP;
    v.faint_bg_color           = BG_PANEL_ALT;
    v.extreme_bg_color         = BG_APP;
    v.override_text_color      = Some(TEXT_PRIMARY);

    v.widgets.noninteractive.bg_fill     = BG_PANEL;
    v.widgets.noninteractive.fg_stroke   = egui::Stroke::new(1.0, TEXT_SECONDARY);
    v.widgets.noninteractive.bg_stroke   = egui::Stroke::new(1.0, BORDER);
    v.widgets.noninteractive.rounding    = egui::Rounding::same(5.0);

    v.widgets.inactive.bg_fill           = BG_PANEL_ALT;
    v.widgets.inactive.fg_stroke         = egui::Stroke::new(1.0, TEXT_SECONDARY);
    v.widgets.inactive.bg_stroke         = egui::Stroke::new(1.0, BORDER);
    v.widgets.inactive.rounding          = egui::Rounding::same(5.0);

    v.widgets.hovered.bg_fill            = BG_HOVER;
    v.widgets.hovered.fg_stroke          = egui::Stroke::new(1.0, TEXT_PRIMARY);
    v.widgets.hovered.bg_stroke          = egui::Stroke::new(1.0, BORDER_STRONG);
    v.widgets.hovered.rounding           = egui::Rounding::same(5.0);

    v.widgets.active.bg_fill             = BG_SELECTED;
    v.widgets.active.fg_stroke           = egui::Stroke::new(1.0, TEXT_PRIMARY);
    v.widgets.active.bg_stroke           = egui::Stroke::new(1.0, ACCENT_TEAL);
    v.widgets.active.rounding            = egui::Rounding::same(5.0);

    v.widgets.open.bg_fill               = BG_SELECTED;
    v.widgets.open.fg_stroke             = egui::Stroke::new(1.0, TEXT_PRIMARY);

    v.selection.bg_fill  = Color32::from_rgba_premultiplied(82, 150, 255, 55);
    v.selection.stroke   = egui::Stroke::new(1.0, ACCENT_BLUE);

    v.window_stroke  = egui::Stroke::new(1.0, BORDER);
    v.popup_shadow   = egui::Shadow::NONE;
    v.window_shadow  = egui::Shadow::NONE;
    v.window_rounding = egui::Rounding::same(8.0);

    ctx.set_visuals(v);
    ctx.style_mut(|s| {
        s.spacing.item_spacing     = egui::Vec2::new(8.0, 5.0);
        s.spacing.window_margin    = egui::Margin::same(0.0);
        s.spacing.button_padding   = egui::Vec2::new(12.0, 7.0);
        s.spacing.interact_size.y  = 26.0;
        s.visuals.widgets.inactive.rounding = egui::Rounding::same(5.0);
        s.visuals.widgets.hovered.rounding  = egui::Rounding::same(5.0);
        s.visuals.widgets.active.rounding   = egui::Rounding::same(5.0);
        kill_debug(s);
    });
    ctx.options_mut(|o| o.warn_on_id_clash = false);
}

pub fn kill_debug(_style: &mut egui::Style) {
    #[cfg(debug_assertions)]
    {
        _style.debug.show_expand_width        = false;
        _style.debug.show_expand_height       = false;
        _style.debug.show_resize              = false;
        _style.debug.show_interactive_widgets = false;
        _style.debug.show_widget_hits         = false;
        _style.debug.debug_on_hover                    = false;
        _style.debug.debug_on_hover_with_all_modifiers = false;
        _style.debug.hover_shows_next                  = false;
    }
}
