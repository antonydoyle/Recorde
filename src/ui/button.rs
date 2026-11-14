use egui::{Color32, CornerRadius, Response, Stroke, Ui, Vec2, Widget, WidgetText};

/// Native button interaction with state-aware custom colours. A fixed egui fill
/// or stroke overrides its hover visuals, so paint the surface separately.
pub struct Button<'a> {
    text: WidgetText,
    /// Label and neutral surface used while disabled, for buttons whose enabled
    /// colours (white on a strong fill) become unreadable once egui fades them.
    disabled_text: Option<WidgetText>,
    min_size: Option<Vec2>,
    truncate: bool,
    fill: Option<Color32>,
    stroke: Option<Stroke>,
    corner_radius: Option<CornerRadius>,
    _lifetime: std::marker::PhantomData<&'a ()>,
}

impl<'a> Button<'a> {
    pub fn new(text: impl Into<WidgetText>) -> Self {
        Self {
            text: text.into(),
            disabled_text: None,
            min_size: None,
            truncate: false,
            fill: None,
            stroke: None,
            corner_radius: None,
            _lifetime: std::marker::PhantomData,
        }
    }

    /// While disabled, show this label on a neutral surface instead of the fill.
    pub fn disabled_text(mut self, text: impl Into<WidgetText>) -> Self {
        self.disabled_text = Some(text.into());
        self
    }

    fn native(&self, enabled: bool) -> egui::Button<'a> {
        let text = match (&self.disabled_text, enabled) {
            (Some(text), false) => text.clone(),
            _ => self.text.clone(),
        };
        let mut button = egui::Button::new(text);
        if let Some(size) = self.min_size {
            button = button.min_size(size);
        }
        if self.truncate {
            button = button.truncate();
        }
        if let Some(radius) = self.corner_radius {
            button = button.corner_radius(radius);
        }
        button
    }

    pub fn fill(mut self, fill: impl Into<Color32>) -> Self {
        self.fill = Some(fill.into());
        self
    }

    pub fn stroke(mut self, stroke: impl Into<Stroke>) -> Self {
        self.stroke = Some(stroke.into());
        self
    }

    pub fn corner_radius(mut self, corner_radius: impl Into<CornerRadius>) -> Self {
        self.corner_radius = Some(corner_radius.into());
        self
    }

    pub fn min_size(mut self, size: Vec2) -> Self {
        self.min_size = Some(size);
        self
    }

    pub fn truncate(mut self) -> Self {
        self.truncate = true;
        self
    }
}

impl Widget for Button<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let enabled = ui.is_enabled();
        if self.fill.is_none() && self.stroke.is_none() {
            return self.native(enabled).ui(ui);
        }
        let neutral_disabled = !enabled && self.disabled_text.is_some();
        // Reserve a background slot before native text, preserving native layout,
        // accessibility, focus, tooltips and click/keyboard activation.
        let background = ui.painter().add(egui::Shape::Noop);
        // egui sizes a button's padding from the current state's stroke width and
        // expansion, so a hovered or pressed button can shrink and shift its
        // neighbours. This widget paints its own surface, so lay the native button
        // out with the idle state's metrics in every state.
        let response = ui
            .scope(|ui| {
                let widgets = &mut ui.visuals_mut().widgets;
                let idle = widgets.inactive;
                for state in [&mut widgets.hovered, &mut widgets.active, &mut widgets.open] {
                    state.bg_stroke.width = idle.bg_stroke.width;
                    state.expansion = idle.expansion;
                }
                self.native(enabled)
                    .fill(Color32::TRANSPARENT)
                    .stroke(Stroke::NONE)
                    .ui(ui)
            })
            .inner;
        if ui.is_rect_visible(response.rect) {
            let visuals = ui.style().interact(&response);
            let highlighted = response.enabled()
                && (response.hovered()
                    || response.has_focus()
                    || response.is_pointer_button_down_on());
            let fill = match self.fill {
                _ if neutral_disabled => crate::theme::bg_panel_alt(),
                Some(fill) if highlighted && fill != Color32::TRANSPARENT => {
                    // Preserve primary/destructive colours and their label contrast:
                    // move the fill away from its label (darker under a light label,
                    // lighter under a dark one, as on Monokai's or Nord's buttons).
                    let label = if fill == crate::theme::primary_fill() {
                        crate::theme::on_primary()
                    } else {
                        crate::theme::contrast_text(fill)
                    };
                    let tint = if crate::theme::contrast_text(label) == Color32::BLACK {
                        egui::Rgba::BLACK
                    } else {
                        egui::Rgba::WHITE
                    };
                    let amount = if response.is_pointer_button_down_on() {
                        0.18
                    } else {
                        0.1
                    };
                    Color32::from(egui::Rgba::from(fill) * (1.0 - amount) + tint * amount)
                }
                Some(_) if highlighted => visuals.weak_bg_fill,
                Some(fill) => fill,
                None => visuals.weak_bg_fill,
            };
            let stroke = if neutral_disabled {
                Stroke::new(1.0, crate::theme::border_strong())
            } else if highlighted {
                Stroke::new(1.5_f32, crate::theme::accent_blue())
            } else {
                self.stroke.unwrap_or(visuals.bg_stroke)
            };
            ui.painter().set(
                background,
                egui::epaint::RectShape::new(
                    response.rect,
                    self.corner_radius.unwrap_or(visuals.corner_radius),
                    fill,
                    stroke,
                    egui::StrokeKind::Middle,
                ),
            );
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_primary_buttons_use_a_readable_neutral_surface() {
        for palette in crate::theme::Palette::ALL {
            for dark in [false, true] {
                let ctx = egui::Context::default();
                crate::theme::apply_palette(&ctx, dark, false, palette);
                ctx.begin_pass(egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(400.0, 200.0),
                    )),
                    ..Default::default()
                });
                let mut ui = egui::Ui::new(
                    ctx.clone(),
                    egui::Id::new("disabled_primary"),
                    egui::UiBuilder::new().max_rect(ctx.content_rect()),
                );
                let response = ui.add_enabled(false, crate::ui::kit::primary_button("Commit"));
                let mut output = ctx.end_pass();
                output.textures_delta.clear();
                let surface = output.shapes.iter().find_map(|shape| match &shape.shape {
                    egui::Shape::Rect(rect) if rect.rect == response.rect => Some(rect.fill),
                    _ => None,
                });
                // Disabled painting fades colours, so compare unmultiplied channels.
                let [r, g, b, _] = surface.expect("Button surface").to_srgba_unmultiplied();
                let expected = crate::theme::bg_panel_alt();
                for (actual, wanted) in [(r, expected.r()), (g, expected.g()), (b, expected.b())] {
                    assert!(
                        actual.abs_diff(wanted) <= 3,
                        "{palette:?} dark={dark}: disabled primary fill {r},{g},{b} is not neutral"
                    );
                }
            }
        }
    }

    #[test]
    fn custom_buttons_render_hover_and_focus_without_activating_disabled_controls() {
        for (palette, dark) in crate::theme::Palette::ALL
            .into_iter()
            .flat_map(|palette| [false, true].map(|dark| (palette, dark)))
        {
            let primary = if palette == crate::theme::Palette::Siege {
                crate::theme::SIEGE_ORANGE
            } else {
                Color32::from_rgb(49, 88, 198)
            };
            for fill in [Color32::TRANSPARENT, primary] {
                let ctx = egui::Context::default();
                crate::theme::apply_palette(&ctx, dark, false, palette);
                let draw = |events: Vec<egui::Event>, enabled, focus| {
                    ctx.begin_pass(egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            Vec2::new(400.0, 200.0),
                        )),
                        events,
                        ..Default::default()
                    });
                    let mut response = None;
                    egui::CentralPanel::default().show(
                        &mut egui::Ui::new(
                            ctx.clone(),
                            egui::Id::new("test_root"),
                            egui::UiBuilder::new().max_rect(ctx.content_rect()),
                        ),
                        |ui| {
                            let button = ui.add_enabled(
                                enabled,
                                Button::new("Action").fill(fill).stroke(Stroke::NONE),
                            );
                            if focus {
                                button.request_focus();
                            }
                            response = Some(button);
                        },
                    );
                    let response = response.unwrap();
                    let mut output = ctx.end_pass();
                    output.textures_delta.clear();
                    let surface = output.shapes.iter().find_map(|shape| match &shape.shape {
                        egui::Shape::Rect(rect)
                            if rect.rect == response.rect && rect.fill != Color32::TRANSPARENT =>
                        {
                            Some((rect.fill, rect.stroke))
                        }
                        _ => None,
                    });
                    (response, surface)
                };
                let (idle, normal) = draw(vec![], true, false);
                let pointer = idle.rect.center();
                let (_, hover) = draw(vec![egui::Event::PointerMoved(pointer)], true, false);
                assert_ne!(normal, hover, "Custom surface must change on hover");
                if fill != Color32::TRANSPARENT {
                    let hover_fill = hover.unwrap().0;
                    assert!(
                        hover_fill.r() <= fill.r()
                            && hover_fill.g() <= fill.g()
                            && hover_fill.b() <= fill.b(),
                        "White primary labels must retain contrast on hover"
                    );
                }
                draw(vec![egui::Event::PointerGone], true, true);
                let (_, focused) = draw(vec![], true, false);
                assert_ne!(normal, focused, "Keyboard focus must be visible");
                ctx.memory_mut(|memory| memory.surrender_focus(idle.id));
                let (_, disabled) = draw(vec![egui::Event::PointerGone], false, false);
                let (response, disabled_hover) =
                    draw(vec![egui::Event::PointerMoved(pointer)], false, false);
                assert_eq!(disabled, disabled_hover);
                assert!(!response.hovered());
                let click = |pressed| egui::Event::PointerButton {
                    pos: pointer,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                };
                draw(vec![click(true)], false, false);
                assert!(!draw(vec![click(false)], false, false).0.clicked());
                // egui hit-testing uses the previous frame's enabled widget list.
                draw(vec![], true, false);
                draw(vec![click(true)], true, false);
                assert!(draw(vec![click(false)], true, false).0.clicked());
            }
        }
    }
}
