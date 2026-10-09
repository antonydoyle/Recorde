use crate::ui::button::Button;
use egui::{Color32, FontId, RichText, Ui};

use crate::state::{RebaseAction, RebaseEntry};
use crate::theme;

pub struct RebaseModal {
    pub preserve_merges: bool,
    pub upstream_oid: String,
    pub upstream_summary: String,
    pub entries: Vec<RebaseEntry>,
}

pub enum RebaseModalResult {
    Execute,
    Cancel,
}

impl RebaseModal {
    pub fn new(upstream_oid: String, upstream_summary: String, entries: Vec<RebaseEntry>) -> Self {
        Self {
            preserve_merges: false,
            upstream_oid,
            upstream_summary,
            entries,
        }
    }

    /// Render the modal contents inside an already-open `egui::Window`.
    /// Returns Some(result) when the user confirms or cancels.
    pub fn show(&mut self, ui: &mut Ui) -> Option<RebaseModalResult> {
        ui.set_min_width(560.0);

        // ── Header ────────────────────────────────────────────────────────────
        ui.label(
            RichText::new("Interactive Rebase")
                .size(15.0)
                .color(theme::text_primary())
                .strong(),
        );
        ui.add_space(2.0);
        ui.label(
            RichText::new(format!(
                "Replaying {} commit{} onto  {}  {}",
                self.entries.len(),
                if self.entries.len() == 1 { "" } else { "s" },
                &self.upstream_oid[..7.min(self.upstream_oid.len())],
                self.upstream_summary,
            ))
            .size(12.0)
            .color(theme::text_muted()),
        );

        ui.add_space(8.0);
        if self.preserve_merges {
            ui.label("Merge topology is preserved. Keep the original order; Pick and Reword are available.");
        }

        // Column headers
        ui.horizontal(|ui| {
            ui.add_space(26.0);
            ui.label(
                RichText::new("Action")
                    .color(theme::text_muted())
                    .size(12.0)
                    .strong(),
            );
            ui.add_space(72.0);
            ui.label(
                RichText::new("SHA    ")
                    .color(theme::text_muted())
                    .size(12.0)
                    .strong(),
            );
            ui.label(
                RichText::new("Summary")
                    .color(theme::text_muted())
                    .size(12.0)
                    .strong(),
            );
        });
        ui.add_space(2.0);
        ui.separator();

        // ── Sortable commit list ──────────────────────────────────────────────
        let mut action_changes: Vec<(usize, RebaseAction)> = Vec::new();
        // drag_from and drop_at are both set by the drop zone (payload = source index)
        let mut drag_from: Option<usize> = None;
        let mut drop_at: Option<usize> = None;
        let is_dragging = !self.preserve_merges && egui::DragAndDrop::has_any_payload(ui.ctx());

        egui::ScrollArea::vertical()
            .max_height(380.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                // Drop zone at position 0
                receive_drop(ui, 0, is_dragging, &mut drag_from, &mut drop_at);

                for i in 0..self.entries.len() {
                    let short_sha =
                        self.entries[i].oid[..7.min(self.entries[i].oid.len())].to_string();
                    let summary = self.entries[i].summary.clone();
                    let cur_action = self.entries[i].action;
                    let is_drop = cur_action == RebaseAction::Drop;
                    let row_id = egui::Id::new(("rebase_row", i));

                    // Wrap the row in a drag source; payload = index i
                    let item = ui.dnd_drag_source(row_id, i, |ui| {
                        ui.horizontal(|ui| {
                            // Drag handle
                            ui.label(RichText::new("⠿").color(theme::text_muted()).size(14.0));

                            // Action ComboBox — must avoid shadowing the free fn `action_color`
                            let mut selected_action = cur_action;
                            let cur_color = action_color(cur_action);
                            egui::ComboBox::new(egui::Id::new(("rebase_action", i)), "")
                                .selected_text(
                                    RichText::new(cur_action.label())
                                        .color(cur_color)
                                        .size(12.0)
                                        .monospace(),
                                )
                                .width(92.0)
                                .show_ui(ui, |ui| {
                                    for a in &[
                                        RebaseAction::Pick,
                                        RebaseAction::Reword,
                                        RebaseAction::Squash,
                                        RebaseAction::Fixup,
                                        RebaseAction::Drop,
                                    ] {
                                        if self.preserve_merges
                                            && !matches!(
                                                a,
                                                RebaseAction::Pick | RebaseAction::Reword
                                            )
                                        {
                                            continue;
                                        }
                                        ui.selectable_value(
                                            &mut selected_action,
                                            *a,
                                            RichText::new(a.label())
                                                .color(action_color(*a))
                                                .monospace(),
                                        );
                                    }
                                });
                            if selected_action != cur_action {
                                action_changes.push((i, selected_action));
                            }

                            // SHA
                            ui.label(
                                RichText::new(&short_sha)
                                    .font(FontId::monospace(12.0))
                                    .color(if is_drop {
                                        theme::text_muted()
                                    } else {
                                        theme::accent_blue()
                                    }),
                            );

                            // Summary (dimmed for drop)
                            let text_col = if is_drop {
                                theme::text_muted()
                            } else {
                                theme::text_secondary()
                            };
                            ui.label(RichText::new(&summary).size(13.0).color(text_col));
                        });
                    });
                    let _ = item;
                    if matches!(cur_action, RebaseAction::Reword | RebaseAction::Squash) {
                        ui.label(if cur_action == RebaseAction::Squash {
                            "Resulting combined commit message"
                        } else {
                            "New commit message"
                        });
                        ui.add(
                            egui::TextEdit::multiline(&mut self.entries[i].message)
                                .desired_rows(3)
                                .desired_width(f32::INFINITY),
                        );
                    }

                    // Drop zone after each row
                    receive_drop(ui, i + 1, is_dragging, &mut drag_from, &mut drop_at);
                }
            });

        // Apply action changes
        for (i, action) in action_changes {
            if action == RebaseAction::Squash && self.entries[i].action != RebaseAction::Squash {
                if let Some(previous) = self.entries[..i]
                    .iter()
                    .rev()
                    .find(|e| e.action != RebaseAction::Drop)
                {
                    self.entries[i].message = format!(
                        "{}\n\n{}",
                        previous.message.trim(),
                        self.entries[i].message.trim()
                    );
                }
            }
            self.entries[i].action = action;
        }

        // Apply drag-and-drop reorder
        if self.preserve_merges {
            drag_from = None;
            drop_at = None;
        }
        if let (Some(from), Some(to)) = (drag_from, drop_at) {
            if to != from && to != from + 1 {
                let entry = self.entries.remove(from);
                let insert = if to > from { to - 1 } else { to };
                self.entries.insert(insert, entry);
            }
        }

        ui.add_space(4.0);
        ui.separator();
        ui.add_space(6.0);

        // ── Legend ────────────────────────────────────────────────────────────
        ui.horizontal_wrapped(|ui| {
            for (a, desc) in &[
                (RebaseAction::Reword, "edit message  ?  "),
                (RebaseAction::Pick, "use commit  ·  "),
                (RebaseAction::Squash, "meld into prev, edit msg  ·  "),
                (RebaseAction::Fixup, "meld into prev, discard msg  ·  "),
                (RebaseAction::Drop, "remove commit"),
            ] {
                ui.label(
                    RichText::new(a.label())
                        .color(action_color(*a))
                        .monospace()
                        .size(12.0),
                );
                ui.label(RichText::new(*desc).color(theme::text_muted()).size(12.0));
            }
        });

        ui.add_space(12.0);

        // ── Buttons ───────────────────────────────────────────────────────────
        let validation = crate::git::rebase::validate_entries(&self.entries);
        if let Err(error) = &validation {
            ui.colored_label(theme::accent_red(), error);
        }
        let mut result = None;
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    validation.is_ok(),
                    Button::new(
                        RichText::new("Start Rebase")
                            .color(Color32::WHITE)
                            .size(14.0),
                    )
                    .fill(theme::accent_blue())
                    .min_size(egui::Vec2::new(130.0, 28.0)),
                )
                .clicked()
            {
                result = Some(RebaseModalResult::Execute);
            }
            ui.add_space(8.0);
            if ui
                .add(
                    Button::new(RichText::new("Cancel").color(theme::text_secondary()))
                        .fill(Color32::TRANSPARENT)
                        .stroke(egui::Stroke::new(1.0, theme::border())),
                )
                .clicked()
                || ui.input(|i| i.key_pressed(egui::Key::Escape))
            {
                result = Some(RebaseModalResult::Cancel);
            }
        });
        ui.add_space(4.0);

        result
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn action_color(action: RebaseAction) -> Color32 {
    match action {
        RebaseAction::Pick | RebaseAction::Reword => theme::accent_blue(),
        RebaseAction::Squash => theme::accent_yellow(),
        RebaseAction::Fixup => Color32::from_rgb(0xE5, 0x97, 0x20),
        RebaseAction::Drop => theme::accent_red(),
    }
}

/// Renders a drop-zone gap. When an item is released here, populates `drag_from`
/// (from payload) and `drop_at` (= position).
fn receive_drop(
    ui: &mut Ui,
    position: usize,
    show_line: bool,
    drag_from: &mut Option<usize>,
    drop_at: &mut Option<usize>,
) {
    let (inner, payload) = ui.dnd_drop_zone::<usize, ()>(
        egui::Frame::none().inner_margin(egui::Margin::symmetric(0.0, 2.0)),
        |ui| {
            if show_line {
                let w = ui.available_width();
                let (rect, _) =
                    ui.allocate_exact_size(egui::Vec2::new(w, 4.0), egui::Sense::hover());
                let hovered = inner_is_hovered(ui, rect);
                let color = if hovered {
                    theme::accent_blue()
                } else {
                    theme::border()
                };
                ui.painter().line_segment(
                    [rect.left_center(), rect.right_center()],
                    egui::Stroke::new(if hovered { 2.0 } else { 1.0 }, color),
                );
            } else {
                ui.add_space(1.0);
            }
        },
    );
    let _ = inner;

    if let Some(p) = payload {
        *drag_from = Some(*p);
        *drop_at = Some(position);
    }
}

fn inner_is_hovered(ui: &egui::Ui, rect: egui::Rect) -> bool {
    ui.ctx()
        .input(|i| i.pointer.hover_pos())
        .map_or(false, |p| rect.contains(p))
}
