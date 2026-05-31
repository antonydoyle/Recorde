use egui::{Color32, FontId, Pos2, Rect, Sense, Stroke, Ui, Vec2};

use crate::state::{FileDiff, SharedState};
use crate::theme;

const ROW_HEIGHT: f32 = 19.0;
const GUTTER_W: f32 = 76.0; // "  1234   1234  " (old + new line numbers)
const BAR_W: f32 = 3.0;      // left colour bar for +/- lines
const FONT_SIZE: f32 = 12.0;

// Subtle tinted row backgrounds for added/deleted lines
use crate::theme::{BG_DIFF_ADD as BG_ADD, BG_DIFF_DEL as BG_DEL, BG_DIFF_HUNK as BG_HUNK};

/// A flat renderable item — one per visual row.
#[derive(Clone)]
enum Item {
    /// "@@ -1,4 +1,8 @@ context"
    HunkHeader(String),
    /// A diff line (+/-/context)
    Line {
        origin: char,
        old_no: String,
        new_no: String,
        content: String,
    },
    Binary,
}

pub struct DiffView {
    state: SharedState,
    /// Pre-flattened items for the currently displayed diff.
    items: Vec<Item>,
    /// OID+path key we last built items for.
    last_key: Option<(String, String)>,
    show_console: bool,
}

impl DiffView {
    pub fn new(state: SharedState) -> Self {
        Self { state, items: Vec::new(), last_key: None, show_console: false }
    }

    pub fn show(&mut self, ui: &mut Ui) {
        // Snapshot state — release lock before any rendering.
        let (diff_opt, selected_file, console_log) = {
            let s = self.state.lock().unwrap();
            (s.current_diff.clone(), s.selected_file.clone(), s.console_log.clone())
        };

        ui.painter().rect_filled(ui.max_rect(), 0.0, theme::BG_APP);

        // Rebuild flat item list when the diff changes.
        if let Some(ref diff) = diff_opt {
            let key = (diff.commit_oid.clone(), diff.file_path.clone());
            if self.last_key.as_ref() != Some(&key) {
                self.items = flatten(diff);
                self.last_key = Some(key);
            }
        } else {
            self.items.clear();
            self.last_key = None;
        }

        // ── Header bar ────────────────────────────────────────────────────
        let header_height = 36.0;
        let header_rect = Rect::from_min_size(
            ui.min_rect().min,
            Vec2::new(ui.available_width(), header_height),
        );
        ui.painter().rect_filled(header_rect, 0.0, theme::BG_PANEL);
        ui.painter().line_segment(
            [Pos2::new(header_rect.min.x, header_rect.max.y),
             Pos2::new(header_rect.max.x, header_rect.max.y)],
            Stroke::new(1.0, theme::BORDER),
        );

        ui.allocate_ui_at_rect(header_rect, |ui| {
            ui.horizontal_centered(|ui| {
                ui.add_space(12.0);

                // "File Diff" and "Console" tab labels
                for (label, is_console) in &[("File Diff", false), ("Console", true)] {
                    let active = *is_console == self.show_console;
                    let tc = if active { theme::TEXT_PRIMARY } else { theme::TEXT_MUTED };
                    let btn = ui.add(
                        egui::Button::new(egui::RichText::new(*label).size(12.5).color(tc))
                            .fill(Color32::TRANSPARENT)
                            .stroke(Stroke::NONE),
                    );
                    if btn.clicked() { self.show_console = *is_console; }
                    if active {
                        let br = btn.rect;
                        ui.painter().line_segment(
                            [br.left_bottom(), br.right_bottom()],
                            Stroke::new(2.0, theme::ACCENT_TEAL),
                        );
                    }
                    ui.add_space(10.0);
                }

                ui.add_space(4.0);
                ui.add(egui::Separator::default().vertical().spacing(3.0));
                ui.add_space(6.0);

                // Filename
                if let Some(ref diff) = diff_opt {
                    let filename = diff.file_path.rsplit('/').next().unwrap_or(&diff.file_path);
                    ui.label(egui::RichText::new(filename).color(theme::TEXT_PRIMARY).size(12.5).strong());
                    if diff.file_path != filename {
                        ui.label(egui::RichText::new(&diff.file_path).color(theme::TEXT_MUTED).size(11.0));
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(10.0);
                        // Close / clear selection
                        if ui.add(egui::Button::new(egui::RichText::new("✕").color(theme::TEXT_MUTED).size(12.0))
                            .fill(Color32::TRANSPARENT).stroke(Stroke::NONE)).clicked() {
                            let mut s = self.state.lock().unwrap();
                            s.selected_file = None; s.current_diff = None;
                        }
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new(format!("+{}  −{}", diff.additions, diff.deletions))
                            .color(theme::TEXT_SECONDARY)
                            .size(11.5),
                        );
                    });
                }
            });
        });

        // Reserve space below the header
        let content_rect = Rect::from_min_max(
            Pos2::new(ui.min_rect().min.x, ui.min_rect().min.y + header_height),
            ui.max_rect().max,
        );

        // --- Console tab ---
        if self.show_console {
            ui.allocate_ui_at_rect(content_rect, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        ui.add_space(6.0);
                        if console_log.is_empty() {
                            ui.horizontal(|ui| {
                                ui.add_space(12.0);
                                ui.label(egui::RichText::new("No git operations yet.")
                                    .color(theme::TEXT_MUTED).size(12.0));
                            });
                        } else {
                            for line in &console_log {
                                let color = if line.starts_with("error:") {
                                    theme::ACCENT_RED
                                } else if line.starts_with('$') {
                                    theme::ACCENT_TEAL
                                } else {
                                    theme::TEXT_SECONDARY
                                };
                                ui.horizontal(|ui| {
                                    ui.add_space(12.0);
                                    ui.label(egui::RichText::new(line)
                                        .color(color)
                                        .size(12.0)
                                        .font(egui::FontId::monospace(12.0)));
                                });
                            }
                        }
                        ui.add_space(6.0);
                    });
            });
            return;
        }

        // --- No diff yet ---
        if diff_opt.is_none() {
            ui.allocate_ui_at_rect(content_rect, |ui| {
                ui.centered_and_justified(|ui| {
                    ui.label(
                        egui::RichText::new(if selected_file.is_some() {
                            "Loading diff…"
                        } else {
                            "Select a file to view its diff."
                        })
                        .color(theme::TEXT_MUTED)
                        .size(12.0),
                    );
                });
            });
            return;
        }

        // Binary file notice
        if diff_opt.as_ref().map_or(false, |d| d.is_binary) {
            ui.allocate_ui_at_rect(content_rect, |ui| {
                ui.centered_and_justified(|ui| {
                    ui.label(
                        egui::RichText::new("Binary file — no diff available.")
                            .color(theme::TEXT_MUTED)
                            .size(12.0),
                    );
                });
            });
            return;
        }

        if self.items.is_empty() {
            ui.allocate_ui_at_rect(content_rect, |ui| {
                ui.centered_and_justified(|ui| {
                    ui.label(
                        egui::RichText::new("No changes.")
                            .color(theme::TEXT_MUTED)
                            .size(12.0),
                    );
                });
            });
            return;
        }

        // --- Diff content ---
        let avail_width = content_rect.width();
        let total_height = self.items.len() as f32 * ROW_HEIGHT;
        let items = &self.items; // borrow for closure

        ui.allocate_ui_at_rect(content_rect, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    let (resp, painter) = ui.allocate_painter(
                        Vec2::new(avail_width, total_height),
                        Sense::hover(),
                    );
                    let origin = resp.rect.min;
                    let clip = painter.clip_rect();

                    let first = ((clip.top() - origin.y) / ROW_HEIGHT).floor().max(0.0) as usize;
                    let last =
                        (((clip.bottom() - origin.y) / ROW_HEIGHT).ceil() as usize + 1)
                            .min(items.len());

                    for i in first..last {
                        let y = origin.y + i as f32 * ROW_HEIGHT;
                        let row_rect = Rect::from_min_size(
                            Pos2::new(origin.x, y),
                            Vec2::new(avail_width, ROW_HEIGHT),
                        );

                        match &items[i] {
                            Item::HunkHeader(text) => {
                                painter.rect_filled(row_rect, 0.0, BG_HUNK);
                                painter.line_segment(
                                    [
                                        Pos2::new(row_rect.min.x, row_rect.max.y - 0.5),
                                        Pos2::new(row_rect.max.x, row_rect.max.y - 0.5),
                                    ],
                                    Stroke::new(0.5, theme::BORDER),
                                );
                                painter.text(
                                    Pos2::new(origin.x + 10.0, y + ROW_HEIGHT / 2.0),
                                    egui::Align2::LEFT_CENTER,
                                    text,
                                    FontId::monospace(FONT_SIZE),
                                    theme::TEXT_MUTED,
                                );
                            }

                            Item::Line { origin: ch, old_no, new_no, content } => {
                                let (row_bg, bar_color, text_color) = match ch {
                                    '+' => (BG_ADD, theme::ACCENT_GREEN, Color32::from_rgb(0xAB, 0xE0, 0xAB)),
                                    '-' => (BG_DEL, theme::ACCENT_RED, Color32::from_rgb(0xF0, 0xA0, 0xA0)),
                                    _ => (Color32::TRANSPARENT, Color32::TRANSPARENT, theme::TEXT_PRIMARY),
                                };

                                if row_bg != Color32::TRANSPARENT {
                                    painter.rect_filled(row_rect, 0.0, row_bg);
                                }

                                // Left colour bar
                                if bar_color != Color32::TRANSPARENT {
                                    painter.rect_filled(
                                        Rect::from_min_size(
                                            Pos2::new(origin.x, y),
                                            Vec2::new(BAR_W, ROW_HEIGHT),
                                        ),
                                        0.0,
                                        bar_color,
                                    );
                                }

                                let cy = y + ROW_HEIGHT / 2.0;

                                // Old line number
                                painter.text(
                                    Pos2::new(origin.x + BAR_W + 4.0 + 30.0, cy),
                                    egui::Align2::RIGHT_CENTER,
                                    old_no,
                                    FontId::monospace(10.5),
                                    theme::TEXT_MUTED,
                                );

                                // New line number
                                painter.text(
                                    Pos2::new(origin.x + BAR_W + 4.0 + 68.0, cy),
                                    egui::Align2::RIGHT_CENTER,
                                    new_no,
                                    FontId::monospace(10.5),
                                    theme::TEXT_MUTED,
                                );

                                // Gutter separator
                                painter.line_segment(
                                    [
                                        Pos2::new(origin.x + GUTTER_W, y),
                                        Pos2::new(origin.x + GUTTER_W, y + ROW_HEIGHT),
                                    ],
                                    Stroke::new(0.5, theme::BORDER),
                                );

                                // +/- sigil
                                if *ch != ' ' && *ch != '\\' {
                                    painter.text(
                                        Pos2::new(origin.x + GUTTER_W + 8.0, cy),
                                        egui::Align2::LEFT_CENTER,
                                        &ch.to_string(),
                                        FontId::monospace(FONT_SIZE),
                                        bar_color,
                                    );
                                }

                                // Line content (clipped)
                                let content_x = origin.x + GUTTER_W + 18.0;
                                let content_clip = Rect::from_min_max(
                                    Pos2::new(content_x, y),
                                    Pos2::new(origin.x + avail_width - 4.0, y + ROW_HEIGHT),
                                );
                                painter.with_clip_rect(content_clip).text(
                                    Pos2::new(content_x, cy),
                                    egui::Align2::LEFT_CENTER,
                                    content,
                                    FontId::monospace(FONT_SIZE),
                                    text_color,
                                );
                            }

                            Item::Binary => {
                                painter.text(
                                    Pos2::new(origin.x + 12.0, y + ROW_HEIGHT / 2.0),
                                    egui::Align2::LEFT_CENTER,
                                    "Binary file",
                                    FontId::proportional(12.0),
                                    theme::TEXT_MUTED,
                                );
                            }
                        }
                    }
                });
        });
    }
}

/// Flatten `FileDiff` into a list of renderable rows with pre-formatted line numbers.
fn flatten(diff: &FileDiff) -> Vec<Item> {
    if diff.is_binary {
        return vec![Item::Binary];
    }

    let mut items = Vec::new();
    for hunk in &diff.hunks {
        items.push(Item::HunkHeader(hunk.header.clone()));
        for line in &hunk.lines {
            let old_no = line
                .old_lineno
                .map(|n| n.to_string())
                .unwrap_or_default();
            let new_no = line
                .new_lineno
                .map(|n| n.to_string())
                .unwrap_or_default();
            items.push(Item::Line {
                origin: line.origin,
                old_no,
                new_no,
                content: line.content.clone(),
            });
        }
    }
    items
}
