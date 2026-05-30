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
                        ui.add_space(8.0);
                        if ahead > 0 {
                            ui.label(
                                RichText::new(format!("↑{}", ahead))
                                    .color(theme::ACCENT_BLUE)
                                    .size(11.0),
                            );
                        }
                        if behind > 0 {
                            ui.label(
                                RichText::new(format!("↓{}", behind))
                                    .color(theme::ACCENT_YELLOW)
                                    .size(11.0),
                            );
                        }
                        if ahead == 0 && behind == 0 {
                            ui.label(
                                RichText::new("↕ up to date")
                                    .color(theme::TEXT_MUTED)
                                    .size(11.0),
                            );
                        }
                    }

                    // Dirty count
                    if state.dirty_count > 0 {
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new(format!("● {} modified", state.dirty_count))
                                .color(theme::ACCENT_YELLOW)
                                .size(11.0),
                        );
                    } else if state.repo_path.is_some() && state.ahead_behind.is_none() {
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new("✓ clean")
                                .color(theme::TEXT_MUTED)
                                .size(11.0),
                        );
                    }

                    // Remote operation progress (right-aligned)
                    let remote_op = state.remote_op.clone();
                    let error = state.error.clone();
                    drop(state);

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(10.0);
                        if let Some(ref op) = remote_op {
                            ui.label(
                                RichText::new(op).color(theme::TEXT_MUTED).size(11.0),
                            );
                        } else if let Some(ref err) = error {
                            ui.label(
                                RichText::new(format!("⚠ {}", err))
                                    .color(theme::ACCENT_RED)
                                    .size(11.0),
                            );
                        }
                    });
                });
            });
    }
}
