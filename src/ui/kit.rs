//! Layout kit: the shared building blocks for settings pages, panels, dialogs and
//! menus, so every screen uses one spacing scale, one card style and one button
//! hierarchy. Colours come from `theme`; transitions from `motion`.
use egui::{Align, Color32, FontId, Layout, Response, RichText, Sense, Stroke, Ui, Vec2};

use crate::icons;
use crate::theme;
use crate::ui::button::Button;
use crate::ui::motion;

/// Spacing scale in logical points. Use these instead of ad-hoc numbers.
pub mod space {
    pub const XS: f32 = 4.0;
    pub const SM: f32 = 8.0;
    pub const MD: f32 = 12.0;
    pub const LG: f32 = 16.0;
    pub const XL: f32 = 24.0;
}

/// Card corner radius and inner padding.
pub const CARD_RADIUS: u8 = 10;
pub const CARD_PADDING: i8 = 16;

// ── Text ─────────────────────────────────────────────────────────────────────

/// Page title at the top of a settings page or large panel.
pub fn page_header(ui: &mut Ui, title: &str, subtitle: &str) {
    ui.label(
        RichText::new(title)
            .size(theme::font_size::TITLE)
            .strong()
            .color(theme::text_primary()),
    );
    if !subtitle.is_empty() {
        ui.add_space(2.0);
        hint(ui, subtitle);
    }
    ui.add_space(space::LG);
}

/// Secondary explanatory text that wraps to the available width.
pub fn hint(ui: &mut Ui, text: &str) -> Response {
    ui.add(
        egui::Label::new(
            RichText::new(text)
                .size(theme::font_size::SMALL)
                .color(theme::text_muted()),
        )
        .wrap(),
    )
}

/// Small uppercase-free group label used above a run of related rows.
pub fn group_label(ui: &mut Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .size(theme::font_size::SECTION)
            .strong()
            .color(theme::text_secondary()),
    );
}

/// A path as people write it, without the `\\?\` verbatim prefix that
/// canonicalised Windows paths carry. For display and copying only.
pub fn display_path(path: &std::path::Path) -> String {
    let text = path.display().to_string();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        rest.to_owned()
    } else {
        text
    }
}

// ── Cards and rows ───────────────────────────────────────────────────────────

/// A titled card that groups related settings. `description` (may be empty)
/// explains what the group is for in one or two plain sentences.
pub fn section<R>(
    ui: &mut Ui,
    title: &str,
    description: &str,
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    let inner = card(ui, |ui| {
        ui.label(
            RichText::new(title)
                .size(15.0)
                .strong()
                .color(theme::text_primary()),
        );
        if !description.is_empty() {
            ui.add_space(2.0);
            hint(ui, description);
        }
        ui.add_space(space::MD);
        add(ui)
    });
    ui.add_space(space::MD);
    inner
}

/// The bare card surface used by [`section`]; full available width.
pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .fill(card_fill())
        .stroke(Stroke::new(1.0, theme::border()))
        .corner_radius(CARD_RADIUS)
        .inner_margin(CARD_PADDING)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// Card background: one step above the surrounding panel in every palette.
pub fn card_fill() -> Color32 {
    theme::bg_elevated()
}

/// A labelled setting: label and help on the left, the control on the right.
/// Stacks vertically when the card is narrow.
pub fn setting_row<R>(
    ui: &mut Ui,
    label: &str,
    help: &str,
    add_control: impl FnOnce(&mut Ui) -> R,
) -> R {
    let wide = ui.available_width() >= 520.0;
    let label_block = |ui: &mut Ui| {
        ui.label(
            RichText::new(label)
                .size(theme::font_size::BODY)
                .color(theme::text_primary()),
        );
        if !help.is_empty() {
            hint(ui, help);
        }
    };
    if wide {
        ui.horizontal_top(|ui| {
            let label_width = (ui.available_width() * 0.42).clamp(180.0, 300.0);
            ui.allocate_ui_with_layout(
                Vec2::new(label_width, 0.0),
                Layout::top_down(Align::Min),
                |ui| {
                    ui.set_width(label_width);
                    label_block(ui);
                },
            );
            ui.add_space(space::MD);
            ui.vertical(add_control).inner
        })
        .inner
    } else {
        ui.vertical(|ui| {
            label_block(ui);
            ui.add_space(space::XS);
            add_control(ui)
        })
        .inner
    }
}

/// Hairline between rows inside a card.
pub fn row_divider(ui: &mut Ui) {
    ui.add_space(space::SM);
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 1.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, 0.0, theme::border().gamma_multiply(0.7));
    ui.add_space(space::SM);
}

// ── Disclosure ───────────────────────────────────────────────────────────────

/// "Show advanced…" style disclosure with an animated chevron and body.
/// Openness is remembered in egui memory under `id_salt` within this `ui`.
pub fn disclosure<R>(
    ui: &mut Ui,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    label: &str,
    default_open: bool,
    add: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    let id = ui.make_persistent_id(id_salt);
    let open = motion::is_open(ui.ctx(), id, default_open);
    let openness = motion::openness(ui.ctx(), id, default_open);
    let response = ui
        .horizontal(|ui| {
            let (rect, response) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.0), Sense::click());
            if response.hovered() || response.has_focus() {
                ui.painter()
                    .rect_filled(rect, 6.0, theme::bg_hover().gamma_multiply(0.6));
            }
            let chevron_center = egui::pos2(rect.left() + 10.0, rect.center().y);
            paint_chevron(ui, chevron_center, openness, theme::text_muted());
            ui.painter().text(
                egui::pos2(rect.left() + 24.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                label,
                FontId::proportional(theme::font_size::BODY),
                theme::text_secondary(),
            );
            response
        })
        .inner;
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::CollapsingHeader,
            ui.is_enabled(),
            open,
            label,
        )
    });
    let response = response.on_hover_text(if open { "Hide" } else { "Show" });
    let open = if response.clicked() { !open } else { open };
    motion::set_open(ui.ctx(), id, open);
    motion::reveal(ui, id, open, |ui| {
        ui.add_space(space::XS);
        ui.indent(id.with("body"), add).inner
    })
}

/// Chevron that rotates from pointing right (closed) to down (open).
pub fn paint_chevron(ui: &Ui, center: egui::Pos2, openness: f32, color: Color32) {
    let angle = openness.clamp(0.0, 1.0) * std::f32::consts::FRAC_PI_2;
    let rotation = egui::emath::Rot2::from_angle(angle);
    let points = [
        egui::vec2(-2.5, -4.5),
        egui::vec2(2.5, 0.0),
        egui::vec2(-2.5, 4.5),
    ]
    .map(|offset| center + rotation * offset);
    ui.painter()
        .add(egui::Shape::line(points.to_vec(), Stroke::new(1.6, color)));
}

// ── Toggle ───────────────────────────────────────────────────────────────────

/// On/off switch for settings rows. The knob slides and the track colour eases
/// between states. `label` is the accessible name (the row shows it visually).
pub fn toggle(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    let size = Vec2::new(36.0, 20.0);
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let t = motion::progress(ui.ctx(), response.id.with("toggle"), *on, motion::FAST);
    let enabled = ui.is_enabled();
    let off_track = theme::bg_app();
    let on_track = theme::primary_fill();
    let mut track = motion::mix(off_track, on_track, t);
    if !enabled {
        track = track.gamma_multiply(0.5);
    }
    let radius = rect.height() / 2.0;
    ui.painter().rect_filled(rect, radius, track);
    let outline = motion::mix(theme::border_strong(), on_track, t);
    ui.painter().rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, outline),
        egui::StrokeKind::Inside,
    );
    let hover = motion::hover(ui, &response);
    let knob_x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), t);
    let knob = motion::mix(theme::text_secondary(), theme::on_primary(), t);
    ui.painter().circle_filled(
        egui::pos2(knob_x, rect.center().y),
        radius - 3.0 + hover * 0.6,
        knob,
    );
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.expand(2.0),
            radius + 2.0,
            Stroke::new(1.5, theme::accent_teal()),
            egui::StrokeKind::Outside,
        );
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled, *on, label)
    });
    response
}

// ── Navigation ───────────────────────────────────────────────────────────────

/// Full-width navigation entry (settings categories, panel lists). Animates its
/// hover and selection fill and shows an accent bar when selected.
pub fn nav_item(ui: &mut Ui, icon: char, label: &str, selected: bool) -> Response {
    let height = 34.0;
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::click());
    let hover = motion::hover(ui, &response);
    let selection = motion::progress(
        ui.ctx(),
        response.id.with("selected"),
        selected,
        motion::FAST,
    );
    let base = Color32::TRANSPARENT;
    let fill = motion::mix(
        motion::mix(base, theme::bg_hover(), hover),
        theme::bg_selected(),
        selection,
    );
    ui.painter().rect_filled(rect, 7.0, fill);
    if selection > 0.0 {
        let bar = egui::Rect::from_min_size(
            egui::pos2(rect.left(), rect.top() + 8.0),
            Vec2::new(3.0, (height - 16.0) * selection),
        );
        ui.painter()
            .rect_filled(bar, 2.0, theme::primary_fill().gamma_multiply(selection));
    }
    let text_color = motion::mix(theme::text_secondary(), theme::text_primary(), selection);
    ui.painter().text(
        egui::pos2(rect.left() + 14.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        icon.to_string(),
        icons::font(18.0),
        text_color,
    );
    ui.painter().text(
        egui::pos2(rect.left() + 42.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        FontId::proportional(theme::font_size::BODY),
        text_color,
    );
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.shrink(1.0),
            7.0,
            Stroke::new(1.5, theme::accent_teal()),
            egui::StrokeKind::Inside,
        );
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, label)
    });
    response
}

// ── Status ───────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ok,
    Warning,
    Error,
    Info,
    Neutral,
}

impl Status {
    pub fn color(self) -> Color32 {
        match self {
            Self::Ok => theme::accent_green(),
            Self::Warning => theme::accent_yellow(),
            Self::Error => theme::accent_red(),
            Self::Info => theme::accent_blue(),
            Self::Neutral => theme::text_muted(),
        }
    }
}

/// Compact status pill: coloured dot plus text (never colour alone).
pub fn status_chip(ui: &mut Ui, status: Status, text: &str) -> Response {
    let color = status.color();
    let font = FontId::proportional(theme::font_size::SMALL);
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font, theme::text_primary());
    let size = Vec2::new(galley.size().x + 26.0, 22.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter()
        .rect_filled(rect, rect.height() / 2.0, color.gamma_multiply(0.16));
    ui.painter().rect_stroke(
        rect,
        rect.height() / 2.0,
        Stroke::new(1.0, color.gamma_multiply(0.55)),
        egui::StrokeKind::Inside,
    );
    ui.painter()
        .circle_filled(egui::pos2(rect.left() + 11.0, rect.center().y), 3.5, color);
    ui.painter().galley(
        egui::pos2(rect.left() + 19.0, rect.center().y - galley.size().y / 2.0),
        galley,
        theme::text_primary(),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text));
    response
}

/// Inline notice box (info, warning, error) with wrapped text.
pub fn callout(ui: &mut Ui, status: Status, text: &str) -> Response {
    let color = status.color();
    egui::Frame::new()
        .fill(color.gamma_multiply(0.10))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.45)))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(
                egui::Label::new(
                    RichText::new(text)
                        .size(theme::font_size::SMALL + 0.5)
                        .color(theme::text_primary()),
                )
                .wrap(),
            );
        })
        .response
}

// ── Empty states ─────────────────────────────────────────────────────────────

/// Centred empty state: icon, title, one-line explanation and an optional
/// action. Returns true when the action was clicked.
pub fn empty_state(ui: &mut Ui, icon: char, title: &str, body: &str, action: Option<&str>) -> bool {
    let mut clicked = false;
    ui.vertical_centered(|ui| {
        ui.add_space(space::LG);
        ui.label(
            RichText::new(icon.to_string())
                .font(icons::font(28.0))
                .color(theme::text_muted()),
        );
        ui.add_space(space::XS);
        ui.label(
            RichText::new(title)
                .size(theme::font_size::BODY)
                .strong()
                .color(theme::text_secondary()),
        );
        if !body.is_empty() {
            ui.add(
                egui::Label::new(
                    RichText::new(body)
                        .size(theme::font_size::SMALL)
                        .color(theme::text_muted()),
                )
                .wrap(),
            );
        }
        if let Some(action) = action {
            ui.add_space(space::SM);
            clicked = ui.add(secondary_button(action)).clicked();
        }
        ui.add_space(space::LG);
    });
    clicked
}

// ── Text fields ──────────────────────────────────────────────────────────────

/// Inner margin shared by every single-line field. egui's default (4 by 2) made
/// fields about 22 pixels tall with text touching the border.
pub const TEXT_FIELD_MARGIN: egui::Vec2 = egui::Vec2::new(8.0, 5.0);

/// Height of a compact field in dense panels (sidebar filter, commit summary,
/// history search), where neighbouring controls are icon buttons.
pub const COMPACT_FIELD_HEIGHT: f32 = 28.0;

/// A single-line text field as tall as a standard button (the theme's
/// interaction height), with its text centred vertically, so fields beside Save
/// or Browse buttons in Settings and dialogs line up with them. Callers may
/// still override the margin, font, width or minimum size.
pub fn text_field(text: &mut dyn egui::TextBuffer) -> egui::TextEdit<'_> {
    egui::TextEdit::singleline(text)
        .margin(TEXT_FIELD_MARGIN)
        .min_size(Vec2::new(0.0, theme::CONTROL_HEIGHT))
        .vertical_align(Align::Center)
}

/// A shorter single-line field for dense panels; see [`COMPACT_FIELD_HEIGHT`].
pub fn compact_text_field(text: &mut dyn egui::TextBuffer) -> egui::TextEdit<'_> {
    text_field(text).min_size(Vec2::new(0.0, COMPACT_FIELD_HEIGHT))
}

/// A checkbox for menus: no box, so unchecked rows show nothing and checked rows
/// show only the tick. With the theme's rounded corners egui's box rendered as a
/// filled dot beside unchecked items. Keeps the checkbox role for accessibility.
pub fn menu_check(ui: &mut Ui, value: &mut bool, label: &str) -> Response {
    ui.scope(|ui| {
        let widgets = &mut ui.visuals_mut().widgets;
        for state in [
            &mut widgets.inactive,
            &mut widgets.hovered,
            &mut widgets.active,
        ] {
            state.bg_fill = Color32::TRANSPARENT;
            state.bg_stroke = Stroke::NONE;
        }
        ui.checkbox(value, label)
    })
    .inner
}

/// Radius of the selected dot inside a [`radio`], as a fraction of the icon.
const RADIO_DOT: f32 = 0.27;

/// A radio button whose state is obvious: every option is a clear outlined ring
/// and the selected one gets an accent ring and dot. egui's default fills each
/// ring with the app background behind a faint border and marks the selected
/// option with a dot of about 3 pixels, so options looked alike. Keeps egui's
/// layout, keyboard handling and radio-button accessibility role.
pub fn radio(ui: &mut Ui, selected: bool, label: impl Into<egui::WidgetText>) -> Response {
    let response = ui
        .scope(|ui| {
            let widgets = &mut ui.visuals_mut().widgets;
            for (state, ring) in [
                (&mut widgets.inactive, theme::border_strong()),
                (&mut widgets.hovered, theme::accent_blue()),
                (&mut widgets.active, theme::accent_blue()),
            ] {
                state.bg_fill = Color32::TRANSPARENT;
                state.bg_stroke = Stroke::new(1.5, ring);
                state.expansion = 0.0;
            }
            ui.radio(selected, label)
        })
        .inner;
    if selected && ui.is_rect_visible(response.rect) {
        let icon = ui.spacing().icon_width;
        let center = egui::pos2(response.rect.left() + icon / 2.0, response.rect.center().y);
        let accent = if response.enabled() {
            theme::accent_blue()
        } else {
            theme::accent_blue().gamma_multiply(0.45)
        };
        let painter = ui.painter();
        painter.circle_stroke(center, icon / 2.0, Stroke::new(1.5, accent));
        painter.circle_filled(center, icon * RADIO_DOT, accent);
    }
    response
}

/// [`radio`] bound to a value, like `Ui::radio_value`.
pub fn radio_value<T: PartialEq>(
    ui: &mut Ui,
    current: &mut T,
    value: T,
    label: impl Into<egui::WidgetText>,
) -> Response {
    let mut response = radio(ui, *current == value, label);
    if response.clicked() && *current != value {
        *current = value;
        response.mark_changed();
    }
    response
}

/// A slider whose rail stays visible on light cards (egui draws the rail in the
/// inactive widget fill, which the palettes set to the app background) and whose
/// travelled part uses the interactive accent.
pub fn slider(ui: &mut Ui, slider: egui::Slider<'_>) -> Response {
    ui.scope(|ui| {
        let visuals = ui.visuals_mut();
        visuals.widgets.inactive.bg_fill = theme::border_strong();
        visuals.selection.bg_fill = theme::accent_blue();
        ui.add(slider.trailing_fill(true))
    })
    .inner
}

// ── Buttons ──────────────────────────────────────────────────────────────────

/// The one main action in a region (Commit, Save, Sign in).
pub fn primary_button(label: &str) -> Button<'_> {
    Button::new(RichText::new(label).color(theme::on_primary()).size(14.0))
        .disabled_text(
            RichText::new(label)
                .color(theme::text_secondary())
                .size(14.0),
        )
        .fill(theme::primary_fill())
        .corner_radius(7.0)
        .min_size(Vec2::new(96.0, 30.0))
}

/// Neutral actions (Cancel, Copy, Open…).
pub fn secondary_button(label: &str) -> Button<'_> {
    Button::new(RichText::new(label).color(theme::text_secondary()))
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::new(1.0, theme::border()))
        .corner_radius(7.0)
        .min_size(Vec2::new(0.0, 30.0))
}

/// Destructive actions; pair with a confirmation naming the exact target.
pub fn danger_button(label: &str) -> Button<'_> {
    Button::new(RichText::new(label).color(theme::contrast_text(theme::accent_red())))
        .fill(theme::accent_red())
        .corner_radius(7.0)
        .min_size(Vec2::new(0.0, 30.0))
}

/// Quiet destructive action (outline) for lists where a filled red button
/// would dominate, e.g. "Remove" on an account row.
pub fn danger_outline_button(label: &str) -> Button<'_> {
    Button::new(RichText::new(label).color(theme::accent_red()))
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::new(1.0, theme::accent_red().gamma_multiply(0.6)))
        .corner_radius(7.0)
        .min_size(Vec2::new(0.0, 30.0))
}

/// Compact button for rows and section headers.
pub fn small_button(label: &str) -> Button<'_> {
    Button::new(
        RichText::new(label)
            .size(12.0)
            .color(theme::text_secondary()),
    )
    .fill(Color32::TRANSPARENT)
    .stroke(Stroke::new(1.0, theme::border()))
    .corner_radius(6.0)
    .min_size(Vec2::new(0.0, 24.0))
}

/// Icon-only button with a required accessible label and tooltip.
pub fn icon_button(ui: &mut Ui, icon: char, label: &str) -> Response {
    crate::ui::widgets::icon_btn_mat(ui, icon, 18.0, label)
}

/// Adds `reason` as the tooltip of a disabled control (egui shows
/// `on_disabled_hover_text` only while disabled).
pub fn explain_disabled(response: Response, reason: &str) -> Response {
    if reason.is_empty() {
        response
    } else {
        response.on_disabled_hover_text(reason)
    }
}

// ── Menus ────────────────────────────────────────────────────────────────────

/// A menu entry with right-aligned shortcut text and a tooltip. A disabled
/// entry stays visible and explains why it is unavailable.
/// Minimum space between a menu label and its shortcut.
const MENU_SHORTCUT_GAP: f32 = 28.0;

pub fn menu_item(
    ui: &mut Ui,
    label: &str,
    shortcut: Option<&str>,
    enabled: bool,
    tooltip: &str,
) -> Response {
    let mut button = egui::Button::new(label);
    let has_shortcut = shortcut.is_some_and(|s| !s.is_empty());
    if let Some(shortcut) = shortcut.filter(|s| !s.is_empty()) {
        button = button.shortcut_text(RichText::new(shortcut).color(theme::text_muted()));
    }
    // egui separates the label and shortcut by twice the icon spacing, so in the
    // widest row of a menu the shortcut ran into the label and the menu edge.
    let response = ui
        .scope(|ui| {
            if has_shortcut {
                ui.spacing_mut().icon_spacing = MENU_SHORTCUT_GAP / 2.0;
            }
            ui.add_enabled(enabled, button)
        })
        .inner;
    if has_shortcut {
        // egui joins the shortcut onto the accessible name ("Clone Repository…
        // Ctrl+Shift+O"). Announce the command by its label alone, as native
        // menus do, so assistive technology and automation find it by name.
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    }
    if tooltip.is_empty() {
        response
    } else if enabled {
        response.on_hover_text(tooltip)
    } else {
        response.on_disabled_hover_text(tooltip)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_path_drops_only_the_verbatim_prefix() {
        use std::path::Path;
        assert_eq!(display_path(Path::new(r"\\?\F:\repo")), r"F:\repo");
        assert_eq!(
            display_path(Path::new(r"\\?\UNC\server\share\repo")),
            r"\\server\share\repo"
        );
        assert_eq!(display_path(Path::new("/home/me/repo")), "/home/me/repo");
    }

    fn frame(width: f32, add: impl FnOnce(&mut Ui)) {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::icons::font_definitions());
        ctx.begin_pass(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(width, 600.0),
            )),
            ..Default::default()
        });
        let mut ui = Ui::new(
            ctx.clone(),
            egui::Id::new("kit_test"),
            egui::UiBuilder::new().max_rect(ctx.content_rect()),
        );
        add(&mut ui);
        ctx.end_pass().textures_delta.clear();
    }

    #[test]
    fn text_fields_match_the_button_row_height() {
        let ctx = egui::Context::default();
        crate::theme::apply_palette(&ctx, true, false, crate::theme::Palette::ALL[0]);
        // Fonts and style apply from the next pass, so lay out twice.
        for pass in 0..2 {
            ctx.begin_pass(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(500.0, 200.0),
                )),
                ..Default::default()
            });
            let mut ui = egui::Ui::new(
                ctx.clone(),
                egui::Id::new("field_height"),
                egui::UiBuilder::new().max_rect(ctx.content_rect()),
            );
            ui.horizontal(|ui| {
                let mut text = String::new();
                let mut compact = String::new();
                let field = ui.add(text_field(&mut text).hint_text("Editor"));
                let save = ui.add(small_button("Save"));
                let primary = ui.add(primary_button("Save identity"));
                let dense = ui.add(compact_text_field(&mut compact));
                if pass == 1 {
                    for button in [save, primary] {
                        assert!(
                            (field.rect.height() - button.rect.height()).abs() <= 0.5,
                            "Field {} must match button {}",
                            field.rect.height(),
                            button.rect.height()
                        );
                    }
                    assert!((dense.rect.height() - COMPACT_FIELD_HEIGHT).abs() <= 0.5);
                }
            });
            ctx.end_pass().textures_delta.clear();
        }
    }

    #[test]
    fn selected_radio_draws_an_accent_dot_on_the_native_ring() {
        let ctx = egui::Context::default();
        crate::theme::apply_palette(&ctx, true, false, crate::theme::Palette::ALL[0]);
        for pass in 0..2 {
            ctx.begin_pass(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(400.0, 200.0),
                )),
                ..Default::default()
            });
            let mut ui = egui::Ui::new(
                ctx.clone(),
                egui::Id::new("radio"),
                egui::UiBuilder::new().max_rect(ctx.content_rect()),
            );
            let mut choice = 1;
            radio_value(&mut ui, &mut choice, 1, "GitHub.com");
            radio_value(&mut ui, &mut choice, 2, "GitLab.com");
            let icon = ui.spacing().icon_width;
            let mut output = ctx.end_pass();
            output.textures_delta.clear();
            if pass == 0 {
                continue;
            }
            let circles: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Circle(circle) => Some(*circle),
                    _ => None,
                })
                .collect();
            let rings: Vec<_> = circles
                .iter()
                .filter(|c| (c.radius - icon / 2.0).abs() < 0.01 && c.fill == Color32::TRANSPARENT)
                .collect();
            assert!(rings.len() >= 2, "Every option needs an outlined ring");
            let dots: Vec<_> = circles
                .iter()
                .filter(|c| c.fill == theme::accent_blue() && c.radius > 3.0)
                .collect();
            assert_eq!(
                dots.len(),
                1,
                "Only the selected option gets the accent dot"
            );
            assert!(
                rings
                    .iter()
                    .any(|ring| ring.center.distance(dots[0].center) < 0.01),
                "The accent dot must sit on the native ring"
            );
        }
    }

    #[test]
    fn cards_and_rows_stay_within_narrow_and_wide_widths() {
        for width in [320.0, 900.0] {
            frame(width, |ui| {
                let before = ui.max_rect();
                section(ui, "Title", "A description that wraps.", |ui| {
                    setting_row(ui, "Label", "Help text that wraps.", |ui| {
                        ui.label("control");
                    });
                    row_divider(ui);
                    status_chip(ui, Status::Ok, "Connected");
                    callout(ui, Status::Warning, "Careful.");
                });
                assert!(ui.min_rect().width() <= before.width() + 0.5);
            });
        }
    }

    #[test]
    fn nav_items_expose_selection_to_accessibility() {
        frame(240.0, |ui| {
            let selected = nav_item(ui, crate::icons::SETTINGS, "General", true);
            let other = nav_item(ui, crate::icons::SETTINGS, "Keyboard", false);
            assert!(selected.rect.bottom() <= other.rect.top() + 0.5);
        });
    }
}
