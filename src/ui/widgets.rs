use egui::{Color32, FontId, Pos2, Rect, Response, RichText, Stroke, Ui, Vec2};

use crate::icons;
use crate::theme;

// ── Pill / badge ──────────────────────────────────────────────────────────────

/// Draw a fully-rounded pill badge at `rect`.
pub fn pill_rect(painter: &egui::Painter, rect: Rect, bg: Color32, text: &str, text_color: Color32, font_size: f32) {
    painter.rect_filled(rect, rect.height() / 2.0, bg);
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        FontId::proportional(font_size),
        text_color,
    );
}

/// Add a pill label widget and return its response.
pub fn pill(ui: &mut Ui, text: &str, bg: Color32, text_color: Color32) -> Response {
    let font = FontId::proportional(theme::font_size::SMALL);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font.clone(), text_color);
    let pad = Vec2::new(8.0, 3.0);
    let size = galley.size() + pad * 2.0;
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::hover());
    ui.painter().rect_filled(rect, rect.height() / 2.0, bg);
    ui.painter().galley(rect.min + pad, galley, text_color);
    resp
}

// ── Status badge (M / A / D / R) ─────────────────────────────────────────────

pub fn status_badge_color(ch: char) -> Color32 {
    match ch {
        'A' | '?' => theme::ACCENT_GREEN,
        'D' | 'd' => theme::ACCENT_RED,
        'R'       => theme::ACCENT_BLUE,
        _         => theme::ACCENT_YELLOW, // M and others
    }
}

pub fn status_badge(ui: &mut Ui, ch: char) -> Response {
    let color = status_badge_color(ch);
    let label_ch = match ch { '?' => '?', 'd' => 'D', _ => ch };
    let size = Vec2::splat(18.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::hover());
    ui.painter().rect_filled(rect, 3.0, color.gamma_multiply(0.25));
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label_ch.to_string(),
        FontId::monospace(10.5),
        color,
    );
    resp
}

// ── Section header (sidebar / panel) ─────────────────────────────────────────

/// Renders a collapsible section header. Returns true if the section is open.
pub fn section_header(ui: &mut Ui, title: &str, count: usize, open: &mut bool) -> Response {
    let resp = ui.horizontal(|ui| {
        ui.add_space(8.0);
        // Chevron
        let chevron = if *open { "▾" } else { "▸" };
        ui.label(RichText::new(chevron).color(theme::TEXT_MUTED).size(9.0));
        ui.add_space(4.0);
        ui.label(
            RichText::new(title)
                .size(theme::font_size::SECTION)
                .color(theme::TEXT_MUTED)
                .strong(),
        );
        if count > 0 {
            ui.add_space(6.0);
            // Small count pill
            let count_str = count.to_string();
            pill(
                ui,
                &count_str,
                theme::BG_ELEVATED,
                theme::TEXT_MUTED,
            );
        }
    }).response;

    if resp.interact(egui::Sense::click()).clicked() {
        *open = !*open;
    }
    resp
}

// ── Toolbar button ────────────────────────────────────────────────────────────

pub fn toolbar_btn(ui: &mut Ui, label: &str, enabled: bool) -> Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(
            RichText::new(label)
                .color(if enabled { theme::TEXT_SECONDARY } else { theme::TEXT_MUTED })
                .size(theme::font_size::BODY),
        )
        .fill(theme::BG_PANEL_ALT)
        .stroke(Stroke::new(1.0, theme::BORDER))
        .rounding(6.0),
    )
}

/// Toolbar button with a Material Icon glyph followed by a text label.
pub fn toolbar_btn_icon(ui: &mut Ui, icon: char, label: &str, enabled: bool) -> Response {
    let col = if enabled { theme::TEXT_SECONDARY } else { theme::TEXT_MUTED };
    let mut job = egui::text::LayoutJob::default();
    job.append(
        &icon.to_string(),
        0.0,
        egui::TextFormat { font_id: icons::font(15.0), color: col, ..Default::default() },
    );
    job.append(
        &format!("  {label}"),
        0.0,
        egui::TextFormat { font_id: egui::FontId::proportional(12.5), color: col, ..Default::default() },
    );
    ui.add_enabled(
        enabled,
        egui::Button::new(job)
            .fill(theme::BG_PANEL_ALT)
            .stroke(Stroke::new(1.0, theme::BORDER))
            .rounding(6.0),
    )
}

/// Icon-only button using Material Icons font.
pub fn icon_btn_mat(ui: &mut Ui, icon: char, size: f32, tip: &str) -> Response {
    ui.add(
        egui::Button::new(icons::rich(icon, size).color(theme::TEXT_MUTED))
            .fill(egui::Color32::TRANSPARENT)
            .stroke(Stroke::NONE),
    )
    .on_hover_text(tip)
}

// ── Branch pill (graph labels) ────────────────────────────────────────────────

/// Draw a branch-label pill inline. Returns the right edge x.
pub fn branch_label(
    painter: &egui::Painter,
    pos: Pos2,
    name: &str,
    row_top: f32,
    row_height: f32,
) -> f32 {
    let font = FontId::proportional(10.5);
    let galley = painter.layout_no_wrap(name.to_string(), font, Color32::WHITE);
    let pad_x = 6.0;
    let pad_y = 2.0;
    let w = galley.size().x + pad_x * 2.0;
    let h = (galley.size().y + pad_y * 2.0).min(row_height - 6.0);
    let rect = Rect::from_min_size(
        Pos2::new(pos.x, row_top + (row_height - h) / 2.0),
        Vec2::new(w, h),
    );

    // colour by remote vs local vs tag
    let bg = if name.contains('/') {
        Color32::from_rgb(28, 42, 68)
    } else if name.starts_with('v') && name.chars().nth(1).map_or(false, |c| c.is_ascii_digit()) {
        Color32::from_rgb(42, 28, 68)
    } else {
        Color32::from_rgb(24, 48, 92)
    };
    let border_col = if name.contains('/') { theme::ACCENT_BLUE.gamma_multiply(0.5) } else { theme::ACCENT_TEAL.gamma_multiply(0.6) };

    painter.rect_filled(rect, rect.height() / 2.0, bg);
    painter.rect_stroke(rect, rect.height() / 2.0, Stroke::new(0.8, border_col));
    painter.galley(
        Pos2::new(rect.min.x + pad_x, rect.min.y + (h - galley.size().y) / 2.0),
        galley,
        Color32::WHITE,
    );
    rect.max.x
}

// ── Avatar circle ─────────────────────────────────────────────────────────────

pub fn avatar(painter: &egui::Painter, center: Pos2, radius: f32, name: &str) {
    let color = theme::author_color(name);
    painter.circle_filled(center, radius, color);
    painter.text(
        center,
        egui::Align2::CENTER_CENTER,
        &theme::author_initials(name),
        FontId::proportional(radius * 0.75),
        Color32::WHITE,
    );
}

// ── Horizontal separator line ─────────────────────────────────────────────────

pub fn h_line(ui: &Ui, y: f32, color: Color32) {
    let rect = ui.max_rect();
    ui.painter().line_segment(
        [Pos2::new(rect.min.x, y), Pos2::new(rect.max.x, y)],
        Stroke::new(1.0, color),
    );
}
