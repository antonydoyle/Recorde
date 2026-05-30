use egui::{Color32, FontId, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2};

use crate::git::graph::CommitGraph;
use crate::git::operations::load_commit_detail;
use crate::state::SharedState;
use crate::theme;

// Layout constants
const ROW_HEIGHT: f32 = 28.0;
const LANE_WIDTH: f32 = 14.0;
const DOT_RADIUS: f32 = 4.5;
const GRAPH_PAD_LEFT: f32 = 8.0;
const GRAPH_PAD_RIGHT: f32 = 10.0;
// Right-side column widths (right-aligned)
const COL_SHA: f32 = 58.0;
const COL_TIME: f32 = 64.0;
const COL_AUTHOR: f32 = 120.0;
const RIGHT_COLS: f32 = COL_SHA + COL_TIME + COL_AUTHOR + 24.0; // gaps

pub struct CommitGraphView {
    state: SharedState,
    last_selected: Option<String>,
}

impl CommitGraphView {
    pub fn new(state: SharedState) -> Self {
        Self { state, last_selected: None }
    }

    pub fn show(&mut self, ui: &mut Ui) {
        let state_snap = {
            let s = self.state.lock().unwrap();
            (s.commit_graph.clone(), s.selected_commit.clone())
        };

        let (Some(graph), selected) = state_snap else {
            ui.centered_and_justified(|ui| {
                ui.label(
                    egui::RichText::new("Open a repository to view commits.")
                        .color(theme::TEXT_MUTED),
                );
            });
            return;
        };

        // Background fill
        ui.painter().rect_filled(ui.max_rect(), 0.0, theme::BG_BASE);

        let graph_area_width =
            GRAPH_PAD_LEFT + (graph.max_lane + 1) as f32 * LANE_WIDTH + GRAPH_PAD_RIGHT;
        let total_height = graph.nodes.len() as f32 * ROW_HEIGHT;
        let avail_width = ui.available_width();

        let mut new_selection: Option<Option<String>> = None; // Some(Some(oid)) = select, Some(None) = deselect

        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                let (resp, painter) =
                    ui.allocate_painter(Vec2::new(avail_width, total_height), Sense::hover());

                let origin = resp.rect.min;
                let clip = painter.clip_rect();

                // Visible row range
                let first_row =
                    ((clip.top() - origin.y) / ROW_HEIGHT).floor().max(0.0) as usize;
                let last_row = (((clip.bottom() - origin.y) / ROW_HEIGHT).ceil() as usize + 1)
                    .min(graph.nodes.len());

                // --- Draw edges ---
                for edge in &graph.edges {
                    let from_y = origin.y + edge.from_row as f32 * ROW_HEIGHT + ROW_HEIGHT / 2.0;
                    let to_y = origin.y + edge.to_row as f32 * ROW_HEIGHT + ROW_HEIGHT / 2.0;

                    // Cull edges entirely outside the viewport
                    if to_y < clip.top() - ROW_HEIGHT || from_y > clip.bottom() + ROW_HEIGHT {
                        continue;
                    }

                    let color = theme::lane_color(edge.from_lane);
                    let from_x = origin.x + GRAPH_PAD_LEFT + edge.from_lane as f32 * LANE_WIDTH;
                    let to_x = origin.x + GRAPH_PAD_LEFT + edge.to_lane as f32 * LANE_WIDTH;

                    if edge.from_lane == edge.to_lane {
                        // Straight vertical line
                        painter.line_segment(
                            [Pos2::new(from_x, from_y), Pos2::new(to_x, to_y)],
                            Stroke::new(1.5, color),
                        );
                    } else {
                        // Elbow connector: vertical → diagonal
                        let mid_y = from_y + (to_y - from_y) * 0.6;
                        painter.line_segment(
                            [Pos2::new(from_x, from_y + DOT_RADIUS + 1.0), Pos2::new(from_x, mid_y)],
                            Stroke::new(1.5, color),
                        );
                        painter.line_segment(
                            [Pos2::new(from_x, mid_y), Pos2::new(to_x, to_y - DOT_RADIUS - 1.0)],
                            Stroke::new(1.5, color),
                        );
                    }
                }

                // --- Draw rows ---
                for i in first_row..last_row {
                    let node = &graph.nodes[i];
                    let row_top = origin.y + i as f32 * ROW_HEIGHT;
                    let row_rect = Rect::from_min_size(
                        Pos2::new(origin.x, row_top),
                        Vec2::new(avail_width, ROW_HEIGHT),
                    );
                    let cy = row_top + ROW_HEIGHT / 2.0;

                    // Row hover / select detection
                    let row_resp = ui.interact(row_rect, egui::Id::new(("row", i)), Sense::click());
                    let is_selected = selected.as_deref() == Some(&node.oid);
                    let is_hovered = row_resp.hovered();

                    let row_bg = if is_selected {
                        theme::BG_SELECTED
                    } else if is_hovered {
                        theme::BG_ELEVATED
                    } else {
                        Color32::TRANSPARENT
                    };

                    if row_bg != Color32::TRANSPARENT {
                        painter.rect_filled(row_rect, 0.0, row_bg);
                    }

                    if row_resp.clicked() {
                        new_selection = Some(Some(node.oid.clone()));
                    }

                    let lane_color = theme::lane_color(node.lane);
                    let dot_x = origin.x + GRAPH_PAD_LEFT + node.lane as f32 * LANE_WIDTH;

                    // Commit dot
                    painter.circle_filled(Pos2::new(dot_x, cy), DOT_RADIUS, lane_color);
                    painter.circle_stroke(
                        Pos2::new(dot_x, cy),
                        DOT_RADIUS,
                        Stroke::new(1.2, Color32::from_white_alpha(80)),
                    );

                    // Right column x positions (right-aligned)
                    let right = origin.x + avail_width - 8.0;
                    let sha_x = right - COL_SHA;
                    let time_x = sha_x - 8.0 - COL_TIME;
                    let author_x = time_x - 8.0 - COL_AUTHOR;

                    // SHA (7 chars)
                    let short_sha = &node.oid[..7.min(node.oid.len())];
                    painter.text(
                        Pos2::new(right, cy),
                        egui::Align2::RIGHT_CENTER,
                        short_sha,
                        FontId::monospace(11.0),
                        theme::TEXT_MUTED,
                    );

                    // Relative time
                    let time_str = theme::relative_time(node.time);
                    painter.text(
                        Pos2::new(time_x + COL_TIME, cy),
                        egui::Align2::RIGHT_CENTER,
                        &time_str,
                        FontId::proportional(11.0),
                        theme::TEXT_MUTED,
                    );

                    // Author avatar (circle with initials)
                    let avatar_r = 9.0;
                    let avatar_x = author_x + avatar_r;
                    let avatar_color = theme::author_color(&node.author);
                    painter.circle_filled(Pos2::new(avatar_x, cy), avatar_r, avatar_color);
                    let initials = theme::author_initials(&node.author);
                    painter.text(
                        Pos2::new(avatar_x, cy),
                        egui::Align2::CENTER_CENTER,
                        &initials,
                        FontId::proportional(7.5),
                        Color32::WHITE,
                    );

                    // Author name (after avatar, before time)
                    let author_name_x = avatar_x + avatar_r + 4.0;
                    let author_clip = Rect::from_min_max(
                        Pos2::new(author_name_x, row_top),
                        Pos2::new(time_x - 4.0, row_top + ROW_HEIGHT),
                    );
                    painter.with_clip_rect(author_clip).text(
                        Pos2::new(author_name_x, cy),
                        egui::Align2::LEFT_CENTER,
                        &node.author,
                        FontId::proportional(11.0),
                        theme::TEXT_SECONDARY,
                    );

                    // Ref labels
                    let text_start_x = dot_x + DOT_RADIUS + 6.0;
                    let mut label_x = text_start_x;
                    for ref_name in node.refs.iter().take(3) {
                        label_x = draw_ref_label(
                            &painter,
                            Pos2::new(label_x, cy),
                            ref_name,
                            row_top,
                            ROW_HEIGHT,
                        );
                        label_x += 4.0;
                    }

                    // Commit summary (truncated at author column)
                    let summary_start = label_x + 2.0;
                    let summary_end = author_x - 8.0;
                    if summary_end > summary_start + 20.0 {
                        let summary_clip = Rect::from_min_max(
                            Pos2::new(summary_start, row_top),
                            Pos2::new(summary_end, row_top + ROW_HEIGHT),
                        );
                        painter.with_clip_rect(summary_clip).text(
                            Pos2::new(summary_start, cy),
                            egui::Align2::LEFT_CENTER,
                            &node.summary,
                            FontId::proportional(13.0),
                            if is_selected { theme::TEXT_PRIMARY } else { Color32::from_rgb(0xCC, 0xCE, 0xD4) },
                        );
                    }
                }

                // Context menu on right-click
                resp.context_menu(|ui| {
                    if let Some(ref oid) = selected {
                        let short = &oid[..7.min(oid.len())];
                        if ui.button(format!("Copy SHA  {}", short)).clicked() {
                            ui.output_mut(|o| o.copied_text = oid.clone());
                            ui.close_menu();
                        }
                        ui.separator();
                    }
                    if ui.button("Deselect").clicked() {
                        new_selection = Some(None);
                        ui.close_menu();
                    }
                });
            });

        // Apply selection change and trigger detail load
        if let Some(new_sel) = new_selection {
            let trigger_load = new_sel.clone();
            {
                let mut s = self.state.lock().unwrap();
                s.selected_commit = new_sel.clone();
                if new_sel.is_none() {
                    s.selected_commit_detail = None;
                }
            }
            if let Some(oid) = trigger_load {
                if self.last_selected.as_deref() != Some(&oid) {
                    self.last_selected = Some(oid.clone());
                    load_commit_detail(&oid, self.state.clone());
                }
            } else {
                self.last_selected = None;
            }
        }
    }
}

/// Draw a branch/tag ref label badge. Returns the right edge x of the badge.
fn draw_ref_label(
    painter: &egui::Painter,
    pos: Pos2,
    name: &str,
    row_top: f32,
    row_height: f32,
) -> f32 {
    let font = FontId::proportional(10.5);
    let galley = painter.layout_no_wrap(name.to_string(), font.clone(), Color32::WHITE);
    let pad_x = 5.0;
    let pad_y = 2.0;
    let w = galley.size().x + pad_x * 2.0;
    let h = (galley.size().y + pad_y * 2.0).min(row_height - 4.0);
    let rect = Rect::from_min_size(
        Pos2::new(pos.x, row_top + (row_height - h) / 2.0),
        Vec2::new(w, h),
    );

    // Color by ref type: remote → muted blue, tag → purple, local → blue
    let bg = if name.contains('/') {
        Color32::from_rgb(0x2A, 0x3A, 0x5A) // remote
    } else if name.starts_with('v') && name.chars().nth(1).map_or(false, |c| c.is_ascii_digit()) {
        Color32::from_rgb(0x3A, 0x2A, 0x5A) // tag-like
    } else {
        Color32::from_rgb(0x1E, 0x3A, 0x6E) // local branch
    };

    painter.rect_filled(rect, 3.0, bg);
    painter.rect_stroke(rect, 3.0, Stroke::new(0.5, theme::ACCENT_BLUE));
    painter.galley(
        Pos2::new(rect.min.x + pad_x, rect.min.y + (h - galley.size().y) / 2.0),
        galley,
        Color32::WHITE,
    );

    rect.max.x
}
