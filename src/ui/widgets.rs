use crate::ui::button::Button;
use egui::{Color32, FontId, Pos2, Rect, Response, RichText, Stroke, Ui, Vec2};

use crate::icons;
use crate::theme;

// ── Pill / badge ──────────────────────────────────────────────────────────────

/// Draw a fully-rounded pill badge at `rect`.
pub fn pill_rect(
    painter: &egui::Painter,
    rect: Rect,
    bg: Color32,
    text: &str,
    text_color: Color32,
    font_size: f32,
) {
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
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), font.clone(), text_color);
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
        'A' | '?' => theme::accent_green(),
        'D' | 'd' => theme::accent_red(),
        'R' => theme::accent_blue(),
        _ => theme::accent_yellow(), // M and others
    }
}

pub fn status_badge(ui: &mut Ui, ch: char) -> Response {
    let color = status_badge_color(ch);
    let label_ch = match ch {
        '?' => '?',
        'd' => 'D',
        _ => ch,
    };
    let size = Vec2::splat(18.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::hover());
    // A saturated tint behind the same dark foreground drops light-theme
    // 12-point badge labels below readable contrast, especially on selected rows.
    let background = if theme::is_dark() {
        color.gamma_multiply(0.25)
    } else {
        theme::bg_panel()
    };
    ui.painter().rect_filled(rect, 3.0, background);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label_ch.to_string(),
        FontId::monospace(12.0),
        color,
    );
    resp
}

// ── Section header (sidebar / panel) ─────────────────────────────────────────

/// Renders a collapsible section header. Returns true if the section is open.
pub fn section_header(ui: &mut Ui, title: &str, count: usize, open: &mut bool) -> Response {
    let row_h = 34.0;
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), row_h), egui::Sense::click());
    resp.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::CollapsingHeader, ui.is_enabled(), title)
    });
    if resp.hovered() || resp.has_focus() {
        ui.painter().rect_filled(rect, 4.0, theme::bg_hover());
    }
    let chevron = if *open {
        icons::EXPAND_MORE
    } else {
        icons::CHEVRON_RIGHT
    };
    ui.painter().text(
        Pos2::new(rect.left() + 10.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        chevron.to_string(),
        icons::font(16.0),
        theme::text_muted(),
    );
    ui.painter().text(
        Pos2::new(rect.left() + 30.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        title,
        FontId::proportional(theme::font_size::SECTION),
        theme::text_muted(),
    );
    if count > 0 {
        let text = count.to_string();
        let font = FontId::proportional(12.0);
        let galley = ui.painter().layout_no_wrap(text, font, theme::text_muted());
        let pill = Rect::from_min_size(
            Pos2::new(rect.right() - galley.size().x - 22.0, rect.center().y - 8.0),
            Vec2::new(galley.size().x + 14.0, 16.0),
        );
        ui.painter().rect_filled(pill, 8.0, theme::bg_elevated());
        ui.painter().galley(
            Pos2::new(
                pill.center().x - galley.size().x / 2.0,
                pill.center().y - galley.size().y / 2.0,
            ),
            galley,
            theme::text_muted(),
        );
    }

    if resp.clicked() {
        *open = !*open;
    }
    resp
}

// ── Toolbar button ────────────────────────────────────────────────────────────

pub fn toolbar_btn(ui: &mut Ui, label: &str, enabled: bool) -> Response {
    ui.add_enabled(
        enabled,
        Button::new(
            RichText::new(label)
                .color(if enabled {
                    theme::text_secondary()
                } else {
                    theme::text_muted()
                })
                .size(theme::font_size::BODY),
        )
        .fill(theme::bg_panel_alt())
        .stroke(Stroke::new(1.0, theme::border()))
        .corner_radius(6.0),
    )
}

/// Toolbar button with a Material Icon glyph followed by a text label.
pub fn toolbar_btn_icon(ui: &mut Ui, icon: char, label: &str, enabled: bool) -> Response {
    let col = if enabled {
        theme::text_secondary()
    } else {
        theme::text_muted()
    };
    toolbar_btn_icon_colored(
        ui,
        icon,
        label,
        col,
        enabled,
        Color32::TRANSPARENT,
        Color32::TRANSPARENT,
    )
}

pub fn toolbar_btn_icon_colored(
    ui: &mut Ui,
    icon: char,
    label: &str,
    color: Color32,
    enabled: bool,
    fill: Color32,
    stroke: Color32,
) -> Response {
    let col = if enabled && ui.is_enabled() {
        color
    } else {
        theme::text_muted()
    };
    let icon = ui
        .painter()
        .layout_no_wrap(icon.to_string(), icons::font(18.0), col);
    let text = ui
        .painter()
        .layout_no_wrap(label.to_owned(), FontId::proportional(13.0), col);
    let gap = 8.0;
    let padding = 12.0;
    let content_width = 18.0 + gap + text.size().x;
    let response = ui.add_enabled(
        enabled,
        Button::new("")
            .fill(fill)
            .stroke(Stroke::new(1.0, stroke))
            .corner_radius(7.0)
            .min_size(Vec2::new(content_width + padding * 2.0, 34.0)),
    );
    // Font line boxes have different ascenders/descenders. Centre the visible
    // glyphs independently, keeping native button focus, hover and activation.
    let left = response.rect.center().x - content_width / 2.0;
    paint_centered_glyphs(
        ui,
        &response,
        icon,
        Pos2::new(left + 9.0, response.rect.center().y),
        col,
    );
    let text_position = Pos2::new(
        left + 18.0 + gap,
        response.rect.center().y - text.mesh_bounds.center().y,
    );
    ui.painter()
        .with_clip_rect(response.rect)
        .galley(text_position, text, col);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled && ui.is_enabled(), label)
    });
    response
}

fn paint_centered_glyphs(
    ui: &Ui,
    response: &Response,
    galley: std::sync::Arc<egui::Galley>,
    center: Pos2,
    color: Color32,
) {
    let position = center - galley.mesh_bounds.center().to_vec2();
    ui.painter()
        .with_clip_rect(response.rect)
        .galley(position, galley, color);
}

/// Icon-only buttons use native egui button focus and keyboard activation.
pub fn icon_btn_mat(ui: &mut Ui, icon: char, size: f32, tip: &str) -> Response {
    icon_btn_mat_colored(ui, icon, size, tip, theme::text_secondary())
}

pub fn icon_btn_mat_colored(
    ui: &mut Ui,
    icon: char,
    size: f32,
    tip: &str,
    color: Color32,
) -> Response {
    let response = ui.add_sized(
        [32.0, 32.0],
        Button::new("")
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::NONE),
    );
    let color = if ui.is_enabled() {
        color
    } else {
        theme::text_muted()
    };
    let galley = ui
        .painter()
        .layout_no_wrap(icon.to_string(), icons::font(size), color);
    paint_centered_glyphs(ui, &response, galley, response.rect.center(), color);
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tip));
    response.on_hover_text(tip)
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
    let font = FontId::proportional(12.0);
    let galley = painter.layout_no_wrap(name.to_string(), font, theme::text_primary());
    let pad_x = 6.0;
    let pad_y = 2.0;
    let w = galley.size().x + pad_x * 2.0;
    let h = (galley.size().y + pad_y * 2.0).min(row_height - 6.0);
    let rect = Rect::from_min_size(
        Pos2::new(pos.x, row_top + (row_height - h) / 2.0),
        Vec2::new(w, h),
    );

    // colour by remote vs local vs tag
    let bg = theme::bg_selected();
    let border_col = if name.contains('/') {
        theme::accent_blue().gamma_multiply(0.5)
    } else {
        theme::accent_teal().gamma_multiply(0.6)
    };

    painter.rect_filled(rect, rect.height() / 2.0, bg);
    painter.rect_stroke(
        rect,
        rect.height() / 2.0,
        Stroke::new(0.8, border_col),
        egui::StrokeKind::Middle,
    );
    painter.galley(
        Pos2::new(rect.min.x + pad_x, rect.min.y + (h - galley.size().y) / 2.0),
        galley,
        theme::text_primary(),
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
        theme::contrast_text(color),
    );
}

/// A downloaded author picture when available, otherwise initials.
pub fn avatar_for(painter: &egui::Painter, center: Pos2, radius: f32, name: &str, email: &str) {
    match crate::ui::avatars::texture(painter.ctx(), email) {
        Some(texture) => {
            painter.image(
                texture,
                Rect::from_center_size(center, Vec2::splat(radius * 2.0)),
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        None => avatar(painter, center, radius, name),
    }
}

// ── Horizontal separator line ─────────────────────────────────────────────────

pub fn h_line(ui: &Ui, y: f32, color: Color32) {
    let rect = ui.max_rect();
    ui.painter().line_segment(
        [Pos2::new(rect.min.x, y), Pos2::new(rect.max.x, y)],
        Stroke::new(1.0, color),
    );
}
