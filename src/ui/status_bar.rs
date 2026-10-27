use egui::{RichText, Stroke};

use crate::icons;
use crate::state::SharedState;
use crate::theme;

pub struct StatusBar {
    state: SharedState,
}

impl StatusBar {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    pub fn show(&self, root_ui: &mut egui::Ui) {
        egui::Panel::bottom("status_bar")
            .exact_size(28.0)
            .frame(egui::Frame::NONE.fill(theme::bg_titlebar()))
            .show(root_ui, |ui| {
                let r = ui.max_rect();
                ui.painter().line_segment(
                    [r.left_top(), r.right_top()],
                    Stroke::new(1.0, theme::border()),
                );

                ui.horizontal_centered(|ui| {
                    ui.add_space(12.0);

                    let s = self.state.lock().unwrap();

                    // ── Left: version + repo ───────────────────────────
                    ui.label(
                        RichText::new(concat!("Recorde v", env!("CARGO_PKG_VERSION")))
                            .color(theme::text_muted())
                            .size(12.0),
                    );

                    if let Some(ref p) = s.repo_path {
                        let name = p
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        ui.add_space(8.0);
                        ui.add(egui::Separator::default().vertical().spacing(2.0));
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new(&name)
                                .color(theme::text_secondary())
                                .size(12.0),
                        );
                    }

                    // ── Right ─────────────────────────────────────────
                    let branch = s.head_branch.clone().unwrap_or_else(|| "—".into());
                    let ab = s.ahead_behind;
                    let dirty = s.dirty_count;
                    let remote_op = s.remote_op.clone();
                    let fetching = s
                        .active_ops
                        .iter()
                        .any(|op| op.background && s.repo_path.as_ref() == Some(&op.repo_path));
                    let fetch_error = s
                        .background_fetch
                        .as_ref()
                        .filter(|status| s.repo_path.as_ref() == Some(&status.repo_path))
                        .map(|status| crate::auth::redact::text(&status.message));
                    let error = s
                        .error
                        .as_deref()
                        .or_else(|| s.errors.front().map(|error| error.message.as_str()))
                        .map(crate::auth::redact::text);
                    drop(s);

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(12.0);

                        if let Some(ref e) = error {
                            ui.label(
                                RichText::new(format!("⚠ {}", e))
                                    .color(theme::accent_red())
                                    .size(12.0),
                            );
                            ui.add_space(8.0);
                        } else if let Some(ref op) = remote_op {
                            ui.label(RichText::new(op).color(theme::text_muted()).size(12.0));
                            ui.add_space(8.0);
                        }

                        if fetching {
                            ui.label(
                                RichText::new("Fetching…")
                                    .color(theme::text_muted())
                                    .size(12.0),
                            );
                            ui.add_space(8.0);
                        } else if let Some(message) = &fetch_error {
                            ui.label(
                                RichText::new("Last fetch failed")
                                    .color(theme::accent_yellow())
                                    .size(12.0),
                            )
                            .on_hover_text(message);
                            ui.add_space(8.0);
                        }

                        if dirty > 0 {
                            ui.label(
                                RichText::new(format!("● {}", dirty))
                                    .color(theme::accent_yellow())
                                    .size(12.0),
                            );
                            ui.add_space(8.0);
                        } else if error.is_none()
                            && remote_op.is_none()
                            && !fetching
                            && fetch_error.is_none()
                        {
                            ui.label(RichText::new("✓").color(theme::accent_green()).size(12.0));
                            ui.add_space(8.0);
                        }

                        match ab {
                            Some((a, b)) if a > 0 && b > 0 => {
                                // Diverged — both ahead and behind: amber warning
                                let badge = egui::RichText::new(format!("↑{} ↓{}", a, b))
                                    .color(theme::accent_orange())
                                    .size(12.0);
                                ui.label(badge).on_hover_text(
                                    "Branch has diverged from upstream — pull with rebase or merge",
                                );
                                ui.add_space(4.0);
                            }
                            Some((a, b)) => {
                                if b > 0 {
                                    ui.label(
                                        RichText::new(format!("↓{}", b))
                                            .color(theme::accent_yellow())
                                            .size(12.0),
                                    );
                                }
                                if a > 0 {
                                    ui.label(
                                        RichText::new(format!("↑{}", a))
                                            .color(theme::accent_blue())
                                            .size(12.0),
                                    );
                                }
                                if a > 0 || b > 0 {
                                    ui.add_space(4.0);
                                }
                            }
                            None if !branch.is_empty() && branch != "—" => {
                                // Head branch with no upstream tracking
                                ui.label(
                                    RichText::new("no upstream")
                                        .color(theme::text_muted())
                                        .size(12.0),
                                )
                                .on_hover_text(
                                    "No upstream tracking branch — use Push and Set Upstream",
                                );
                                ui.add_space(4.0);
                            }
                            None => {}
                        }

                        ui.label(icons::rich(icons::COMMIT, 13.0).color(theme::accent_teal()));
                        ui.add_space(2.0);
                        ui.label(
                            RichText::new(&branch)
                                .color(theme::accent_teal())
                                .size(12.0),
                        );
                    });
                });
            });
    }
}
