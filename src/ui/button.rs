use egui::{Color32, Response, Rounding, Stroke, Ui, Vec2, Widget, WidgetText};

/// Native button interaction with state-aware custom colours. A fixed egui fill
/// or stroke overrides its hover visuals, so paint the surface separately.
pub struct Button<'a> {
    inner: egui::Button<'a>,
    fill: Option<Color32>,
    stroke: Option<Stroke>,
    rounding: Option<Rounding>,
}

impl<'a> Button<'a> {
    pub fn new(text: impl Into<WidgetText>) -> Self {
        Self {
            inner: egui::Button::new(text),
            fill: None,
            stroke: None,
            rounding: None,
        }
    }

    pub fn fill(mut self, fill: impl Into<Color32>) -> Self {
        self.fill = Some(fill.into());
        self
    }

    pub fn stroke(mut self, stroke: impl Into<Stroke>) -> Self {
        self.stroke = Some(stroke.into());
        self
    }

    pub fn rounding(mut self, rounding: impl Into<Rounding>) -> Self {
        let rounding = rounding.into();
        self.rounding = Some(rounding);
        self.inner = self.inner.rounding(rounding);
        self
    }

    pub fn min_size(mut self, size: Vec2) -> Self {
        self.inner = self.inner.min_size(size);
        self
    }

    pub fn truncate(mut self) -> Self {
        self.inner = self.inner.truncate();
        self
    }
}

impl Widget for Button<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        if self.fill.is_none() && self.stroke.is_none() {
            return self.inner.ui(ui);
        }
        // Reserve a background slot before native text, preserving native layout,
        // accessibility, focus, tooltips and click/keyboard activation.
        let background = ui.painter().add(egui::Shape::Noop);
        let response = self
            .inner
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::NONE)
            .ui(ui);
        if ui.is_rect_visible(response.rect) {
            let visuals = ui.style().interact(&response);
            let highlighted = response.enabled()
                && (response.hovered()
                    || response.has_focus()
                    || response.is_pointer_button_down_on());
            let fill = match self.fill {
                Some(fill) if highlighted && fill != Color32::TRANSPARENT => {
                    // Preserve primary/destructive colours and their label contrast.
                    let tint = if response.is_pointer_button_down_on() {
                        egui::Rgba::BLACK
                    } else {
                        egui::Rgba::WHITE
                    };
                    Color32::from(egui::Rgba::from(fill) * 0.9 + tint * 0.1)
                }
                Some(_) if highlighted => visuals.weak_bg_fill,
                Some(fill) => fill,
                None => visuals.weak_bg_fill,
            };
            let stroke = if highlighted {
                Stroke::new(1.5_f32, crate::theme::accent_blue())
            } else {
                self.stroke.unwrap_or(visuals.bg_stroke)
            };
            ui.painter().set(
                background,
                egui::epaint::RectShape::new(
                    response.rect,
                    self.rounding.unwrap_or(visuals.rounding),
                    fill,
                    stroke,
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
    fn custom_buttons_render_hover_and_focus_without_activating_disabled_controls() {
        for dark in [false, true] {
            for fill in [Color32::TRANSPARENT, Color32::from_rgb(49, 88, 198)] {
                let ctx = egui::Context::default();
                crate::theme::apply(&ctx, dark, false);
                let draw = |events: Vec<egui::Event>, enabled, focus| {
                    ctx.begin_frame(egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            Vec2::new(400.0, 200.0),
                        )),
                        events,
                        ..Default::default()
                    });
                    let mut response = None;
                    egui::CentralPanel::default().show(&ctx, |ui| {
                        let button = ui.add_enabled(
                            enabled,
                            Button::new("Action").fill(fill).stroke(Stroke::NONE),
                        );
                        if focus {
                            button.request_focus();
                        }
                        response = Some(button);
                    });
                    let response = response.unwrap();
                    let output = ctx.end_frame();
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
