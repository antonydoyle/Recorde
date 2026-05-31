use egui::{Color32, FontId, Pos2, Rect, Sense, Stroke, Ui, Vec2};

use crate::state::{FileDiff, SharedState};
use crate::theme;

const ROW_HEIGHT: f32 = 19.0;
const FONT_SIZE: f32 = 12.0;
// Unified gutter: "  1234   1234  "
const U_GUTTER: f32 = 76.0;
const BAR_W: f32 = 3.0;
// Split gutter per side: "  1234  "
const S_GUTTER: f32 = 44.0;

use crate::theme::{BG_DIFF_ADD as BG_ADD, BG_DIFF_DEL as BG_DEL, BG_DIFF_HUNK as BG_HUNK};

// ── Unified items ─────────────────────────────────────────────────────────────

#[derive(Clone)]
enum Item {
    HunkHeader(String),
    Line { origin: char, old_no: String, new_no: String, content: String },
    Binary,
}

// ── Split items ───────────────────────────────────────────────────────────────

#[derive(Clone)]
struct SideLine {
    lineno: String,
    content: String,
    changed: bool,
}

#[derive(Clone)]
enum SplitRow {
    HunkHeader(String),
    Pair { left: Option<SideLine>, right: Option<SideLine> },
}

// ── DiffView ──────────────────────────────────────────────────────────────────

pub struct DiffView {
    state: SharedState,
    items: Vec<Item>,
    split_rows: Vec<SplitRow>,
    last_key: Option<(String, String)>,
    show_console: bool,
    split_view: bool,
}

impl DiffView {
    pub fn new(state: SharedState) -> Self {
        Self {
            state,
            items: Vec::new(),
            split_rows: Vec::new(),
            last_key: None,
            show_console: false,
            split_view: false,
        }
    }

    pub fn show(&mut self, ui: &mut Ui) {
        let (diff_opt, selected_file, console_log) = {
            let s = self.state.lock().unwrap();
            (s.current_diff.clone(), s.selected_file.clone(), s.console_log.clone())
        };

        ui.painter().rect_filled(ui.max_rect(), 0.0, theme::BG_APP);

        // Rebuild item caches when diff changes.
        if let Some(ref diff) = diff_opt {
            let key = (diff.commit_oid.clone(), diff.file_path.clone());
            if self.last_key.as_ref() != Some(&key) {
                self.items = flatten_unified(diff);
                self.split_rows = flatten_split(diff);
                self.last_key = Some(key);
            }
        } else {
            self.items.clear();
            self.split_rows.clear();
            self.last_key = None;
        }

        // ── Header ────────────────────────────────────────────────────────
        let header_h = 36.0;
        let header_rect = Rect::from_min_size(ui.min_rect().min, Vec2::new(ui.available_width(), header_h));
        ui.painter().rect_filled(header_rect, 0.0, theme::BG_PANEL);
        ui.painter().line_segment(
            [Pos2::new(header_rect.min.x, header_rect.max.y),
             Pos2::new(header_rect.max.x, header_rect.max.y)],
            Stroke::new(1.0, theme::BORDER),
        );

        ui.allocate_ui_at_rect(header_rect, |ui| {
            ui.horizontal_centered(|ui| {
                ui.add_space(12.0);

                // File Diff / Console tabs
                for (label, is_console) in &[("File Diff", false), ("Console", true)] {
                    let active = *is_console == self.show_console;
                    let tc = if active { theme::TEXT_PRIMARY } else { theme::TEXT_MUTED };
                    let btn = ui.add(
                        egui::Button::new(egui::RichText::new(*label).size(12.5).color(tc))
                            .fill(Color32::TRANSPARENT).stroke(Stroke::NONE),
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

                ui.add(egui::Separator::default().vertical().spacing(3.0));
                ui.add_space(6.0);

                // Filename
                if let Some(ref diff) = diff_opt {
                    let filename = diff.file_path.rsplit('/').next().unwrap_or(&diff.file_path);
                    ui.label(egui::RichText::new(filename).color(theme::TEXT_PRIMARY).size(12.5).strong());
                    if diff.file_path != filename {
                        ui.label(egui::RichText::new(&diff.file_path).color(theme::TEXT_MUTED).size(11.0));
                    }
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(10.0);

                    if let Some(ref diff) = diff_opt {
                        // Close button
                        if ui.add(egui::Button::new(egui::RichText::new("✕").color(theme::TEXT_MUTED).size(12.0))
                            .fill(Color32::TRANSPARENT).stroke(Stroke::NONE)).clicked() {
                            let mut s = self.state.lock().unwrap();
                            s.selected_file = None; s.current_diff = None;
                        }
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new(format!("+{}  −{}", diff.additions, diff.deletions))
                            .color(theme::TEXT_SECONDARY).size(11.5));
                        ui.add_space(8.0);

                        // Unified | Split toggle (only when viewing File Diff)
                        if !self.show_console {
                            ui.add(egui::Separator::default().vertical().spacing(3.0));
                            ui.add_space(6.0);
                            for (label, is_split) in &[("Unified", false), ("Split", true)] {
                                let active = *is_split == self.split_view;
                                let tc = if active { theme::ACCENT_TEAL } else { theme::TEXT_MUTED };
                                let btn = ui.add(
                                    egui::Button::new(egui::RichText::new(*label).size(11.5).color(tc))
                                        .fill(Color32::TRANSPARENT).stroke(Stroke::NONE),
                                );
                                if btn.clicked() { self.split_view = *is_split; }
                                ui.add_space(2.0);
                            }
                        }
                    }
                });
            });
        });

        let content_rect = Rect::from_min_max(
            Pos2::new(ui.min_rect().min.x, ui.min_rect().min.y + header_h),
            ui.max_rect().max,
        );

        // ── Console ───────────────────────────────────────────────────────
        if self.show_console {
            ui.allocate_ui_at_rect(content_rect, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false; 2]).stick_to_bottom(true).show(ui, |ui| {
                    ui.add_space(6.0);
                    if console_log.is_empty() {
                        ui.horizontal(|ui| {
                            ui.add_space(12.0);
                            ui.label(egui::RichText::new("No git operations yet.").color(theme::TEXT_MUTED).size(12.0));
                        });
                    } else {
                        for line in &console_log {
                            let color = if line.starts_with("error:") { theme::ACCENT_RED }
                                else if line.starts_with('$') { theme::ACCENT_TEAL }
                                else { theme::TEXT_SECONDARY };
                            ui.horizontal(|ui| {
                                ui.add_space(12.0);
                                ui.label(egui::RichText::new(line).color(color).size(12.0)
                                    .font(FontId::monospace(12.0)));
                            });
                        }
                    }
                    ui.add_space(6.0);
                });
            });
            return;
        }

        // ── Empty / binary states ─────────────────────────────────────────
        if diff_opt.is_none() {
            ui.allocate_ui_at_rect(content_rect, |ui| {
                ui.centered_and_justified(|ui| {
                    ui.label(egui::RichText::new(
                        if selected_file.is_some() { "Loading diff…" } else { "Select a file to view its diff." }
                    ).color(theme::TEXT_MUTED).size(12.0));
                });
            });
            return;
        }
        if diff_opt.as_ref().map_or(false, |d| d.is_binary) {
            ui.allocate_ui_at_rect(content_rect, |ui| {
                ui.centered_and_justified(|ui| {
                    ui.label(egui::RichText::new("Binary file — no diff available.").color(theme::TEXT_MUTED).size(12.0));
                });
            });
            return;
        }
        if self.items.is_empty() {
            ui.allocate_ui_at_rect(content_rect, |ui| {
                ui.centered_and_justified(|ui| {
                    ui.label(egui::RichText::new("No changes.").color(theme::TEXT_MUTED).size(12.0));
                });
            });
            return;
        }

        // ── Dispatch to unified or split renderer ─────────────────────────
        if self.split_view {
            render_split(ui, content_rect, &self.split_rows);
        } else {
            render_unified(ui, content_rect, &self.items);
        }
    }
}

// ── Unified renderer ──────────────────────────────────────────────────────────

fn render_unified(ui: &mut Ui, rect: Rect, items: &[Item]) {
    let avail_w = rect.width();
    let total_h = items.len() as f32 * ROW_HEIGHT;

    ui.allocate_ui_at_rect(rect, |ui| {
        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            let (resp, painter) = ui.allocate_painter(Vec2::new(avail_w, total_h), Sense::hover());
            let origin = resp.rect.min;
            let clip = painter.clip_rect();

            let first = ((clip.top() - origin.y) / ROW_HEIGHT).floor().max(0.0) as usize;
            let last = (((clip.bottom() - origin.y) / ROW_HEIGHT).ceil() as usize + 1).min(items.len());

            for i in first..last {
                let y = origin.y + i as f32 * ROW_HEIGHT;
                let row_rect = Rect::from_min_size(Pos2::new(origin.x, y), Vec2::new(avail_w, ROW_HEIGHT));

                match &items[i] {
                    Item::HunkHeader(text) => {
                        painter.rect_filled(row_rect, 0.0, BG_HUNK);
                        painter.line_segment(
                            [Pos2::new(row_rect.min.x, row_rect.max.y - 0.5),
                             Pos2::new(row_rect.max.x, row_rect.max.y - 0.5)],
                            Stroke::new(0.5, theme::BORDER),
                        );
                        painter.text(Pos2::new(origin.x + 10.0, y + ROW_HEIGHT / 2.0),
                            egui::Align2::LEFT_CENTER, text, FontId::monospace(FONT_SIZE), theme::TEXT_MUTED);
                    }
                    Item::Line { origin: ch, old_no, new_no, content } => {
                        let (row_bg, bar_col, text_col) = line_colors(*ch);
                        if row_bg != Color32::TRANSPARENT { painter.rect_filled(row_rect, 0.0, row_bg); }
                        if bar_col != Color32::TRANSPARENT {
                            painter.rect_filled(
                                Rect::from_min_size(Pos2::new(origin.x, y), Vec2::new(BAR_W, ROW_HEIGHT)),
                                0.0, bar_col);
                        }
                        let cy = y + ROW_HEIGHT / 2.0;
                        painter.text(Pos2::new(origin.x + BAR_W + 4.0 + 30.0, cy),
                            egui::Align2::RIGHT_CENTER, old_no, FontId::monospace(10.5), theme::TEXT_MUTED);
                        painter.text(Pos2::new(origin.x + BAR_W + 4.0 + 68.0, cy),
                            egui::Align2::RIGHT_CENTER, new_no, FontId::monospace(10.5), theme::TEXT_MUTED);
                        painter.line_segment(
                            [Pos2::new(origin.x + U_GUTTER, y), Pos2::new(origin.x + U_GUTTER, y + ROW_HEIGHT)],
                            Stroke::new(0.5, theme::BORDER));
                        if *ch != ' ' && *ch != '\\' {
                            painter.text(Pos2::new(origin.x + U_GUTTER + 8.0, cy),
                                egui::Align2::LEFT_CENTER, &ch.to_string(), FontId::monospace(FONT_SIZE), bar_col);
                        }
                        let cx = origin.x + U_GUTTER + 18.0;
                        painter.with_clip_rect(Rect::from_min_max(
                            Pos2::new(cx, y), Pos2::new(origin.x + avail_w - 4.0, y + ROW_HEIGHT),
                        )).text(Pos2::new(cx, cy), egui::Align2::LEFT_CENTER, content,
                            FontId::monospace(FONT_SIZE), text_col);
                    }
                    Item::Binary => {
                        painter.text(Pos2::new(origin.x + 12.0, y + ROW_HEIGHT / 2.0),
                            egui::Align2::LEFT_CENTER, "Binary file", FontId::proportional(12.0), theme::TEXT_MUTED);
                    }
                }
            }
        });
    });
}

// ── Split renderer ────────────────────────────────────────────────────────────

fn render_split(ui: &mut Ui, rect: Rect, rows: &[SplitRow]) {
    let avail_w = rect.width();
    let half = (avail_w / 2.0).floor();
    let total_h = rows.len() as f32 * ROW_HEIGHT;

    ui.allocate_ui_at_rect(rect, |ui| {
        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            let (resp, painter) = ui.allocate_painter(Vec2::new(avail_w, total_h), Sense::hover());
            let origin = resp.rect.min;
            let clip = painter.clip_rect();

            // Centre divider
            let div_x = origin.x + half;
            painter.line_segment(
                [Pos2::new(div_x, clip.top()), Pos2::new(div_x, clip.bottom())],
                Stroke::new(1.0, theme::BORDER),
            );

            let first = ((clip.top() - origin.y) / ROW_HEIGHT).floor().max(0.0) as usize;
            let last = (((clip.bottom() - origin.y) / ROW_HEIGHT).ceil() as usize + 1).min(rows.len());

            for i in first..last {
                let y = origin.y + i as f32 * ROW_HEIGHT;

                match &rows[i] {
                    SplitRow::HunkHeader(text) => {
                        let full = Rect::from_min_size(Pos2::new(origin.x, y), Vec2::new(avail_w, ROW_HEIGHT));
                        painter.rect_filled(full, 0.0, BG_HUNK);
                        painter.line_segment(
                            [Pos2::new(full.min.x, full.max.y - 0.5), Pos2::new(full.max.x, full.max.y - 0.5)],
                            Stroke::new(0.5, theme::BORDER));
                        painter.text(Pos2::new(origin.x + 10.0, y + ROW_HEIGHT / 2.0),
                            egui::Align2::LEFT_CENTER, text, FontId::monospace(FONT_SIZE), theme::TEXT_MUTED);
                    }
                    SplitRow::Pair { left, right } => {
                        render_split_side(&painter, origin.x, y, half, left, true);
                        render_split_side(&painter, origin.x + half, y, half, right, false);
                    }
                }
            }
        });
    });
}

fn render_split_side(
    painter: &egui::Painter,
    x0: f32, y: f32, width: f32,
    side: &Option<SideLine>,
    is_left: bool,
) {
    let cy = y + ROW_HEIGHT / 2.0;
    let (bg, bar_col, text_col) = match side {
        Some(sl) if sl.changed && is_left  => (BG_DEL, theme::ACCENT_RED,   Color32::from_rgb(0xF0, 0xA0, 0xA0)),
        Some(sl) if sl.changed && !is_left => (BG_ADD, theme::ACCENT_GREEN,  Color32::from_rgb(0xAB, 0xE0, 0xAB)),
        Some(_)                            => (Color32::TRANSPARENT, Color32::TRANSPARENT, theme::TEXT_PRIMARY),
        None                               => (theme::BG_PANEL_ALT, Color32::TRANSPARENT, theme::TEXT_PRIMARY),
    };

    let row_rect = Rect::from_min_size(Pos2::new(x0, y), Vec2::new(width, ROW_HEIGHT));
    if bg != Color32::TRANSPARENT { painter.rect_filled(row_rect, 0.0, bg); }

    if bar_col != Color32::TRANSPARENT {
        painter.rect_filled(
            Rect::from_min_size(Pos2::new(x0, y), Vec2::new(BAR_W, ROW_HEIGHT)),
            0.0, bar_col);
    }

    if let Some(sl) = side {
        // Line number
        painter.text(Pos2::new(x0 + BAR_W + 4.0 + S_GUTTER - 6.0, cy),
            egui::Align2::RIGHT_CENTER, &sl.lineno, FontId::monospace(10.5), theme::TEXT_MUTED);

        // Gutter divider
        painter.line_segment(
            [Pos2::new(x0 + S_GUTTER, y), Pos2::new(x0 + S_GUTTER, y + ROW_HEIGHT)],
            Stroke::new(0.5, theme::BORDER));

        // +/- sigil
        if sl.changed {
            let sigil = if is_left { "-" } else { "+" };
            painter.text(Pos2::new(x0 + S_GUTTER + 6.0, cy),
                egui::Align2::LEFT_CENTER, sigil, FontId::monospace(FONT_SIZE), bar_col);
        }

        // Content
        let cx = x0 + S_GUTTER + 16.0;
        painter.with_clip_rect(Rect::from_min_max(
            Pos2::new(cx, y), Pos2::new(x0 + width - 2.0, y + ROW_HEIGHT),
        )).text(Pos2::new(cx, cy), egui::Align2::LEFT_CENTER, &sl.content,
            FontId::monospace(FONT_SIZE), text_col);
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn line_colors(ch: char) -> (Color32, Color32, Color32) {
    match ch {
        '+' => (BG_ADD, theme::ACCENT_GREEN,  Color32::from_rgb(0xAB, 0xE0, 0xAB)),
        '-' => (BG_DEL, theme::ACCENT_RED,    Color32::from_rgb(0xF0, 0xA0, 0xA0)),
        _   => (Color32::TRANSPARENT, Color32::TRANSPARENT, theme::TEXT_PRIMARY),
    }
}

// ── Flatten: unified ──────────────────────────────────────────────────────────

fn flatten_unified(diff: &FileDiff) -> Vec<Item> {
    if diff.is_binary { return vec![Item::Binary]; }
    let mut items = Vec::new();
    for hunk in &diff.hunks {
        items.push(Item::HunkHeader(hunk.header.clone()));
        for line in &hunk.lines {
            items.push(Item::Line {
                origin: line.origin,
                old_no: line.old_lineno.map(|n| n.to_string()).unwrap_or_default(),
                new_no: line.new_lineno.map(|n| n.to_string()).unwrap_or_default(),
                content: line.content.trim_end_matches('\n').to_string(),
            });
        }
    }
    items
}

// ── Flatten: split ────────────────────────────────────────────────────────────

fn flatten_split(diff: &FileDiff) -> Vec<SplitRow> {
    if diff.is_binary { return vec![SplitRow::HunkHeader("Binary file".into())]; }

    let mut rows = Vec::new();
    for hunk in &diff.hunks {
        rows.push(SplitRow::HunkHeader(hunk.header.clone()));

        // Collect deleted and added lines; emit context immediately.
        let mut dels: Vec<SideLine> = Vec::new();
        let mut adds: Vec<SideLine> = Vec::new();

        let flush = |rows: &mut Vec<SplitRow>, dels: &mut Vec<SideLine>, adds: &mut Vec<SideLine>| {
            let n = dels.len().max(adds.len());
            for i in 0..n {
                rows.push(SplitRow::Pair {
                    left:  dels.get(i).cloned(),
                    right: adds.get(i).cloned(),
                });
            }
            dels.clear();
            adds.clear();
        };

        for line in &hunk.lines {
            let content = line.content.trim_end_matches('\n').to_string();
            match line.origin {
                '-' => dels.push(SideLine {
                    lineno: line.old_lineno.map(|n| n.to_string()).unwrap_or_default(),
                    content, changed: true,
                }),
                '+' => adds.push(SideLine {
                    lineno: line.new_lineno.map(|n| n.to_string()).unwrap_or_default(),
                    content, changed: false,
                }),
                _ => {
                    flush(&mut rows, &mut dels, &mut adds);
                    let sl = SideLine {
                        lineno: line.old_lineno.map(|n| n.to_string()).unwrap_or_default(),
                        content: content.clone(), changed: false,
                    };
                    let sl2 = SideLine {
                        lineno: line.new_lineno.map(|n| n.to_string()).unwrap_or_default(),
                        content, changed: false,
                    };
                    rows.push(SplitRow::Pair { left: Some(sl), right: Some(sl2) });
                }
            }
        }
        flush(&mut rows, &mut dels, &mut adds);
    }
    rows
}
