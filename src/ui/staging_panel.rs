use egui::{Color32, RichText, Ui, Vec2};

use crate::git::{operations::load_workdir_diff, staging};
use crate::state::{SharedState, WorkingDirFile};
use crate::theme;

pub struct StagingPanel {
    state: SharedState,
    subject: String,
    body: String,
    is_amend: bool,
}

impl StagingPanel {
    pub fn new(state: SharedState) -> Self {
        Self { state, subject: String::new(), body: String::new(), is_amend: false }
    }

    pub fn show(&mut self, ui: &mut Ui) {
        ui.painter().rect_filled(ui.max_rect(), 0.0, theme::BG_SURFACE);

        // Snapshot what we need without holding the lock during rendering.
        let (files, head_branch, selected_file, dirty_count) = {
            let s = self.state.lock().unwrap();
            (
                s.working_dir_files.clone(),
                s.head_branch.clone().unwrap_or_default(),
                s.selected_file.clone(),
                s.dirty_count,
            )
        };

        let unstaged: Vec<&WorkingDirFile> =
            files.iter().filter(|f| f.unstaged_status.is_some()).collect();
        let staged: Vec<&WorkingDirFile> =
            files.iter().filter(|f| f.staged_status.is_some()).collect();

        // Actions collected during rendering, applied after.
        let mut stage_file: Option<String> = None;
        let mut unstage_file: Option<String> = None;
        let mut do_stage_all = false;
        let mut do_unstage_all = false;
        let mut do_commit = false;
        let mut do_amend = false;
        let mut view_file: Option<(String, bool)> = None; // (path, staged?)

        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                ui.add_space(8.0);

                // ── UNSTAGED CHANGES ─────────────────────────────────────
                section_header(ui, "UNSTAGED CHANGES", unstaged.len(), |ui| {
                    if ui
                        .add_enabled(
                            !unstaged.is_empty(),
                            small_btn("Stage All"),
                        )
                        .clicked()
                    {
                        do_stage_all = true;
                    }
                });

                if unstaged.is_empty() {
                    empty_label(ui, "Nothing to stage.");
                } else {
                    for f in &unstaged {
                        let is_sel = selected_file.as_deref() == Some(f.path.as_str());
                        let resp = file_row(ui, &f.path, f.unstaged_status.unwrap_or('?'), is_sel, |ui| {
                            if ui.add(small_btn("Stage")).clicked() {
                                stage_file = Some(f.path.clone());
                            }
                        });
                        if resp.clicked() {
                            view_file = Some((f.path.clone(), false));
                        }
                    }
                }

                ui.add_space(6.0);

                // ── STAGED CHANGES ───────────────────────────────────────
                section_header(ui, "STAGED CHANGES", staged.len(), |ui| {
                    if ui
                        .add_enabled(
                            !staged.is_empty(),
                            small_btn("Unstage All"),
                        )
                        .clicked()
                    {
                        do_unstage_all = true;
                    }
                });

                if staged.is_empty() {
                    empty_label(ui, "Nothing staged for commit.");
                } else {
                    for f in &staged {
                        let is_sel = selected_file.as_deref() == Some(f.path.as_str());
                        let resp = file_row(ui, &f.path, f.staged_status.unwrap_or('M'), is_sel, |ui| {
                            if ui.add(small_btn("Unstage")).clicked() {
                                unstage_file = Some(f.path.clone());
                            }
                        });
                        if resp.clicked() {
                            view_file = Some((f.path.clone(), true));
                        }
                    }
                }

                ui.add_space(10.0);
                ui.add(egui::Separator::default().spacing(0.0));
                ui.add_space(8.0);

                // ── COMMIT FORM ──────────────────────────────────────────
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.label(
                        RichText::new("Commit message")
                            .color(theme::TEXT_MUTED)
                            .size(10.5)
                            .strong(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(12.0);
                        // Amend toggle
                        let amend_color = if self.is_amend { theme::ACCENT_YELLOW } else { theme::TEXT_MUTED };
                        if ui
                            .add(
                                egui::Button::new(RichText::new("Amend").color(amend_color).size(11.0))
                                    .fill(if self.is_amend { Color32::from_rgb(0x2A, 0x24, 0x10) } else { Color32::TRANSPARENT })
                                    .stroke(egui::Stroke::new(1.0, if self.is_amend { theme::ACCENT_YELLOW } else { theme::BORDER })),
                            )
                            .clicked()
                        {
                            self.is_amend = !self.is_amend;
                            if self.is_amend && self.subject.is_empty() {
                                // Pre-populate with HEAD commit message
                                if let Some(msg) = staging::load_head_message(self.state.clone()) {
                                    let mut lines = msg.lines();
                                    self.subject = lines.next().unwrap_or("").to_string();
                                    let rest: String = lines.skip_while(|l| l.is_empty()).collect::<Vec<_>>().join("\n");
                                    self.body = rest;
                                }
                            }
                        }
                    });
                });

                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    let subject_edit = ui.add(
                        egui::TextEdit::singleline(&mut self.subject)
                            .desired_width(ui.available_width() - 12.0)
                            .hint_text("Summary (required)")
                            .font(egui::FontId::proportional(12.5)),
                    );
                    let _ = subject_edit;
                });

                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.add(
                        egui::TextEdit::multiline(&mut self.body)
                            .desired_width(ui.available_width() - 12.0)
                            .desired_rows(3)
                            .hint_text("Description (optional)")
                            .font(egui::FontId::proportional(12.0)),
                    );
                });

                ui.add_space(10.0);

                // ── COMMIT BUTTON ────────────────────────────────────────
                let can_commit = !self.subject.trim().is_empty() && !staged.is_empty();
                let btn_label = if self.is_amend {
                    format!("Amend commit on {}", head_branch)
                } else {
                    format!("Commit to {}", head_branch)
                };

                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    let btn = ui.add_enabled(
                        can_commit,
                        egui::Button::new(
                            RichText::new(&btn_label).color(Color32::WHITE).size(12.5),
                        )
                        .fill(if can_commit { theme::ACCENT_BLUE } else { theme::BG_ELEVATED })
                        .min_size(Vec2::new(ui.available_width() - 12.0, 28.0)),
                    );
                    if btn.clicked() {
                        if self.is_amend {
                            do_amend = true;
                        } else {
                            do_commit = true;
                        }
                    }
                });

                ui.add_space(16.0);
            });

        // ── Apply actions ────────────────────────────────────────────────
        if let Some(path) = stage_file {
            staging::stage_file(&path, self.state.clone());
        }
        if let Some(path) = unstage_file {
            staging::unstage_file(&path, self.state.clone());
        }
        if do_stage_all {
            staging::stage_all(self.state.clone());
        }
        if do_unstage_all {
            staging::unstage_all(self.state.clone());
        }

        if do_commit || do_amend {
            let full_msg = if self.body.trim().is_empty() {
                self.subject.trim().to_string()
            } else {
                format!("{}\n\n{}", self.subject.trim(), self.body.trim())
            };
            if do_commit {
                staging::create_commit(&full_msg, self.state.clone());
            } else {
                staging::amend_commit(&full_msg, self.state.clone());
            }
            self.subject.clear();
            self.body.clear();
            self.is_amend = false;
        }

        if let Some((path, is_staged)) = view_file {
            {
                let mut s = self.state.lock().unwrap();
                s.selected_file = Some(path.clone());
                s.current_diff = None;
            }
            load_workdir_diff(&path, is_staged, self.state.clone());
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn section_header(
    ui: &mut Ui,
    title: &str,
    count: usize,
    add_right: impl FnOnce(&mut Ui),
) {
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        ui.label(
            RichText::new(format!("{}  ({})", title, count))
                .color(theme::TEXT_MUTED)
                .size(10.5)
                .strong(),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(12.0);
            add_right(ui);
        });
    });
    ui.add_space(2.0);
}

/// Renders a single file row with a right-aligned button. Returns the row response.
fn file_row(
    ui: &mut Ui,
    path: &str,
    status: char,
    is_selected: bool,
    add_btn: impl FnOnce(&mut Ui),
) -> egui::Response {
    let row_resp = ui
        .horizontal(|ui| {
            ui.add_space(12.0);
            let (icon, color) = status_icon(status);
            ui.label(RichText::new(icon).font(egui::FontId::monospace(11.0)).color(color));
            ui.add_space(4.0);
            let name = path.rsplit('/').next().unwrap_or(path);
            let label = ui.selectable_label(
                is_selected,
                RichText::new(name)
                    .color(if is_selected { theme::ACCENT_BLUE } else { theme::TEXT_SECONDARY })
                    .size(12.0),
            );
            if path != name {
                label.on_hover_text(path);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(8.0);
                add_btn(ui);
            });
        })
        .response;

    if row_resp.hovered() {
        ui.painter()
            .rect_filled(row_resp.rect, 0.0, Color32::from_white_alpha(5));
    }

    row_resp
}

fn small_btn(label: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(label).color(theme::TEXT_MUTED).size(10.5))
        .fill(Color32::TRANSPARENT)
        .stroke(egui::Stroke::new(1.0, theme::BORDER))
}

fn status_icon(status: char) -> (&'static str, Color32) {
    match status {
        'A' | '?' => ("A", theme::ACCENT_GREEN),
        'D' => ("D", theme::ACCENT_RED),
        'R' => ("R", theme::ACCENT_YELLOW),
        _ => ("M", theme::ACCENT_YELLOW),
    }
}

fn empty_label(ui: &mut Ui, text: &str) {
    ui.horizontal(|ui| {
        ui.add_space(20.0);
        ui.label(RichText::new(text).color(theme::TEXT_MUTED).size(11.0));
    });
    ui.add_space(2.0);
}
