use egui::{Color32, FontId, Key, Modifiers, Pos2, Rect, Sense, Stroke, Ui, Vec2};

use crate::git::operations::load_commit_detail;
use crate::state::SharedState;
use crate::theme;

// Layout constants
const ROW_HEIGHT: f32 = 28.0;
const LANE_WIDTH: f32 = 14.0;
const DOT_RADIUS: f32 = 4.5;
const GRAPH_PAD_LEFT: f32 = 8.0;
const GRAPH_PAD_RIGHT: f32 = 10.0;
const COL_SHA: f32 = 58.0;
const COL_TIME: f32 = 64.0;
const COL_AUTHOR: f32 = 120.0;
const RIGHT_COLS: f32 = COL_SHA + COL_TIME + COL_AUTHOR + 24.0;

pub struct CommitGraphView {
    state: SharedState,
    last_selected: Option<String>,
}

impl CommitGraphView {
    pub fn new(state: SharedState) -> Self {
        Self { state, last_selected: None }
    }

    pub fn show(&mut self, ui: &mut Ui) {
        let (graph_opt, selected, search_active, search_query) = {
            let s = self.state.lock().unwrap();
            (
                s.commit_graph.clone(),
                s.selected_commit.clone(),
                s.search_active,
                s.search_query.clone(),
            )
        };

        ui.painter().rect_filled(ui.max_rect(), 0.0, theme::BG_BASE);

        let Some(graph) = graph_opt else {
            ui.centered_and_justified(|ui| {
                ui.label(egui::RichText::new("Open a repository to view commits.").color(theme::TEXT_MUTED));
            });
            return;
        };

        // ── Search bar ────────────────────────────────────────────────────────
        // Toggle with Ctrl+F; close with Escape.
        let want_search = ui.input(|i| i.key_pressed(Key::F) && i.modifiers == Modifiers::COMMAND);
        if want_search {
            let mut s = self.state.lock().unwrap();
            s.search_active = !s.search_active;
            if !s.search_active {
                s.search_query.clear();
            }
        }

        let search_bar_h = if search_active { 32.0 } else { 0.0 };
        let mut new_query = search_query.clone();
        let mut close_search = false;

        if search_active {
            let bar_rect = Rect::from_min_size(
                ui.min_rect().min,
                Vec2::new(ui.available_width(), search_bar_h),
            );
            ui.painter().rect_filled(bar_rect, 0.0, theme::BG_ELEVATED);
            ui.painter().line_segment(
                [Pos2::new(bar_rect.min.x, bar_rect.max.y), Pos2::new(bar_rect.max.x, bar_rect.max.y)],
                Stroke::new(1.0, theme::BORDER),
            );
            ui.allocate_ui_at_rect(bar_rect, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add_space(10.0);
                    ui.label(egui::RichText::new("🔍").size(13.0).color(theme::TEXT_MUTED));
                    ui.add_space(4.0);
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut new_query)
                            .desired_width(280.0)
                            .hint_text("Search commits…  (message, author, SHA, branch)")
                            .font(FontId::proportional(12.5)),
                    );
                    resp.request_focus();
                    if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Escape)) {
                        close_search = true;
                    }

                    // Match count
                    if !new_query.is_empty() {
                        let q = new_query.to_lowercase();
                        let count = graph.nodes.iter().filter(|n| node_matches(n, &q)).count();
                        ui.add_space(8.0);
                        ui.label(
                            egui::RichText::new(format!("{} match{}", count, if count == 1 { "" } else { "es" }))
                                .color(theme::TEXT_MUTED)
                                .size(11.0),
                        );
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(8.0);
                        if ui.small_button(egui::RichText::new("✕").color(theme::TEXT_MUTED)).clicked() {
                            close_search = true;
                        }
                    });
                });
            });

            if new_query != search_query {
                self.state.lock().unwrap().search_query = new_query.clone();
            }
            if close_search || ui.input(|i| i.key_pressed(Key::Escape)) {
                let mut s = self.state.lock().unwrap();
                s.search_active = false;
                s.search_query.clear();
            }
        }

        // ── Graph area ────────────────────────────────────────────────────────
        let graph_area_rect = Rect::from_min_max(
            Pos2::new(ui.min_rect().min.x, ui.min_rect().min.y + search_bar_h),
            ui.max_rect().max,
        );

        let nodes = graph.nodes.clone();
        let edges = graph.edges.clone();
        let max_lane = graph.max_lane;
        let avail_width = graph_area_rect.width();
        let total_height = nodes.len() as f32 * ROW_HEIGHT;

        let q_lower = if search_active && !search_query.is_empty() {
            Some(search_query.to_lowercase())
        } else {
            None
        };

        // Keyboard navigation: arrow keys move selection
        let mut new_selection: Option<Option<String>> = None;
        if ui.memory(|m| m.focused().is_none()) || true {
            let (up, down) = ui.input(|i| (
                i.key_pressed(Key::ArrowUp),
                i.key_pressed(Key::ArrowDown),
            ));
            if up || down {
                let current_idx = selected.as_ref()
                    .and_then(|oid| nodes.iter().position(|n| &n.oid == oid));
                let new_idx = match (current_idx, up) {
                    (Some(i), true)  => i.saturating_sub(1),
                    (Some(i), false) => (i + 1).min(nodes.len().saturating_sub(1)),
                    (None, true)     => nodes.len().saturating_sub(1),
                    (None, false)    => 0,
                };
                if !nodes.is_empty() {
                    new_selection = Some(Some(nodes[new_idx].oid.clone()));
                }
            }
        }

        ui.allocate_ui_at_rect(graph_area_rect, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    let (resp, painter) = ui.allocate_painter(
                        Vec2::new(avail_width, total_height),
                        Sense::hover(),
                    );
                    let origin = resp.rect.min;
                    let clip = painter.clip_rect();

                    let first_row = ((clip.top() - origin.y) / ROW_HEIGHT).floor().max(0.0) as usize;
                    let last_row = (((clip.bottom() - origin.y) / ROW_HEIGHT).ceil() as usize + 1).min(nodes.len());

                    // Draw edges
                    for edge in &edges {
                        let from_y = origin.y + edge.from_row as f32 * ROW_HEIGHT + ROW_HEIGHT / 2.0;
                        let to_y   = origin.y + edge.to_row   as f32 * ROW_HEIGHT + ROW_HEIGHT / 2.0;
                        if to_y < clip.top() - ROW_HEIGHT || from_y > clip.bottom() + ROW_HEIGHT {
                            continue;
                        }
                        let color = theme::lane_color(edge.from_lane);
                        let from_x = origin.x + GRAPH_PAD_LEFT + edge.from_lane as f32 * LANE_WIDTH;
                        let to_x   = origin.x + GRAPH_PAD_LEFT + edge.to_lane   as f32 * LANE_WIDTH;

                        if edge.from_lane == edge.to_lane {
                            painter.line_segment(
                                [Pos2::new(from_x, from_y), Pos2::new(to_x, to_y)],
                                Stroke::new(1.5, color),
                            );
                        } else {
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

                    // Draw rows
                    for i in first_row..last_row {
                        let node = &nodes[i];
                        let row_top = origin.y + i as f32 * ROW_HEIGHT;
                        let row_rect = Rect::from_min_size(
                            Pos2::new(origin.x, row_top),
                            Vec2::new(avail_width, ROW_HEIGHT),
                        );
                        let cy = row_top + ROW_HEIGHT / 2.0;

                        let is_selected = selected.as_deref() == Some(&node.oid);
                        let row_resp = ui.interact(row_rect, egui::Id::new(("row", i)), Sense::click());
                        let is_hovered = row_resp.hovered();

                        // Search dimming
                        let is_match = q_lower.as_ref().map_or(true, |q| node_matches(node, q));
                        let dim = q_lower.is_some() && !is_match;

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
                        if dim {
                            painter.rect_filled(row_rect, 0.0, Color32::from_black_alpha(100));
                        }

                        if row_resp.clicked() {
                            new_selection = Some(Some(node.oid.clone()));
                        }

                        let alpha = if dim { 80 } else { 255 };
                        let lane_color = theme::lane_color(node.lane).gamma_multiply(if dim { 0.3 } else { 1.0 });
                        let dot_x = origin.x + GRAPH_PAD_LEFT + node.lane as f32 * LANE_WIDTH;

                        painter.circle_filled(Pos2::new(dot_x, cy), DOT_RADIUS, lane_color);
                        painter.circle_stroke(
                            Pos2::new(dot_x, cy),
                            DOT_RADIUS,
                            Stroke::new(1.2, Color32::from_white_alpha(if dim { 30 } else { 80 })),
                        );

                        let right = origin.x + avail_width - 8.0;
                        let sha_x    = right - COL_SHA;
                        let time_x   = sha_x - 8.0 - COL_TIME;
                        let author_x = time_x - 8.0 - COL_AUTHOR;

                        let muted = Color32::from_rgba_premultiplied(
                            theme::TEXT_MUTED.r(), theme::TEXT_MUTED.g(),
                            theme::TEXT_MUTED.b(), alpha,
                        );
                        let secondary = Color32::from_rgba_premultiplied(
                            theme::TEXT_SECONDARY.r(), theme::TEXT_SECONDARY.g(),
                            theme::TEXT_SECONDARY.b(), alpha,
                        );
                        let primary = Color32::from_rgba_premultiplied(
                            0xCC, 0xCE, 0xD4, alpha,
                        );

                        // SHA
                        painter.text(
                            Pos2::new(right, cy),
                            egui::Align2::RIGHT_CENTER,
                            &node.oid[..7.min(node.oid.len())],
                            FontId::monospace(11.0),
                            muted,
                        );

                        // Time
                        painter.text(
                            Pos2::new(time_x + COL_TIME, cy),
                            egui::Align2::RIGHT_CENTER,
                            &theme::relative_time(node.time),
                            FontId::proportional(11.0),
                            muted,
                        );

                        // Author avatar
                        let avatar_r = 9.0;
                        let avatar_x = author_x + avatar_r;
                        let avatar_color = theme::author_color(&node.author).gamma_multiply(if dim { 0.3 } else { 1.0 });
                        painter.circle_filled(Pos2::new(avatar_x, cy), avatar_r, avatar_color);
                        painter.text(
                            Pos2::new(avatar_x, cy),
                            egui::Align2::CENTER_CENTER,
                            &theme::author_initials(&node.author),
                            FontId::proportional(7.5),
                            Color32::from_white_alpha(alpha),
                        );

                        // Author name
                        let author_name_x = avatar_x + avatar_r + 4.0;
                        painter.with_clip_rect(Rect::from_min_max(
                            Pos2::new(author_name_x, row_top),
                            Pos2::new(time_x - 4.0, row_top + ROW_HEIGHT),
                        )).text(
                            Pos2::new(author_name_x, cy),
                            egui::Align2::LEFT_CENTER,
                            &node.author,
                            FontId::proportional(11.0),
                            secondary,
                        );

                        // Ref labels
                        let text_start_x = dot_x + DOT_RADIUS + 6.0;
                        let mut label_x = text_start_x;
                        for ref_name in node.refs.iter().take(3) {
                            label_x = draw_ref_label(&painter, Pos2::new(label_x, cy), ref_name, row_top, ROW_HEIGHT, dim);
                            label_x += 4.0;
                        }

                        // Summary — highlight query match
                        let summary_start = label_x + 2.0;
                        let summary_end = author_x - 8.0;
                        if summary_end > summary_start + 20.0 {
                            let text_color = if is_selected { theme::TEXT_PRIMARY } else { primary };
                            painter.with_clip_rect(Rect::from_min_max(
                                Pos2::new(summary_start, row_top),
                                Pos2::new(summary_end, row_top + ROW_HEIGHT),
                            )).text(
                                Pos2::new(summary_start, cy),
                                egui::Align2::LEFT_CENTER,
                                &node.summary,
                                FontId::proportional(13.0),
                                text_color,
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
                            if ui.button("Cherry-pick").clicked() {
                                crate::git::branches::cherry_pick(oid, self.state.clone());
                                ui.close_menu();
                            }
                            if ui.button("Create Branch Here").clicked() {
                                self.state.lock().unwrap().modal =
                                    crate::state::ModalKind::CreateBranch { from_oid: Some(oid.clone()) };
                                ui.close_menu();
                            }
                            if ui.button("Create Tag Here").clicked() {
                                crate::git::branches::create_tag("", Some(oid), self.state.clone());
                                // TODO: open a name-input modal — for now tag is created unnamed (will error, handled by ops)
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
        });

        // Apply selection change
        if let Some(new_sel) = new_selection {
            let trigger = new_sel.clone();
            {
                let mut s = self.state.lock().unwrap();
                s.selected_commit = new_sel.clone();
                if new_sel.is_none() {
                    s.selected_commit_detail = None;
                }
            }
            if let Some(oid) = trigger {
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

/// Returns true if the commit node matches the search query.
fn node_matches(node: &crate::git::graph::CommitNode, q: &str) -> bool {
    node.summary.to_lowercase().contains(q)
        || node.author.to_lowercase().contains(q)
        || node.oid.starts_with(q)
        || node.refs.iter().any(|r| r.to_lowercase().contains(q))
}

fn draw_ref_label(
    painter: &egui::Painter,
    pos: Pos2,
    name: &str,
    row_top: f32,
    row_height: f32,
    dim: bool,
) -> f32 {
    let font = FontId::proportional(10.5);
    let galley = painter.layout_no_wrap(name.to_string(), font, Color32::WHITE);
    let pad_x = 5.0;
    let pad_y = 2.0;
    let w = galley.size().x + pad_x * 2.0;
    let h = (galley.size().y + pad_y * 2.0).min(row_height - 4.0);
    let rect = Rect::from_min_size(
        Pos2::new(pos.x, row_top + (row_height - h) / 2.0),
        Vec2::new(w, h),
    );

    let alpha = if dim { 0.2 } else { 1.0 };
    let bg = if name.contains('/') {
        Color32::from_rgb(0x2A, 0x3A, 0x5A)
    } else if name.starts_with('v') && name.chars().nth(1).map_or(false, |c| c.is_ascii_digit()) {
        Color32::from_rgb(0x3A, 0x2A, 0x5A)
    } else {
        Color32::from_rgb(0x1E, 0x3A, 0x6E)
    };

    painter.rect_filled(rect, 3.0, bg.gamma_multiply(alpha));
    painter.rect_stroke(rect, 3.0, Stroke::new(0.5, theme::ACCENT_BLUE.gamma_multiply(alpha)));
    painter.galley(
        Pos2::new(rect.min.x + pad_x, rect.min.y + (h - galley.size().y) / 2.0),
        galley,
        Color32::from_white_alpha(if dim { 60 } else { 255 }),
    );
    rect.max.x
}
