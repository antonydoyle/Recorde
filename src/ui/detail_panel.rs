use egui::{FontId, RichText, Ui};

use crate::git::operations::{load_blame, load_diff, load_file_history};
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
            .rect_filled(ui.max_rect(), 0.0, theme::bg_surface());

        // Snapshot everything we need, then release the lock.
        let (selected_commit, detail, selected_file) = {
            let s = self.state.lock().unwrap();
            (
                s.selected_commit.clone(),
                s.selected_commit_detail.clone(),
                s.selected_file.clone(),
            )
        };

        if selected_commit.is_none() {
            ui.centered_and_justified(|ui| {
                ui.label(
                    RichText::new("Select a commit to view details.")
                        .color(theme::text_muted())
                        .size(13.0),
                );
            });
            return;
        }

        let Some(detail) = detail else {
            ui.centered_and_justified(|ui| {
                ui.label(
                    RichText::new("Loading…")
                        .color(theme::text_muted())
                        .size(13.0),
                );
            });
            return;
        };

        // Collect any file the user clicks on during this frame.
        let mut clicked_file: Option<String> = None;

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
                            .font(FontId::monospace(14.0))
                            .color(theme::accent_blue()),
                    );
                    ui.add_space(6.0);
                    if ui
                        .small_button(RichText::new("Copy").color(theme::text_muted()).size(12.0))
                        .clicked()
                    {
                        ui.output_mut(|o| o.copied_text = detail.oid.clone());
                    }
                });

                ui.add_space(8.0);
                meta_row(ui, "Author", &detail.author);
                meta_row(ui, "Email", &detail.author_email);
                meta_row(ui, "Date", &theme::relative_time(detail.timestamp))
                    .on_hover_text(theme::absolute_time(detail.timestamp));

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

                egui::Frame::none()
                    .inner_margin(egui::Margin::symmetric(12.0, 0.0))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(first_line)
                                .color(theme::text_primary())
                                .size(14.0)
                                .strong(),
                        );
                    });

                if !rest.is_empty() {
                    ui.add_space(6.0);
                    egui::Frame::none()
                        .inner_margin(egui::Margin::symmetric(12.0, 0.0))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(rest)
                                    .color(theme::text_secondary())
                                    .size(13.0),
                            );
                        });
                }

                ui.add_space(10.0);
                ui.add(egui::Separator::default().spacing(0.0));
                ui.add_space(6.0);

                // --- Changed files ---
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.label(
                        RichText::new(format!("Changed files  ({})", detail.changed_files.len()))
                            .color(theme::text_muted())
                            .size(12.0)
                            .strong(),
                    );
                });
                ui.add_space(4.0);

                for file in &detail.changed_files {
                    let is_sel = selected_file.as_deref() == Some(&file.path);
                    let (icon, icon_color) = status_icon(file.status);

                    let row_resp = ui
                        .horizontal(|ui| {
                            ui.add_space(12.0);
                            ui.label(
                                RichText::new(icon)
                                    .font(FontId::monospace(12.0))
                                    .color(icon_color),
                            )
                            .on_hover_text(status_tooltip(file.status));
                            ui.add_space(6.0);

                            let display = file.path.rsplit('/').next().unwrap_or(&file.path);
                            let label = ui.selectable_label(
                                is_sel,
                                RichText::new(display)
                                    .color(if is_sel {
                                        theme::text_primary()
                                    } else {
                                        theme::text_secondary()
                                    })
                                    .size(13.0),
                            );

                            if label.on_hover_text(&file.path).clicked() {
                                clicked_file = Some(file.path.clone());
                            }
                        })
                        .response;

                    if row_resp.interact(egui::Sense::click()).clicked() {
                        clicked_file = Some(file.path.clone());
                    }

                    row_resp.context_menu(|ui| {
                        ui.set_min_width(175.0);
                        if ui.button("Show File History").clicked() {
                            load_file_history(&file.path, self.state.clone());
                            ui.close_menu();
                        }
                        if ui.button("Blame").clicked() {
                            {
                                let mut s = self.state.lock().unwrap();
                                s.selected_file = Some(file.path.clone());
                                s.show_blame = true;
                            }
                            load_blame(&file.path, self.state.clone());
                            ui.close_menu();
                        }
                    });
                }

                ui.add_space(16.0);
            });

        // Handle file click outside the borrow scope.
        if let Some(path) = clicked_file {
            {
                let mut s = self.state.lock().unwrap();
                s.selected_file = Some(path.clone());
                s.current_diff = None;
            }
            load_diff(&detail.oid, &path, self.state.clone());
        }
    }
}

fn status_icon(status: char) -> (&'static str, egui::Color32) {
    match status {
        'A' => ("A", theme::accent_green()),
        'D' => ("D", theme::accent_red()),
        'R' => ("R", theme::accent_yellow()),
        'C' => ("C", theme::accent_blue()),
        _ => ("M", theme::accent_yellow()),
    }
}

fn status_tooltip(status: char) -> &'static str {
    match status {
        'A' => "Added",
        'D' => "Deleted",
        'R' => "Renamed",
        'C' => "Copied",
        _ => "Modified",
    }
}

fn meta_row(ui: &mut Ui, label: &str, value: &str) -> egui::Response {
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        ui.label(
            RichText::new(format!("{:<8}", label))
                .color(theme::text_muted())
                .font(FontId::monospace(12.0)),
        );
        ui.add_space(4.0);
        ui.label(
            RichText::new(value)
                .color(theme::text_secondary())
                .size(13.0),
        )
    })
    .response
}
