use egui::{Context, RichText};

use crate::state::SharedState;
use crate::theme;

pub struct StatusBar {
    state: SharedState,
}

impl StatusBar {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    pub fn show(&self, ctx: &Context) {
        egui::TopBottomPanel::bottom("status_bar")
            .exact_height(22.0)
            .show(ctx, |ui| {
                ui.painter()
                    .rect_filled(ui.max_rect(), 0.0, theme::BG_ELEVATED);

                ui.horizontal_centered(|ui| {
                    ui.add_space(10.0);

                    let state = self.state.lock().unwrap();

                    // Branch name
                    let branch = state.head_branch.as_deref().unwrap_or("—");
                    ui.label(
                        RichText::new(format!("⎇  {}", branch))
                            .color(theme::ACCENT_GREEN)
                            .size(11.5),
                    );

                    // Ahead / behind
                    if let Some((ahead, behind)) = state.ahead_behind {
                        ui.add_space(10.0);
                        ui.label(
                            RichText::new(format!("↑{}", ahead))
                                .color(theme::TEXT_SECONDARY)
                                .size(11.0),
                        );
                        ui.label(
                            RichText::new(format!("↓{}", behind))
                                .color(theme::TEXT_SECONDARY)
                                .size(11.0),
                        );
                    }

                    // Dirty count
                    if state.dirty_count > 0 {
                        ui.add_space(10.0);
                        ui.label(
                            RichText::new(format!("● {} modified", state.dirty_count))
                                .color(theme::ACCENT_YELLOW)
                                .size(11.0),
                        );
                    } else if state.repo_path.is_some() {
                        ui.add_space(10.0);
                        ui.label(
                            RichText::new("✓ clean")
                                .color(theme::TEXT_MUTED)
                                .size(11.0),
                        );
                    }

                    // Error indicator
                    if let Some(ref err) = state.error {
                        ui.add_space(10.0);
                        ui.label(
                            RichText::new(format!("⚠ {}", err))
                                .color(theme::ACCENT_RED)
                                .size(11.0),
                        );
                    }
                });
            });
    }
}
