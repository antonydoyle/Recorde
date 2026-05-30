use egui::{Color32, FontId, RichText, Ui};

use crate::state::SharedState;
use crate::theme;

pub struct DetailPanel {
    state: SharedState,
}

impl DetailPanel {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    pub fn show(&mut self, ui: &mut Ui) {
        ui.painter()
            .rect_filled(ui.max_rect(), 0.0, theme::BG_SURFACE);

        let state = self.state.lock().unwrap();

        if state.selected_commit.is_none() {
            ui.centered_and_justified(|ui| {
                ui.label(
                    RichText::new("Select a commit to view details.")
                        .color(theme::TEXT_MUTED)
                        .size(12.0),
                );
            });
            return;
        }

        let Some(ref detail) = state.selected_commit_detail else {
            ui.centered_and_justified(|ui| {
                ui.label(
                    RichText::new("Loading…").color(theme::TEXT_MUTED).size(12.0),
                );
            });
            return;
        };

        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                ui.add_space(12.0);

                // --- SHA header ---
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    let short = &detail.oid[..7.min(detail.oid.len())];
                    ui.label(
                        RichText::new(short)
                            .font(FontId::monospace(13.0))
                            .color(theme::ACCENT_BLUE),
                    );
                    ui.add_space(6.0);
                    if ui
                        .small_button(RichText::new("Copy").color(theme::TEXT_MUTED).size(10.5))
                        .clicked()
                    {
                        ui.output_mut(|o| o.copied_text = detail.oid.clone());
                    }
                });

                ui.add_space(8.0);

                // --- Author / Date ---
                meta_row(ui, "Author", &detail.author);
                meta_row(ui, "Email", &detail.author_email);
                meta_row(ui, "Date", &theme::relative_time(detail.timestamp));

                ui.add_space(10.0);
                ui.add(egui::Separator::default().spacing(0.0));
                ui.add_space(10.0);

                // --- Commit message ---
                let first_line = detail.message.lines().next().unwrap_or("");
                let rest = detail
                    .message
                    .trim_start_matches(first_line)
                    .trim_start_matches('\n')
                    .trim_end();

                ui.horizontal_wrapped(|ui| {
                    ui.add_space(12.0);
                    ui.label(
                        RichText::new(first_line)
                            .color(theme::TEXT_PRIMARY)
                            .size(13.0)
                            .strong(),
                    );
                });

                if !rest.is_empty() {
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.add_space(12.0);
                        ui.label(
                            RichText::new(rest)
                                .color(theme::TEXT_SECONDARY)
                                .size(12.0),
                        );
                    });
                }

                ui.add_space(10.0);
                ui.add(egui::Separator::default().spacing(0.0));
                ui.add_space(6.0);

                // --- Changed files ---
                let file_count = detail.changed_files.len();
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.label(
                        RichText::new(format!(
                            "Changed files  ({})",
                            file_count
                        ))
                        .color(theme::TEXT_MUTED)
                        .size(10.5)
                        .strong(),
                    );
                });

                ui.add_space(4.0);

                for file in &detail.changed_files {
                    let (icon, color) = match file.status {
                        'A' => ("A", theme::ACCENT_GREEN),
                        'D' => ("D", theme::ACCENT_RED),
                        'R' => ("R", theme::ACCENT_YELLOW),
                        'C' => ("C", theme::ACCENT_BLUE),
                        _ => ("M", theme::ACCENT_YELLOW),
                    };

                    let resp = ui
                        .horizontal(|ui| {
                            ui.add_space(12.0);
                            ui.label(
                                RichText::new(icon)
                                    .font(FontId::monospace(11.0))
                                    .color(color),
                            );
                            ui.add_space(6.0);

                            // Show just the filename, full path as tooltip
                            let display_name = file
                                .path
                                .rsplit('/')
                                .next()
                                .unwrap_or(&file.path);

                            let state_ref = &state;
                            let is_selected = state_ref
                                .selected_file
                                .as_deref()
                                == Some(&file.path);

                            let label = ui.selectable_label(
                                is_selected,
                                RichText::new(display_name)
                                    .color(if is_selected {
                                        theme::ACCENT_BLUE
                                    } else {
                                        theme::TEXT_SECONDARY
                                    })
                                    .size(12.0),
                            );

                            if file.path != *display_name {
                                label.on_hover_text(&file.path);
                            }
                        })
                        .response;

                    // Hover highlight
                    if resp.hovered() {
                        ui.painter()
                            .rect_filled(resp.rect, 0.0, Color32::from_white_alpha(6));
                    }
                }

                ui.add_space(16.0);
            });
    }
}

fn meta_row(ui: &mut Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        ui.label(
            RichText::new(format!("{:<8}", label))
                .color(theme::TEXT_MUTED)
                .font(FontId::monospace(11.0)),
        );
        ui.add_space(4.0);
        ui.label(RichText::new(value).color(theme::TEXT_SECONDARY).size(12.0));
    });
}
