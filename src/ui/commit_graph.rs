use egui::{Color32, FontId, Key, Pos2, Rect, Sense, Stroke, Ui, Vec2};

use crate::git::operations::load_commit_detail;
use crate::state::SharedState;
use crate::theme;
use crate::ui::widgets;

// Layout
const ROW_H:        f32 = 36.0;
const GRAPH_W:      f32 = 170.0; // fixed graph column width
const LANE_W:       f32 = 18.0;
const DOT_R:        f32 = 6.0;
const DOT_R_SEL:    f32 = 8.0;
const GRAPH_LEFT:   f32 = 10.0;
const HEADER_H:     f32 = 28.0;
// Right-side fixed columns
const COL_AUTHOR:   f32 = 140.0;
const COL_TIME:     f32 = 60.0;
const COL_HASH:     f32 = 62.0;

pub struct CommitGraphView {
    state: SharedState,
    last_selected: Option<String>,
}

impl CommitGraphView {
    pub fn new(state: SharedState) -> Self {
        Self { state, last_selected: None }
    }

    pub fn show(&mut self, ui: &mut Ui) {
        let (graph_opt, selected, head_branch, show_all_branches, search_active, search_query) = {
            let s = self.state.lock().unwrap();
            (s.commit_graph.clone(), s.selected_commit.clone(), s.head_branch.clone().unwrap_or_default(),
             s.show_all_branches, s.search_active, s.search_query.clone())
        };

        ui.painter().rect_filled(ui.max_rect(), 0.0, theme::BG_APP);

        let Some(graph) = graph_opt else {
            ui.centered_and_justified(|ui| {
                ui.label(egui::RichText::new("Open a repository to view commits.").color(theme::TEXT_MUTED).size(14.0));
            });
            return;
        };

        let avail = ui.available_width();

        // ── Search bar ────────────────────────────────────────────────────
        let want_search = ui.input(|i| i.key_pressed(Key::F) && i.modifiers.ctrl);
        if want_search {
            let mut s = self.state.lock().unwrap();
            s.search_active = !s.search_active;
            if !s.search_active { s.search_query.clear(); }
        }

        let search_h = if search_active { 34.0 } else { 0.0 };
        let mut new_query = search_query.clone();
        let mut close_search = false;

        if search_active {
            let bar_rect = Rect::from_min_size(ui.min_rect().min, Vec2::new(avail, search_h));
            ui.painter().rect_filled(bar_rect, 0.0, theme::BG_PANEL);
            ui.painter().line_segment([bar_rect.left_bottom(), bar_rect.right_bottom()], Stroke::new(1.0, theme::BORDER));
            ui.allocate_ui_at_rect(bar_rect, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add_space(10.0);
                    ui.label(egui::RichText::new("⌕").size(13.0).color(theme::TEXT_MUTED));
                    ui.add_space(6.0);
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut new_query)
                            .desired_width(300.0)
                            .hint_text("Search commits…  (message, author, SHA, branch)")
                            .font(FontId::proportional(12.5)),
                    );
                    r.request_focus();
                    if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Escape)) { close_search = true; }
                    if !new_query.is_empty() {
                        let q = new_query.to_lowercase();
                        let n = graph.nodes.iter().filter(|n| node_matches(n, &q)).count();
                        ui.add_space(8.0);
                        ui.label(egui::RichText::new(format!("{} match{}", n, if n==1{""} else {"es"})).color(theme::TEXT_MUTED).size(11.0));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(8.0);
                        if ui.small_button(egui::RichText::new("✕").color(theme::TEXT_MUTED)).clicked() { close_search = true; }
                    });
                });
            });
            if new_query != search_query { self.state.lock().unwrap().search_query = new_query.clone(); }
            if close_search || ui.input(|i| i.key_pressed(Key::Escape)) {
                let mut s = self.state.lock().unwrap();
                s.search_active = false; s.search_query.clear();
            }
        }

        // ── Column headers ────────────────────────────────────────────────
        let header_top = ui.min_rect().min.y + search_h;
        let header_rect = Rect::from_min_size(Pos2::new(ui.min_rect().min.x, header_top), Vec2::new(avail, HEADER_H));
        ui.painter().rect_filled(header_rect, 0.0, theme::BG_PANEL_ALT);
        ui.painter().line_segment([header_rect.left_bottom(), header_rect.right_bottom()], Stroke::new(1.0, theme::BORDER));

        let right_fixed = COL_AUTHOR + COL_TIME + COL_HASH + 24.0;
        let msg_start = header_rect.min.x + GRAPH_W;
        let msg_end   = header_rect.max.x - right_fixed - 8.0;

        let hcol = Color32::from_rgb(90, 100, 120);
        let header_font = FontId::proportional(11.0);
        ui.painter().text(Pos2::new(header_rect.min.x + GRAPH_LEFT, header_rect.center().y), egui::Align2::LEFT_CENTER, "Graph", header_font.clone(), hcol);
        ui.painter().text(Pos2::new(msg_start + 4.0, header_rect.center().y), egui::Align2::LEFT_CENTER, "Commit", header_font.clone(), hcol);
        let author_x = header_rect.max.x - right_fixed;
        ui.painter().text(Pos2::new(author_x, header_rect.center().y), egui::Align2::LEFT_CENTER, "Author", header_font.clone(), hcol);
        let time_x = author_x + COL_AUTHOR + 4.0;
        ui.painter().text(Pos2::new(time_x, header_rect.center().y), egui::Align2::LEFT_CENTER, "Time", header_font.clone(), hcol);
        let hash_x = time_x + COL_TIME + 4.0;
        ui.painter().text(Pos2::new(hash_x, header_rect.center().y), egui::Align2::LEFT_CENTER, "Hash", header_font.clone(), hcol);

        // ── Scroll area (graph + list) ────────────────────────────────────
        let scroll_start = header_top + HEADER_H;
        let scroll_rect = Rect::from_min_max(Pos2::new(ui.min_rect().min.x, scroll_start), ui.max_rect().max);

        // Filter to current branch only when show_all_branches is false
        let nodes: Vec<_> = if show_all_branches {
            graph.nodes.clone()
        } else {
            // Keep commits reachable from HEAD (refs containing head_branch)
            let mut keep = std::collections::HashSet::new();
            for node in &graph.nodes {
                if node.refs.iter().any(|r| r == &head_branch) {
                    keep.insert(node.oid.clone());
                    break;
                }
            }
            // Walk ancestry: keep nodes whose OID is reachable
            // Simple approach: keep nodes up to and including where head_branch ref appears
            let mut reached_head = false;
            graph.nodes.iter().cloned().filter(|n| {
                if !reached_head {
                    if n.refs.iter().any(|r| r == &head_branch) { reached_head = true; }
                    true
                } else {
                    // Keep commits that are ancestors (no branch ref from another tip)
                    !n.refs.iter().any(|r| r != &head_branch && !r.starts_with("origin/") && !r.contains('/'))
                }
            }).collect()
        };
        let edges  = graph.edges.clone();
        let total_h = nodes.len() as f32 * ROW_H;

        let q_lower = if search_active && !search_query.is_empty() {
            Some(search_query.to_lowercase())
        } else {
            None
        };

        let mut new_selection: Option<Option<String>> = None;

        // Arrow-key navigation
        {
            let (up, down) = ui.input(|i| (i.key_pressed(Key::ArrowUp), i.key_pressed(Key::ArrowDown)));
            if up || down {
                let cur = selected.as_ref().and_then(|oid| nodes.iter().position(|n| &n.oid == oid));
                let nxt = match (cur, up) {
                    (Some(i), true)  => i.saturating_sub(1),
                    (Some(i), false) => (i + 1).min(nodes.len().saturating_sub(1)),
                    (None, true)     => nodes.len().saturating_sub(1),
                    (None, false)    => 0,
                };
                if !nodes.is_empty() { new_selection = Some(Some(nodes[nxt].oid.clone())); }
            }
        }

        ui.allocate_ui_at_rect(scroll_rect, |ui| {
            egui::ScrollArea::vertical()
                .id_source("commit_log_scroll")
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    let (resp, painter) = ui.allocate_painter(Vec2::new(avail, total_h), Sense::hover());
                    let origin = resp.rect.min;
                    let clip   = painter.clip_rect();

                    let first = ((clip.top() - origin.y) / ROW_H).floor().max(0.0) as usize;
                    let last  = (((clip.bottom() - origin.y) / ROW_H).ceil() as usize + 1).min(nodes.len());

                    // ── Draw edges (cubic bezier) ─────────────────────
                    for edge in &edges {
                        let from_y = origin.y + edge.from_row as f32 * ROW_H + ROW_H / 2.0;
                        let to_y   = origin.y + edge.to_row   as f32 * ROW_H + ROW_H / 2.0;
                        if to_y < clip.top() - ROW_H || from_y > clip.bottom() + ROW_H { continue; }

                        let color  = theme::lane_color(edge.from_lane);
                        let from_x = origin.x + GRAPH_LEFT + edge.from_lane as f32 * LANE_W;
                        let to_x   = origin.x + GRAPH_LEFT + edge.to_lane   as f32 * LANE_W;

                        if edge.from_lane == edge.to_lane {
                            painter.line_segment(
                                [Pos2::new(from_x, from_y), Pos2::new(to_x, to_y)],
                                Stroke::new(2.0, color),
                            );
                        } else {
                            // Cubic bezier for smooth branching curves
                            let dy = (to_y - from_y).abs();
                            let strength = (dy * 0.45).max(12.0).min(60.0);
                            let shape = egui::epaint::CubicBezierShape::from_points_stroke(
                                [
                                    Pos2::new(from_x, from_y + DOT_R + 1.0),
                                    Pos2::new(from_x, from_y + strength),
                                    Pos2::new(to_x,   to_y - strength),
                                    Pos2::new(to_x,   to_y - DOT_R - 1.0),
                                ],
                                false,
                                Color32::TRANSPARENT,
                                Stroke::new(2.0, color),
                            );
                            painter.add(egui::Shape::CubicBezier(shape));
                        }
                    }

                    // ── Draw rows ─────────────────────────────────────
                    for i in first..last {
                        let node = &nodes[i];
                        let row_top  = origin.y + i as f32 * ROW_H;
                        let row_rect = Rect::from_min_size(Pos2::new(origin.x, row_top), Vec2::new(avail, ROW_H));
                        let cy = row_top + ROW_H / 2.0;

                        let is_sel = selected.as_deref() == Some(&node.oid);
                        let row_resp = ui.interact(row_rect, egui::Id::new(("row", i)), Sense::click());
                        let hovered  = row_resp.hovered();

                        let is_match = q_lower.as_ref().map_or(true, |q| node_matches(node, q));
                        let dim = q_lower.is_some() && !is_match;

                        let row_bg = if is_sel     { theme::BG_SELECTED }
                                     else if hovered { theme::BG_HOVER }
                                     else             { Color32::TRANSPARENT };
                        if row_bg != Color32::TRANSPARENT { painter.rect_filled(row_rect, 0.0, row_bg); }
                        if dim { painter.rect_filled(row_rect, 0.0, Color32::from_black_alpha(110)); }

                        if row_resp.clicked() { new_selection = Some(Some(node.oid.clone())); }

                        let fade = if dim { 0.25 } else { 1.0 };
                        let lane_col = theme::lane_color(node.lane).gamma_multiply(fade);
                        let dot_x = origin.x + GRAPH_LEFT + node.lane as f32 * LANE_W;

                        // Node circle
                        painter.circle_filled(Pos2::new(dot_x, cy), DOT_R, lane_col);
                        painter.circle_stroke(Pos2::new(dot_x, cy), DOT_R, Stroke::new(1.5, theme::BG_APP));
                        if is_sel {
                            painter.circle_stroke(Pos2::new(dot_x, cy), DOT_R_SEL, Stroke::new(2.0, theme::TEXT_PRIMARY));
                        }

                        // Right columns
                        let right = origin.x + avail - 8.0;
                        let hash_right   = right;
                        let time_right   = hash_right - COL_HASH - 8.0;
                        let author_right = time_right - COL_TIME - 8.0;

                        let dim_col = |c: Color32| c.gamma_multiply(fade);

                        // Hash
                        painter.text(
                            Pos2::new(hash_right, cy), egui::Align2::RIGHT_CENTER,
                            &node.oid[..7.min(node.oid.len())],
                            FontId::monospace(11.0), dim_col(theme::TEXT_MUTED),
                        );
                        // Time
                        painter.text(
                            Pos2::new(time_right, cy), egui::Align2::RIGHT_CENTER,
                            &theme::relative_time(node.time),
                            FontId::proportional(11.5), dim_col(theme::TEXT_MUTED),
                        );
                        // Author avatar + name
                        let av_cx = author_right - COL_AUTHOR + 12.0;
                        widgets::avatar(&painter, Pos2::new(av_cx, cy), 10.0, &node.author);
                        let author_text_x = av_cx + 14.0;
                        painter.with_clip_rect(Rect::from_min_max(
                            Pos2::new(author_text_x, row_top),
                            Pos2::new(author_right, row_top + ROW_H),
                        )).text(
                            Pos2::new(author_text_x, cy), egui::Align2::LEFT_CENTER,
                            &node.author,
                            FontId::proportional(12.0), dim_col(theme::TEXT_SECONDARY),
                        );

                        // Ref label pills (branch labels)
                        let mut label_x = origin.x + GRAPH_W;
                        for ref_name in node.refs.iter().take(3) {
                            label_x = widgets::branch_label(&painter, Pos2::new(label_x, cy), ref_name, row_top, ROW_H);
                            label_x += 4.0;
                        }
                        if node.refs.len() > 3 {
                            let extra = format!("+{}", node.refs.len() - 3);
                            painter.text(
                                Pos2::new(label_x + 4.0, cy), egui::Align2::LEFT_CENTER,
                                &extra, FontId::proportional(10.5), dim_col(theme::TEXT_MUTED),
                            );
                            label_x += 30.0;
                        }

                        // Commit summary
                        let summary_start = label_x + 4.0;
                        let summary_end   = author_right - COL_AUTHOR - 8.0;
                        if summary_end > summary_start + 20.0 {
                            let tc = if is_sel { theme::TEXT_PRIMARY } else { dim_col(Color32::from_rgb(200, 207, 218)) };
                            painter.with_clip_rect(Rect::from_min_max(
                                Pos2::new(summary_start, row_top),
                                Pos2::new(summary_end, row_top + ROW_H),
                            )).text(
                                Pos2::new(summary_start, cy), egui::Align2::LEFT_CENTER,
                                &node.summary,
                                FontId::proportional(13.0), tc,
                            );
                        }
                    }

                    // Context menu
                    resp.context_menu(|ui| {
                        if let Some(ref oid) = selected {
                            let short = &oid[..7.min(oid.len())];
                            if ui.button(format!("Copy SHA  {}", short)).clicked() {
                                ui.output_mut(|o| o.copied_text = oid.clone()); ui.close_menu();
                            }
                            ui.separator();
                            if ui.button("Cherry-pick").clicked() {
                                crate::git::branches::cherry_pick(oid, self.state.clone()); ui.close_menu();
                            }
                            if ui.button("Revert Commit").clicked() {
                                crate::git::branches::revert_commit(oid, self.state.clone()); ui.close_menu();
                            }
                            if ui.button("Interactive Rebase from here…").clicked() {
                                self.state.lock().unwrap().pending_open_rebase = Some(oid.clone()); ui.close_menu();
                            }
                            ui.separator();
                            if ui.button("Create Branch Here").clicked() {
                                self.state.lock().unwrap().modal = crate::state::ModalKind::CreateBranch { from_oid: Some(oid.clone()) }; ui.close_menu();
                            }
                            if ui.button("Create Tag Here").clicked() {
                                self.state.lock().unwrap().modal = crate::state::ModalKind::CreateTag { from_oid: Some(oid.clone()) }; ui.close_menu();
                            }
                            ui.separator();
                        }
                        if ui.button("Deselect").clicked() {
                            new_selection = Some(None); ui.close_menu();
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
                if new_sel.is_none() { s.selected_commit_detail = None; }
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

fn node_matches(node: &crate::git::graph::CommitNode, q: &str) -> bool {
    node.summary.to_lowercase().contains(q)
        || node.author.to_lowercase().contains(q)
        || node.oid.starts_with(q)
        || node.refs.iter().any(|r| r.to_lowercase().contains(q))
}
