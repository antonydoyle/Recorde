use egui::{Color32, FontId, Key, Pos2, Rect, Sense, Stroke, Ui, Vec2};

use crate::git::operations::load_commit_detail;
use crate::state::SharedState;
use crate::theme;
use crate::ui::widgets;

// Layout
const ROW_H:        f32 = 36.0;
const REFS_W_DEF:   f32 = 170.0;
const REFS_W_MIN:   f32 = 90.0;
const REFS_W_MAX:   f32 = 320.0;
const GRAPH_W_MIN:  f32 = 80.0;
const GRAPH_W_MAX:  f32 = 340.0;
const GRAPH_W_DEF:  f32 = 170.0;
const COMMIT_W_DEF: f32 = 380.0;
const COMMIT_W_MIN: f32 = 180.0;
const TIME_W_DEF:   f32 = 60.0;
const TIME_W_MIN:   f32 = 46.0;
const HASH_W_DEF:   f32 = 62.0;
const HASH_W_MIN:   f32 = 54.0;
const LANE_W:       f32 = 18.0;
const DOT_R:        f32 = 6.0;
const DOT_R_SEL:    f32 = 8.0;
const GRAPH_LEFT:   f32 = 10.0;
const HEADER_H:     f32 = 28.0;

pub struct CommitGraphView {
    state: SharedState,
    last_selected: Option<String>,
    refs_w: f32,
    graph_w: f32,
    commit_w: f32,
    time_w: f32,
    hash_w: f32,
    cols_dirty: bool,
    last_saved_refs_w: f32,
    last_saved_graph_w: f32,
    colorblind: bool,
}

impl CommitGraphView {
    pub fn new(state: SharedState) -> Self {
        Self {
            state,
            last_selected: None,
            refs_w: REFS_W_DEF,
            graph_w: GRAPH_W_DEF,
            commit_w: COMMIT_W_DEF,
            time_w: TIME_W_DEF,
            hash_w: HASH_W_DEF,
            cols_dirty: false,
            last_saved_refs_w: REFS_W_DEF,
            last_saved_graph_w: GRAPH_W_DEF,
            colorblind: false,
        }
    }

    pub fn set_colorblind(&mut self, enabled: bool) {
        self.colorblind = enabled;
    }

    /// Restore saved column widths (called after opening a repo).
    pub fn set_column_widths(&mut self, refs_w: Option<f32>, graph_w: Option<f32>, time_w: Option<f32>, hash_w: Option<f32>) {
        if let Some(w) = refs_w  { self.refs_w  = w.clamp(REFS_W_MIN,  REFS_W_MAX);  }
        if let Some(w) = graph_w { self.graph_w = w.clamp(GRAPH_W_MIN, GRAPH_W_MAX); }
        if let Some(w) = time_w  { self.time_w  = w.clamp(TIME_W_MIN,  200.0); }
        if let Some(w) = hash_w  { self.hash_w  = w.clamp(HASH_W_MIN,  120.0); }
        self.last_saved_refs_w  = self.refs_w;
        self.last_saved_graph_w = self.graph_w;
        self.cols_dirty = false;
    }

    /// Returns column widths if they have changed since last call, then resets the flag.
    pub fn take_dirty_widths(&mut self) -> Option<(f32, f32, f32, f32)> {
        let changed = (self.refs_w - self.last_saved_refs_w).abs() > 0.5
            || (self.graph_w - self.last_saved_graph_w).abs() > 0.5;
        if self.cols_dirty && changed {
            self.cols_dirty = false;
            self.last_saved_refs_w  = self.refs_w;
            self.last_saved_graph_w = self.graph_w;
            Some((self.refs_w, self.graph_w, self.time_w, self.hash_w))
        } else {
            self.cols_dirty = false;
            None
        }
    }

    pub fn show(&mut self, ui: &mut Ui) {
        let (graph_opt, selected, head_branch, show_all_branches, search_active, search_query,
             file_history_path, file_history_oids) = {
            let s = self.state.lock().unwrap();
            (s.commit_graph.clone(), s.selected_commit.clone(), s.head_branch.clone().unwrap_or_default(),
             s.show_all_branches, s.search_active, s.search_query.clone(),
             s.file_history_path.clone(), s.file_history_oids.clone())
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
                            .hint_text("Search… (message, author:, sha:, after:YYYY-MM-DD, before:YYYY-MM-DD)")
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

        // ── File history banner ───────────────────────────────────────────
        let fh_h = if file_history_path.is_some() { 28.0 } else { 0.0 };
        if let Some(ref fp) = file_history_path {
            let banner_rect = Rect::from_min_size(
                Pos2::new(ui.min_rect().min.x, ui.min_rect().min.y + search_h),
                Vec2::new(avail, fh_h),
            );
            ui.painter().rect_filled(banner_rect, 0.0, theme::ACCENT_TEAL.gamma_multiply(0.15));
            ui.painter().line_segment([banner_rect.left_bottom(), banner_rect.right_bottom()], Stroke::new(1.0, theme::BORDER));
            ui.allocate_ui_at_rect(banner_rect, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add_space(10.0);
                    let fname = fp.rsplit('/').next().unwrap_or(fp.as_str());
                    ui.label(egui::RichText::new(format!("File History: {}", fname)).size(11.5).color(theme::ACCENT_TEAL).strong());
                    ui.label(egui::RichText::new(format!("({} commits)", file_history_oids.len())).size(11.0).color(theme::TEXT_MUTED));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(8.0);
                        if ui.small_button(egui::RichText::new("✕ Clear").color(theme::TEXT_MUTED)).clicked() {
                            let mut s = self.state.lock().unwrap();
                            s.file_history_path = None;
                            s.file_history_oids.clear();
                        }
                    });
                });
            });
        }

        // ── Column headers ────────────────────────────────────────────────
        let header_top = ui.min_rect().min.y + search_h + fh_h;
        let header_rect = Rect::from_min_size(Pos2::new(ui.min_rect().min.x, header_top), Vec2::new(avail, HEADER_H));
        ui.painter().rect_filled(header_rect, 0.0, theme::BG_PANEL_ALT);
        ui.painter().line_segment([header_rect.left_bottom(), header_rect.right_bottom()], Stroke::new(1.0, theme::BORDER));

        self.fit_columns(avail);

        let refs_start = header_rect.min.x;
        let graph_start = refs_start + self.refs_w;
        let msg_start = graph_start + self.graph_w;
        let time_x = msg_start + self.commit_w;
        let hash_x = time_x + self.time_w;
        let end_x = hash_x + self.hash_w;

        let hcol = Color32::from_rgb(90, 100, 120);
        let header_font = FontId::proportional(11.0);
        ui.painter().text(Pos2::new(refs_start + 10.0, header_rect.center().y), egui::Align2::LEFT_CENTER, "Branches / Tags", header_font.clone(), hcol);
        ui.painter().text(Pos2::new(graph_start + GRAPH_LEFT, header_rect.center().y), egui::Align2::LEFT_CENTER, "Graph", header_font.clone(), hcol);
        ui.painter().text(Pos2::new(msg_start + 4.0, header_rect.center().y), egui::Align2::LEFT_CENTER, "Commit", header_font.clone(), hcol);
        ui.painter().text(Pos2::new(time_x + 8.0, header_rect.center().y), egui::Align2::LEFT_CENTER, "Time", header_font.clone(), hcol);
        ui.painter().text(Pos2::new(hash_x + 8.0, header_rect.center().y), egui::Align2::LEFT_CENTER, "Hash", header_font.clone(), hcol);

        self.resize_handle(ui, header_rect, graph_start, ColumnDivider::RefsGraph);
        self.resize_handle(ui, header_rect, msg_start, ColumnDivider::GraphCommit);
        self.resize_handle(ui, header_rect, time_x, ColumnDivider::CommitTime);
        self.resize_handle(ui, header_rect, hash_x, ColumnDivider::TimeHash);
        ui.painter().line_segment(
            [Pos2::new(end_x, header_rect.min.y), Pos2::new(end_x, header_rect.max.y)],
            Stroke::new(1.0, theme::BORDER),
        );

        // ── Scroll area (graph + list) ────────────────────────────────────
        let scroll_start = header_top + HEADER_H;
        let scroll_rect = Rect::from_min_max(Pos2::new(ui.min_rect().min.x, scroll_start), ui.max_rect().max);
        let refs_w = self.refs_w;
        let graph_w = self.graph_w;
        let commit_w = self.commit_w;
        let time_w = self.time_w;
        let hash_w = self.hash_w;

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

                    // ── Draw row backgrounds first so graph/text stay above hover/selection.
                    for i in first..last {
                        let node = &nodes[i];
                        let row_top  = origin.y + i as f32 * ROW_H;
                        let row_rect = Rect::from_min_size(Pos2::new(origin.x, row_top), Vec2::new(avail, ROW_H));
                        let row_resp = ui.interact(row_rect, egui::Id::new(("row_bg", i)), Sense::click());
                        let is_sel = selected.as_deref() == Some(&node.oid);
                        let row_bg = if is_sel { theme::BG_SELECTED }
                                     else if row_resp.hovered() { theme::BG_HOVER }
                                     else { Color32::TRANSPARENT };
                        if row_bg != Color32::TRANSPARENT { painter.rect_filled(row_rect, 0.0, row_bg); }

                        let is_match = q_lower.as_ref().map_or(true, |q| node_matches(node, q));
                        let in_file_history = file_history_oids.is_empty() || file_history_oids.contains(&node.oid);
                        let dimmed = (q_lower.is_some() && !is_match) || (!file_history_oids.is_empty() && !in_file_history);
                        if dimmed {
                            painter.rect_filled(row_rect, 0.0, Color32::from_black_alpha(80));
                        }
                    }

                    // ── Draw edges (cubic bezier) ─────────────────────
                    for edge in &edges {
                        let from_y = origin.y + edge.from_row as f32 * ROW_H + ROW_H / 2.0;
                        let to_y   = origin.y + edge.to_row   as f32 * ROW_H + ROW_H / 2.0;
                        if to_y < clip.top() - ROW_H || from_y > clip.bottom() + ROW_H { continue; }

                        let color  = theme::lane_color_for_palette(edge.from_lane, self.colorblind);
                        let from_x = origin.x + refs_w + GRAPH_LEFT + edge.from_lane as f32 * LANE_W;
                        let to_x   = origin.x + refs_w + GRAPH_LEFT + edge.to_lane   as f32 * LANE_W;

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

                        let is_match = q_lower.as_ref().map_or(true, |q| node_matches(node, q));
                        let in_fh = file_history_oids.is_empty() || file_history_oids.contains(&node.oid);
                        let dim = (q_lower.is_some() && !is_match) || (!file_history_oids.is_empty() && !in_fh);

                        if row_resp.clicked() { new_selection = Some(Some(node.oid.clone())); }

                        let fade = if dim { 0.25 } else { 1.0 };
                        let lane_col = theme::lane_color_for_palette(node.lane, self.colorblind).gamma_multiply(fade);
                        let dot_x = origin.x + refs_w + GRAPH_LEFT + node.lane as f32 * LANE_W;

                        // Node circle
                        painter.circle_filled(Pos2::new(dot_x, cy), DOT_R, lane_col);
                        painter.circle_stroke(Pos2::new(dot_x, cy), DOT_R, Stroke::new(1.5, theme::BG_APP));
                        if is_sel {
                            painter.circle_stroke(Pos2::new(dot_x, cy), DOT_R_SEL, Stroke::new(2.0, theme::TEXT_PRIMARY));
                        }

                        // Right columns
                        let msg_start = origin.x + refs_w + graph_w;
                        let time_start = msg_start + commit_w;
                        let hash_start = time_start + time_w;
                        let hash_right = hash_start + hash_w - 8.0;
                        let time_right = time_start + time_w - 8.0;

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
                        // Author indicator attached to the graph, icon-only.
                        let author_x = (dot_x + 18.0).min(origin.x + refs_w + graph_w - 14.0);
                        let author_pos = Pos2::new(author_x, cy);
                        widgets::avatar(&painter, author_pos, 8.5, &node.author);
                        ui.interact(
                            Rect::from_center_size(author_pos, Vec2::splat(22.0)),
                            egui::Id::new(("author_avatar", i)),
                            Sense::hover(),
                        )
                        .on_hover_text(&node.author);

                        draw_ref_column(&painter, origin.x, refs_w, row_top, cy, node, lane_col);

                        // Commit summary
                        let summary_start = msg_start + 8.0;
                        let summary_end   = time_start - 8.0;
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
                            if ui.button("Checkout (detached HEAD)…").clicked() {
                                self.state.lock().unwrap().modal = crate::state::ModalKind::ConfirmDetachedCheckout { oid: oid.clone() }; ui.close_menu();
                            }
                            ui.separator();
                            // ── Phase 7: compare & bisect ─────────────────────
                            let compare_base = self.state.lock().unwrap().compare_base_oid.clone();
                            if let Some(ref base) = compare_base {
                                let base_short = &base[..7.min(base.len())];
                                if oid != base {
                                    if ui.button(format!("Compare with {}…", base_short)).clicked() {
                                        self.state.lock().unwrap().modal = crate::state::ModalKind::Compare {
                                            base_oid: base.clone(), compare_oid: oid.clone(),
                                        };
                                        ui.close_menu();
                                    }
                                }
                                if ui.button("Clear Compare Base").clicked() {
                                    self.state.lock().unwrap().compare_base_oid = None; ui.close_menu();
                                }
                            } else {
                                if ui.button("Set as Compare Base").clicked() {
                                    self.state.lock().unwrap().compare_base_oid = Some(oid.clone()); ui.close_menu();
                                }
                            }
                            ui.separator();
                            if ui.button("Bisect…").clicked() {
                                self.state.lock().unwrap().modal = crate::state::ModalKind::Bisect; ui.close_menu();
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

    fn fit_columns(&mut self, avail: f32) {
        let min_total = REFS_W_MIN + GRAPH_W_MIN + COMMIT_W_MIN + TIME_W_MIN + HASH_W_MIN;
        if avail <= min_total {
            self.refs_w = REFS_W_MIN;
            self.graph_w = GRAPH_W_MIN;
            self.commit_w = COMMIT_W_MIN;
            self.time_w = TIME_W_MIN;
            self.hash_w = HASH_W_MIN;
            return;
        }

        self.refs_w = self.refs_w.clamp(REFS_W_MIN, REFS_W_MAX);
        self.graph_w = self.graph_w.clamp(GRAPH_W_MIN, GRAPH_W_MAX);
        self.time_w = self.time_w.max(TIME_W_MIN);
        self.hash_w = self.hash_w.max(HASH_W_MIN);

        let fixed = self.refs_w + self.graph_w + self.time_w + self.hash_w;
        self.commit_w = (avail - fixed).max(COMMIT_W_MIN);
    }

    fn resize_handle(&mut self, ui: &mut Ui, header_rect: Rect, x: f32, divider: ColumnDivider) {
        let rect = Rect::from_min_size(
            Pos2::new(x - 3.0, header_rect.min.y),
            Vec2::new(6.0, HEADER_H),
        );
        let resp = ui.allocate_rect(rect, Sense::drag());
        if resp.hovered() || resp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeColumn);
            ui.painter().line_segment(
                [Pos2::new(x, header_rect.min.y + 4.0), Pos2::new(x, header_rect.max.y - 4.0)],
                Stroke::new(2.0, theme::ACCENT_TEAL),
            );
        } else {
            ui.painter().line_segment(
                [Pos2::new(x, header_rect.min.y), Pos2::new(x, header_rect.max.y)],
                Stroke::new(1.0, theme::BORDER),
            );
        }

        if resp.dragged() {
            let delta = resp.drag_delta().x;
            match divider {
                ColumnDivider::RefsGraph => {
                    self.refs_w = (self.refs_w + delta).clamp(REFS_W_MIN, REFS_W_MAX);
                    self.graph_w = (self.graph_w - delta).clamp(GRAPH_W_MIN, GRAPH_W_MAX);
                }
                ColumnDivider::GraphCommit => {
                    self.graph_w = (self.graph_w + delta).clamp(GRAPH_W_MIN, GRAPH_W_MAX);
                    self.commit_w = (self.commit_w - delta).max(COMMIT_W_MIN);
                }
                ColumnDivider::CommitTime => {
                    self.commit_w = (self.commit_w + delta).max(COMMIT_W_MIN);
                    self.time_w = (self.time_w - delta).max(TIME_W_MIN);
                }
                ColumnDivider::TimeHash => {
                    self.time_w = (self.time_w + delta).max(TIME_W_MIN);
                    self.hash_w = (self.hash_w - delta).max(HASH_W_MIN);
                }
            }
            self.cols_dirty = true;
        }
    }
}

#[derive(Clone, Copy)]
enum ColumnDivider {
    RefsGraph,
    GraphCommit,
    CommitTime,
    TimeHash,
}

fn node_matches(node: &crate::git::graph::CommitNode, q: &str) -> bool {
    // Prefix-filter syntax: "author:alice", "after:2024-01-01", "before:2024-12-31"
    if let Some(val) = q.strip_prefix("author:") {
        return node.author.to_lowercase().contains(val);
    }
    if let Some(val) = q.strip_prefix("after:") {
        let ts = parse_date(val);
        return ts.map_or(false, |t| node.time >= t);
    }
    if let Some(val) = q.strip_prefix("before:") {
        let ts = parse_date(val);
        return ts.map_or(false, |t| node.time < t);
    }
    if let Some(val) = q.strip_prefix("sha:") {
        return node.oid.starts_with(val);
    }
    // Default: match message, author, SHA prefix, or ref names
    node.summary.to_lowercase().contains(q)
        || node.author.to_lowercase().contains(q)
        || node.oid.starts_with(q)
        || node.refs.iter().any(|r| r.to_lowercase().contains(q))
}

/// Parse "YYYY-MM-DD" into a Unix timestamp (midnight UTC).
fn parse_date(s: &str) -> Option<i64> {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 3 { return None; }
    let y: i64 = parts[0].parse().ok()?;
    let m: i64 = parts[1].parse().ok()?;
    let d: i64 = parts[2].parse().ok()?;
    // Simplified days-since-epoch calculation (close enough for filtering)
    let leap = |yr: i64| yr % 4 == 0 && (yr % 100 != 0 || yr % 400 == 0);
    let days_in_month = [0i64, 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut days = (y - 1970) * 365 + (y - 1969) / 4 - (y - 1901) / 100 + (y - 1601) / 400;
    for mi in 1..m {
        days += days_in_month[mi as usize];
        if mi == 2 && leap(y) { days += 1; }
    }
    days += d - 1;
    Some(days * 86400)
}

fn draw_ref_column(
    painter: &egui::Painter,
    origin_x: f32,
    refs_w: f32,
    row_top: f32,
    cy: f32,
    node: &crate::git::graph::CommitNode,
    branch_color: Color32,
) {
    let clip = Rect::from_min_max(
        Pos2::new(origin_x + 8.0, row_top),
        Pos2::new(origin_x + refs_w - 8.0, row_top + ROW_H),
    );
    let painter = painter.with_clip_rect(clip);
    let mut x = origin_x + 10.0;
    for name in node.refs.iter().take(3) {
        let is_tag = name.starts_with('v') && name.chars().nth(1).map_or(false, |c| c.is_ascii_digit());
        let color = if is_tag { theme::ACCENT_PURPLE } else { branch_color };
        let font = FontId::proportional(10.5);
        let galley = painter.layout_no_wrap(name.to_string(), font, color);
        let width = galley.size().x + 14.0;
        if x + width > origin_x + refs_w - 10.0 {
            break;
        }
        let rect = Rect::from_min_size(
            Pos2::new(x, row_top + 8.0),
            Vec2::new(width, ROW_H - 16.0),
        );
        painter.rect_filled(rect, 4.0, color.gamma_multiply(0.12));
        painter.rect_stroke(rect, 4.0, Stroke::new(0.8, color.gamma_multiply(0.65)));
        painter.galley(
            Pos2::new(rect.min.x + 7.0, cy - galley.size().y / 2.0),
            galley,
            color,
        );
        x = rect.max.x + 4.0;
    }
}
