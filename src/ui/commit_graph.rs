use egui::{Color32, FontId, Key, Pos2, Rect, Sense, Stroke, Ui, Vec2};

use crate::git::operations::load_commit_detail;
use crate::state::SharedState;
use crate::theme;
use crate::ui::widgets;

// Layout
const ROW_H: f32 = 36.0;
const REFS_W_DEF: f32 = 130.0;
const REFS_W_MIN: f32 = 90.0;
const REFS_W_MAX: f32 = 320.0;
const GRAPH_W_MIN: f32 = 80.0;
const GRAPH_W_MAX: f32 = 340.0;
const GRAPH_W_DEF: f32 = 100.0;
const COMMIT_W_DEF: f32 = 380.0;
const COMMIT_W_MIN: f32 = 180.0;
const TIME_W_DEF: f32 = 60.0;
const TIME_W_MIN: f32 = 46.0;
const HASH_W_DEF: f32 = 62.0;
const HASH_W_MIN: f32 = 54.0;
const LANE_W: f32 = 18.0;
const DOT_R: f32 = 6.0;
const DOT_R_SEL: f32 = 8.0;
const GRAPH_LEFT: f32 = 10.0;
const HEADER_H: f32 = 34.0;
const RESIZE_HIT_W: f32 = 14.0;

pub struct CommitGraphView {
    state: SharedState,
    last_selected: Option<String>,
    scroll_to_oid: Option<String>,
    refs_w: f32,
    graph_w: f32,
    commit_w: f32,
    time_w: f32,
    hash_w: f32,
    cols_dirty: bool,
    last_saved_refs_w: f32,
    last_saved_graph_w: f32,
    last_saved_time_w: f32,
    last_saved_hash_w: f32,
    colorblind: bool,
    range_start: String,
    range_end: String,
    ref_query: String,
}

impl CommitGraphView {
    pub fn new(state: SharedState) -> Self {
        Self {
            state,
            last_selected: None,
            scroll_to_oid: None,
            refs_w: REFS_W_DEF,
            graph_w: GRAPH_W_DEF,
            commit_w: COMMIT_W_DEF,
            time_w: TIME_W_DEF,
            hash_w: HASH_W_DEF,
            cols_dirty: false,
            last_saved_refs_w: REFS_W_DEF,
            last_saved_graph_w: GRAPH_W_DEF,
            last_saved_time_w: TIME_W_DEF,
            last_saved_hash_w: HASH_W_DEF,
            colorblind: false,
            range_start: String::new(),
            range_end: String::new(),
            ref_query: String::new(),
        }
    }

    pub fn set_colorblind(&mut self, enabled: bool) {
        self.colorblind = enabled;
    }

    /// Restore saved column widths (called after opening a repo).
    pub fn set_column_widths(
        &mut self,
        refs_w: Option<f32>,
        graph_w: Option<f32>,
        time_w: Option<f32>,
        hash_w: Option<f32>,
    ) {
        if let Some(w) = refs_w {
            self.refs_w = w.clamp(REFS_W_MIN, REFS_W_MAX);
        }
        if let Some(w) = graph_w {
            self.graph_w = w.clamp(GRAPH_W_MIN, GRAPH_W_MAX);
        }
        if let Some(w) = time_w {
            self.time_w = w.clamp(TIME_W_MIN, 200.0);
        }
        if let Some(w) = hash_w {
            self.hash_w = w.clamp(HASH_W_MIN, 120.0);
        }
        self.last_saved_refs_w = self.refs_w;
        self.last_saved_graph_w = self.graph_w;
        self.last_saved_time_w = self.time_w;
        self.last_saved_hash_w = self.hash_w;
        self.cols_dirty = false;
    }

    pub fn reset_column_widths(&mut self) {
        self.refs_w = REFS_W_DEF;
        self.graph_w = GRAPH_W_DEF;
        self.commit_w = COMMIT_W_DEF;
        self.time_w = TIME_W_DEF;
        self.hash_w = HASH_W_DEF;
        self.last_saved_refs_w = REFS_W_DEF;
        self.last_saved_graph_w = GRAPH_W_DEF;
        self.last_saved_time_w = TIME_W_DEF;
        self.last_saved_hash_w = HASH_W_DEF;
        self.cols_dirty = false;
    }

    /// Returns column widths if any changed since last save, then resets the flag.
    pub fn take_dirty_widths(&mut self) -> Option<(f32, f32, f32, f32)> {
        if !self.cols_dirty {
            return None;
        }
        self.cols_dirty = false;
        let changed = (self.refs_w - self.last_saved_refs_w).abs() > 0.5
            || (self.graph_w - self.last_saved_graph_w).abs() > 0.5
            || (self.time_w - self.last_saved_time_w).abs() > 0.5
            || (self.hash_w - self.last_saved_hash_w).abs() > 0.5;
        if changed {
            self.last_saved_refs_w = self.refs_w;
            self.last_saved_graph_w = self.graph_w;
            self.last_saved_time_w = self.time_w;
            self.last_saved_hash_w = self.hash_w;
            Some((self.refs_w, self.graph_w, self.time_w, self.hash_w))
        } else {
            None
        }
    }

    pub fn show(&mut self, ui: &mut Ui) {
        let (
            graph_opt,
            selected,
            search_active,
            search_query,
            file_history_path,
            file_history_oids,
        ) = {
            let s = self.state.lock().unwrap();
            (
                s.commit_graph.clone(),
                s.selected_commit.clone(),
                s.search_active,
                s.search_query.clone(),
                s.file_history_path.clone(),
                s.file_history_oids.clone(),
            )
        };

        ui.painter()
            .rect_filled(ui.max_rect(), 0.0, theme::bg_app());

        let Some(graph) = graph_opt else {
            ui.centered_and_justified(|ui| {
                ui.label(
                    egui::RichText::new("Open a repository to view commits.")
                        .color(theme::text_muted())
                        .size(14.0),
                );
            });
            return;
        };

        let avail = ui.available_width();

        // ── Search bar ────────────────────────────────────────────────────
        let want_search = self.state.lock().unwrap().focused_pane != 4
            && !ui.ctx().wants_keyboard_input()
            && ui.input(|i| i.key_pressed(Key::F) && i.modifiers.command && !i.modifiers.shift);
        if want_search {
            let mut s = self.state.lock().unwrap();
            s.search_active = !s.search_active;
            if !s.search_active {
                s.search_query.clear();
            }
        }

        let search_h = if search_active { 34.0 } else { 0.0 };
        let mut new_query = search_query.clone();
        let mut close_search = false;

        if search_active {
            let bar_rect = Rect::from_min_size(ui.min_rect().min, Vec2::new(avail, search_h));
            ui.painter().rect_filled(bar_rect, 0.0, theme::bg_panel());
            ui.painter().line_segment(
                [bar_rect.left_bottom(), bar_rect.right_bottom()],
                Stroke::new(1.0, theme::border()),
            );
            ui.allocate_ui_at_rect(bar_rect, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add_space(10.0);
                    ui.label(egui::RichText::new("⌕").size(14.0).color(theme::text_muted()));
                    ui.add_space(6.0);
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut new_query)
                            .desired_width(300.0)
                            .margin(egui::Vec2::new(8.0, 5.0))
                            .hint_text("Search… (message, author:, sha:, after:YYYY-MM-DD, before:YYYY-MM-DD)")
                            .font(FontId::proportional(13.0)),
                    );
                    r.request_focus();
                    if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Escape)) { close_search = true; }
                    if !new_query.is_empty() {
                        let q = new_query.to_lowercase();
                        let n = graph.nodes.iter().filter(|n| node_matches(n, &q)).count();
                        ui.add_space(8.0);
                        ui.label(egui::RichText::new(format!("{} match{}", n, if n==1{""} else {"es"})).color(theme::text_muted()).size(12.0));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(8.0);
                        if ui.small_button(egui::RichText::new("✕").color(theme::text_muted())).clicked() { close_search = true; }
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

        // ── File history banner ───────────────────────────────────────────
        let fh_h = if file_history_path.is_some() {
            28.0
        } else {
            0.0
        };
        if let Some(ref fp) = file_history_path {
            let banner_rect = Rect::from_min_size(
                Pos2::new(ui.min_rect().min.x, ui.min_rect().min.y + search_h),
                Vec2::new(avail, fh_h),
            );
            ui.painter()
                .rect_filled(banner_rect, 0.0, theme::accent_teal().gamma_multiply(0.15));
            ui.painter().line_segment(
                [banner_rect.left_bottom(), banner_rect.right_bottom()],
                Stroke::new(1.0, theme::border()),
            );
            ui.allocate_ui_at_rect(banner_rect, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add_space(10.0);
                    let fname = fp.rsplit('/').next().unwrap_or(fp.as_str());
                    ui.label(
                        egui::RichText::new(format!("File History: {}", fname))
                            .size(12.0)
                            .color(theme::accent_teal())
                            .strong(),
                    );
                    ui.label(
                        egui::RichText::new(format!("({} commits)", file_history_oids.len()))
                            .size(12.0)
                            .color(theme::text_muted()),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(8.0);
                        if ui
                            .small_button(egui::RichText::new("✕ Clear").color(theme::text_muted()))
                            .clicked()
                        {
                            let mut s = self.state.lock().unwrap();
                            s.file_history_path = None;
                            s.file_history_oids.clear();
                        }
                    });
                });
            });
        }

        // ── Column headers ────────────────────────────────────────────────
        let navigation_top = ui.min_rect().min.y + search_h + fh_h;
        let navigation_rect = Rect::from_min_size(
            Pos2::new(ui.min_rect().min.x, navigation_top),
            Vec2::new(avail, 42.0),
        );
        ui.allocate_ui_at_rect(navigation_rect, |ui| {
            ui.horizontal(|ui| {
                let (dirty, loading) = { let s = self.state.lock().unwrap(); (s.dirty_count, s.repo_load_path.is_some()) };
                if ui.button(format!("Working changes ({dirty})")).clicked() {
                    let mut s = self.state.lock().unwrap();
                    s.selected_commit = None; s.selected_commit_detail = None; s.current_diff = None; s.focused_pane = 3;
                }
                if ui.button("Jump to HEAD").clicked() {
                    if let Some(oid) = &graph.head_oid {
                        self.scroll_to_oid = Some(oid.clone());
                        {
                            let mut s = self.state.lock().unwrap();
                            s.selected_commit = Some(oid.clone());
                            if !graph.oid_to_row.contains_key(oid) {
                                s.graph_filter = Default::default(); s.show_all_branches = false;
                            }
                        }
                        if !graph.oid_to_row.contains_key(oid) { crate::git::operations::refresh_repo(self.state.clone()); }
                        load_commit_detail(oid, self.state.clone());
                    }
                }
                self.filter_menu(ui, &graph);
                ui.label(format!("{} commits", graph.nodes.len()));
                if graph.replacement_refs { ui.colored_label(theme::accent_yellow(), "Original object history").on_hover_text("This repository has replacement refs. The graph and commit detail show stored original objects; native Git operations may honor replacements. Review replacement refs in Git before editing this history."); }
                if graph.has_more && ui.add_enabled(!loading, egui::Button::new("More...")).clicked() {
                    crate::git::operations::load_more_history(self.state.clone());
                }
                if graph.has_more && search_active { ui.label("Search covers loaded commits."); }
            });
        });
        let header_top = navigation_top + 42.0;
        let header_rect = Rect::from_min_size(
            Pos2::new(ui.min_rect().min.x, header_top),
            Vec2::new(avail, HEADER_H),
        );
        ui.painter()
            .rect_filled(header_rect, 0.0, theme::bg_panel_alt());
        ui.painter().line_segment(
            [header_rect.left_bottom(), header_rect.right_bottom()],
            Stroke::new(1.0, theme::border()),
        );

        self.fit_columns(avail);

        let refs_start = header_rect.min.x;
        let graph_start = refs_start + self.refs_w;
        let msg_start = graph_start + self.graph_w;
        let time_x = msg_start + self.commit_w;
        let hash_x = time_x + self.time_w;
        let end_x = hash_x + self.hash_w;

        let hcol = theme::text_muted();
        let header_font = FontId::proportional(12.0);
        ui.painter().text(
            Pos2::new(refs_start + 10.0, header_rect.center().y),
            egui::Align2::LEFT_CENTER,
            "Branches / Tags",
            header_font.clone(),
            hcol,
        );
        ui.painter().text(
            Pos2::new(graph_start + GRAPH_LEFT, header_rect.center().y),
            egui::Align2::LEFT_CENTER,
            "Graph",
            header_font.clone(),
            hcol,
        );
        ui.painter().text(
            Pos2::new(msg_start + 4.0, header_rect.center().y),
            egui::Align2::LEFT_CENTER,
            "Commit",
            header_font.clone(),
            hcol,
        );
        ui.painter().text(
            Pos2::new(time_x + 8.0, header_rect.center().y),
            egui::Align2::LEFT_CENTER,
            "Time",
            header_font.clone(),
            hcol,
        );
        ui.painter().text(
            Pos2::new(hash_x + 8.0, header_rect.center().y),
            egui::Align2::LEFT_CENTER,
            "Hash",
            header_font.clone(),
            hcol,
        );

        // Compute content-based auto-widths for double-click sizing
        let auto_refs_w = (graph.max_ref_chars as f32 * 6.5 + 40.0).clamp(REFS_W_MIN, REFS_W_MAX);
        let auto_graph_w =
            ((graph.max_lane as f32 + 1.0) * LANE_W + GRAPH_LEFT + DOT_R * 3.0 + 10.0)
                .clamp(GRAPH_W_MIN, GRAPH_W_MAX);

        self.resize_handle(
            ui,
            header_rect,
            graph_start,
            ColumnDivider::RefsGraph,
            auto_refs_w,
        );
        self.resize_handle(
            ui,
            header_rect,
            msg_start,
            ColumnDivider::GraphCommit,
            auto_graph_w,
        );
        self.resize_handle(
            ui,
            header_rect,
            time_x,
            ColumnDivider::CommitTime,
            TIME_W_DEF,
        );
        self.resize_handle(ui, header_rect, hash_x, ColumnDivider::TimeHash, HASH_W_DEF);
        ui.painter().line_segment(
            [
                Pos2::new(end_x, header_rect.min.y),
                Pos2::new(end_x, header_rect.max.y),
            ],
            Stroke::new(1.0, theme::border()),
        );

        // ── Scroll area (graph + list) ────────────────────────────────────
        let scroll_start = header_top + HEADER_H;
        let scroll_rect = Rect::from_min_max(
            Pos2::new(ui.min_rect().min.x, scroll_start),
            ui.max_rect().max,
        );
        let refs_w = self.refs_w;
        let graph_w = self.graph_w;
        let commit_w = self.commit_w;
        let time_w = self.time_w;
        let hash_w = self.hash_w;

        let nodes = &graph.nodes;
        let show_wip = graph
            .head_oid
            .as_ref()
            .is_none_or(|oid| graph.oid_to_row.contains_key(oid));
        let wip_h = if show_wip { ROW_H } else { 0.0 };
        let total_h = nodes.len() as f32 * ROW_H + wip_h;

        let q_lower = if search_active && !search_query.is_empty() {
            Some(search_query.to_lowercase())
        } else {
            None
        };

        let mut new_selection: Option<Option<String>> = None;

        // Arrow-key navigation
        {
            let (up, down) =
                ui.input(|i| (i.key_pressed(Key::ArrowUp), i.key_pressed(Key::ArrowDown)));
            if (up || down)
                && !ui.ctx().wants_keyboard_input()
                && self.state.lock().unwrap().focused_pane == 2
            {
                let cur = selected
                    .as_ref()
                    .and_then(|oid| nodes.iter().position(|n| &n.oid == oid));
                let nxt = match (cur, up) {
                    (Some(i), true) => i.saturating_sub(1),
                    (Some(i), false) => (i + 1).min(nodes.len().saturating_sub(1)),
                    (None, true) => nodes.len().saturating_sub(1),
                    (None, false) => 0,
                };
                if !nodes.is_empty() {
                    new_selection = Some(Some(nodes[nxt].oid.clone()));
                }
            }
        }

        let scroll_row = self
            .scroll_to_oid
            .as_ref()
            .and_then(|oid| graph.oid_to_row.get(oid))
            .copied();
        if scroll_row.is_some() {
            self.scroll_to_oid = None;
        }
        let repo_key = self.state.lock().unwrap().repo_path.clone();
        ui.allocate_ui_at_rect(scroll_rect, |ui| {
            egui::ScrollArea::vertical()
                .id_source(("commit_log_scroll", repo_key))
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    let (resp, painter) =
                        ui.allocate_painter(Vec2::new(avail, total_h), Sense::hover());
                    let origin = resp.rect.min + Vec2::new(0.0, wip_h);
                    if show_wip {
                        let rect = Rect::from_min_size(resp.rect.min, Vec2::new(avail, ROW_H));
                        let response = ui.interact(rect, egui::Id::new("wip_row"), Sense::click());
                        if response.clicked() {
                            new_selection = Some(None);
                        }
                        painter.rect_filled(
                            rect,
                            0.0,
                            if selected.is_none() {
                                theme::bg_selected()
                            } else if response.hovered() {
                                theme::bg_hover()
                            } else {
                                theme::bg_panel()
                            },
                        );
                        let head_row = graph
                            .head_oid
                            .as_ref()
                            .and_then(|oid| graph.oid_to_row.get(oid))
                            .copied();
                        let lane = head_row.map(|row| nodes[row].lane).unwrap_or(0);
                        let x = origin.x + refs_w + GRAPH_LEFT + lane as f32 * LANE_W;
                        let center = Pos2::new(x, rect.center().y);
                        if let Some(row) = head_row {
                            painter.line_segment(
                                [
                                    center,
                                    Pos2::new(x, origin.y + row as f32 * ROW_H + ROW_H / 2.0),
                                ],
                                Stroke::new(2.0, theme::accent_yellow()),
                            );
                        }
                        painter.circle_stroke(
                            center,
                            DOT_R,
                            Stroke::new(2.0, theme::accent_yellow()),
                        );
                        let dirty = self.state.lock().unwrap().dirty_count;
                        painter.text(
                            Pos2::new(origin.x + 10.0, rect.center().y),
                            egui::Align2::LEFT_CENTER,
                            "Working tree",
                            FontId::proportional(13.0),
                            theme::accent_yellow(),
                        );
                        painter.text(
                            Pos2::new(origin.x + refs_w + graph_w + 4.0, rect.center().y),
                            egui::Align2::LEFT_CENTER,
                            format!("Uncommitted changes ({dirty})"),
                            FontId::proportional(13.0),
                            theme::text_primary(),
                        );
                    }
                    if let Some(row) = scroll_row {
                        ui.scroll_to_rect(
                            Rect::from_min_size(
                                Pos2::new(origin.x, origin.y + row as f32 * ROW_H),
                                Vec2::new(avail, ROW_H),
                            ),
                            Some(egui::Align::Center),
                        );
                    }
                    let clip = painter.clip_rect();

                    let first = ((clip.top() - origin.y) / ROW_H).floor().max(0.0) as usize;
                    let last =
                        (((clip.bottom() - origin.y) / ROW_H).ceil() as usize + 1).min(nodes.len());
                    if graph.has_more && !search_active && last.saturating_add(100) >= nodes.len() {
                        crate::git::operations::load_more_history(self.state.clone());
                    }

                    // ── Draw row backgrounds first so graph/text stay above hover/selection.
                    for i in first..last {
                        let node = &nodes[i];
                        let row_top = origin.y + i as f32 * ROW_H;
                        let row_rect = Rect::from_min_size(
                            Pos2::new(origin.x, row_top),
                            Vec2::new(avail, ROW_H),
                        );
                        let row_resp =
                            ui.interact(row_rect, egui::Id::new(("row_bg", i)), Sense::click());
                        let is_sel = selected.as_deref() == Some(&node.oid);
                        row_resp.widget_info(|| {
                            egui::WidgetInfo::selected(
                                egui::WidgetType::SelectableLabel,
                                true,
                                is_sel,
                                format!("{} by {}, {}", node.summary, node.author, node.oid),
                            )
                        });
                        let row_bg = if is_sel {
                            theme::bg_selected()
                        } else if row_resp.hovered() {
                            theme::bg_hover()
                        } else {
                            Color32::TRANSPARENT
                        };
                        if row_bg != Color32::TRANSPARENT {
                            painter.rect_filled(row_rect, 0.0, row_bg);
                        }

                        let is_match = q_lower.as_ref().map_or(true, |q| node_matches(node, q));
                        let in_file_history =
                            file_history_oids.is_empty() || file_history_oids.contains(&node.oid);
                        let dimmed = (q_lower.is_some() && !is_match)
                            || (!file_history_oids.is_empty() && !in_file_history);
                        if dimmed {
                            painter.rect_filled(row_rect, 0.0, Color32::from_black_alpha(80));
                        }
                    }

                    // ── Draw edges (cubic bezier) ─────────────────────
                    for edge in graph.visible_edges(first.saturating_sub(1), last.saturating_add(1))
                    {
                        let from_y = origin.y + edge.from_row as f32 * ROW_H + ROW_H / 2.0;
                        let to_y = origin.y + edge.to_row as f32 * ROW_H + ROW_H / 2.0;
                        if to_y < clip.top() - ROW_H || from_y > clip.bottom() + ROW_H {
                            continue;
                        }

                        let color = theme::lane_color_for_palette(edge.from_lane, self.colorblind);
                        let from_x =
                            origin.x + refs_w + GRAPH_LEFT + edge.from_lane as f32 * LANE_W;
                        let to_x = origin.x + refs_w + GRAPH_LEFT + edge.to_lane as f32 * LANE_W;

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
                                    Pos2::new(to_x, to_y - strength),
                                    Pos2::new(to_x, to_y - DOT_R - 1.0),
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
                        let row_top = origin.y + i as f32 * ROW_H;
                        let row_rect = Rect::from_min_size(
                            Pos2::new(origin.x, row_top),
                            Vec2::new(avail, ROW_H),
                        );
                        let cy = row_top + ROW_H / 2.0;

                        let is_sel = selected.as_deref() == Some(&node.oid);
                        let row_resp =
                            ui.interact(row_rect, egui::Id::new(("row", i)), Sense::click());

                        let is_match = q_lower.as_ref().map_or(true, |q| node_matches(node, q));
                        let in_fh =
                            file_history_oids.is_empty() || file_history_oids.contains(&node.oid);
                        let dim = (q_lower.is_some() && !is_match)
                            || (!file_history_oids.is_empty() && !in_fh);

                        if row_resp.clicked() {
                            self.state.lock().unwrap().focused_pane = 2;
                            new_selection = Some(Some(node.oid.clone()));
                        }

                        let fade = if dim { 0.25 } else { 1.0 };
                        let lane_col = theme::lane_color_for_palette(node.lane, self.colorblind)
                            .gamma_multiply(fade);
                        let dot_x = origin.x + refs_w + GRAPH_LEFT + node.lane as f32 * LANE_W;

                        // Node circle
                        painter.circle_filled(Pos2::new(dot_x, cy), DOT_R, lane_col);
                        painter.circle_stroke(
                            Pos2::new(dot_x, cy),
                            DOT_R,
                            Stroke::new(1.5, theme::bg_app()),
                        );
                        if is_sel {
                            painter.circle_stroke(
                                Pos2::new(dot_x, cy),
                                DOT_R_SEL,
                                Stroke::new(2.0, theme::text_primary()),
                            );
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
                            Pos2::new(hash_right, cy),
                            egui::Align2::RIGHT_CENTER,
                            &node.oid[..7.min(node.oid.len())],
                            FontId::monospace(12.0),
                            dim_col(theme::text_muted()),
                        );
                        // Time
                        painter.text(
                            Pos2::new(time_right, cy),
                            egui::Align2::RIGHT_CENTER,
                            &theme::relative_time(node.time),
                            FontId::proportional(12.0),
                            dim_col(theme::text_muted()),
                        );
                        ui.interact(
                            Rect::from_min_max(
                                Pos2::new(time_start, row_top),
                                Pos2::new(time_start + time_w, row_top + ROW_H),
                            ),
                            egui::Id::new(("time_cell", i)),
                            Sense::hover(),
                        )
                        .on_hover_text(theme::absolute_time(node.time));
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

                        draw_ref_column(
                            &painter,
                            origin.x,
                            refs_w,
                            row_top,
                            cy,
                            node,
                            lane_col,
                            &graph.tag_names,
                        );

                        // Commit summary
                        let summary_start = msg_start + 8.0;
                        let summary_end = time_start - 8.0;
                        if summary_end > summary_start + 20.0 {
                            let tc = if is_sel {
                                theme::text_primary()
                            } else {
                                dim_col(theme::text_primary())
                            };
                            painter
                                .with_clip_rect(Rect::from_min_max(
                                    Pos2::new(summary_start, row_top),
                                    Pos2::new(summary_end, row_top + ROW_H),
                                ))
                                .text(
                                    Pos2::new(summary_start, cy),
                                    egui::Align2::LEFT_CENTER,
                                    &node.summary,
                                    FontId::proportional(14.0),
                                    tc,
                                );
                        }
                    }

                    // Context menu
                    resp.context_menu(|ui| {
                        if let Some(ref oid) = selected {
                            let short = &oid[..7.min(oid.len())];
                            if ui.button(format!("Copy SHA  {}", short)).clicked() {
                                ui.output_mut(|o| o.copied_text = oid.clone());
                                ui.close_menu();
                            }
                            ui.separator();
                            if ui.button("Cherry-pick").clicked() {
                                crate::git::branches::cherry_pick(oid, self.state.clone());
                                ui.close_menu();
                            }
                            if ui.button("Revert Commit").clicked() {
                                crate::git::branches::revert_commit(oid, self.state.clone());
                                ui.close_menu();
                            }
                            if ui.button("Interactive Rebase from here…").clicked() {
                                self.state.lock().unwrap().pending_open_rebase = Some(oid.clone());
                                ui.close_menu();
                            }
                            ui.separator();
                            if ui.button("Create Branch Here").clicked() {
                                self.state.lock().unwrap().modal =
                                    crate::state::ModalKind::CreateBranch {
                                        from_oid: Some(oid.clone()),
                                    };
                                ui.close_menu();
                            }
                            if ui.button("Create Tag Here").clicked() {
                                self.state.lock().unwrap().modal =
                                    crate::state::ModalKind::CreateTag {
                                        from_oid: Some(oid.clone()),
                                    };
                                ui.close_menu();
                            }
                            ui.separator();
                            if ui.button("Checkout (detached HEAD)…").clicked() {
                                self.state.lock().unwrap().modal =
                                    crate::state::ModalKind::ConfirmDetachedCheckout {
                                        oid: oid.clone(),
                                    };
                                ui.close_menu();
                            }
                            ui.separator();
                            // ── Phase 7: compare & bisect ─────────────────────
                            let compare_base = self.state.lock().unwrap().compare_base_oid.clone();
                            if let Some(ref base) = compare_base {
                                let base_short = &base[..7.min(base.len())];
                                if oid != base {
                                    if ui.button(format!("Compare with {}…", base_short)).clicked()
                                    {
                                        self.state.lock().unwrap().modal =
                                            crate::state::ModalKind::Compare {
                                                base_oid: base.clone(),
                                                compare_oid: oid.clone(),
                                            };
                                        ui.close_menu();
                                    }
                                }
                                if ui.button("Clear Compare Base").clicked() {
                                    self.state.lock().unwrap().compare_base_oid = None;
                                    ui.close_menu();
                                }
                            } else {
                                if ui.button("Set as Compare Base").clicked() {
                                    self.state.lock().unwrap().compare_base_oid = Some(oid.clone());
                                    ui.close_menu();
                                }
                            }
                            ui.separator();
                            if ui.button("Bisect…").clicked() {
                                self.state.lock().unwrap().modal = crate::state::ModalKind::Bisect;
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
                    s.current_diff = None;
                    s.selected_file = None;
                    s.focused_pane = 3;
                }
            }
            if let Some(oid) = trigger {
                if self.last_selected.as_deref() != Some(&oid)
                    || self
                        .state
                        .lock()
                        .unwrap()
                        .selected_commit_detail
                        .as_ref()
                        .map(|d| d.oid.as_str())
                        != Some(oid.as_str())
                {
                    self.last_selected = Some(oid.clone());
                    load_commit_detail(&oid, self.state.clone());
                }
            } else {
                self.last_selected = None;
            }
        }
    }

    fn filter_menu(&mut self, ui: &mut Ui, graph: &crate::git::graph::CommitGraph) {
        let mut filter = self.state.lock().unwrap().graph_filter.clone();
        let original = filter.clone();
        ui.menu_button("Filters", |ui| {
            ui.set_min_width(340.0);
            ui.label("Hide excludes a ref as a history root. Shared ancestors remain visible.");
            if let Some(solo) = &filter.solo_ref {
                ui.label(format!("Solo: {solo}"));
            }
            if let Some((start, end)) = &filter.range {
                ui.label(format!("Range: {start}..{end}"));
            }
            if ui.button("Clear graph filters").clicked() {
                filter = Default::default();
            }
            ui.add(egui::TextEdit::singleline(&mut self.ref_query).hint_text("Find branch or tag"));
            egui::ScrollArea::vertical()
                .id_source("graph_ref_filters")
                .max_height(200.0)
                .show(ui, |ui| {
                    for name in graph
                        .ref_targets
                        .keys()
                        .filter(|name| name.contains(&self.ref_query))
                    {
                        ui.horizontal(|ui| {
                            let mut visible = !filter.hidden_refs.contains(name);
                            if ui.checkbox(&mut visible, name).changed() {
                                if visible {
                                    filter.hidden_refs.remove(name);
                                } else {
                                    filter.hidden_refs.insert(name.clone());
                                }
                                filter.solo_ref = None;
                                filter.range = None;
                            }
                            if ui.small_button("Solo").clicked() {
                                filter.solo_ref = Some(name.clone());
                                filter.range = None;
                            }
                        });
                    }
                });
            ui.separator();
            ui.label(
                "Commit range: commits reachable from End, excluding Start and its ancestors.",
            );
            ui.add(
                egui::TextEdit::singleline(&mut self.range_start).hint_text("Start ref or commit"),
            );
            ui.add(egui::TextEdit::singleline(&mut self.range_end).hint_text("End ref or commit"));
            if ui
                .add_enabled(
                    !self.range_start.trim().is_empty() && !self.range_end.trim().is_empty(),
                    egui::Button::new("Apply range"),
                )
                .clicked()
            {
                filter.range = Some((self.range_start.trim().into(), self.range_end.trim().into()));
                filter.solo_ref = None;
                ui.close_menu();
            }
        });
        if filter != original {
            let mut s = self.state.lock().unwrap();
            s.graph_filter = filter;
            s.show_all_branches = true;
            drop(s);
            crate::git::operations::refresh_repo(self.state.clone());
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

    fn resize_handle(
        &mut self,
        ui: &mut Ui,
        header_rect: Rect,
        x: f32,
        divider: ColumnDivider,
        auto_w: f32,
    ) {
        let rect = Rect::from_min_size(
            Pos2::new(x - RESIZE_HIT_W / 2.0, header_rect.min.y),
            Vec2::new(RESIZE_HIT_W, HEADER_H),
        );
        let resp = ui
            .allocate_rect(rect, Sense::click_and_drag())
            .on_hover_text("Drag to resize  •  Double-click to fit content");
        if resp.hovered() || resp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeColumn);
            ui.painter().rect_filled(
                rect.shrink2(Vec2::new(3.0, 2.0)),
                2.0,
                theme::accent_teal().gamma_multiply(0.08),
            );
            ui.painter().line_segment(
                [
                    Pos2::new(x, header_rect.min.y + 4.0),
                    Pos2::new(x, header_rect.max.y - 4.0),
                ],
                Stroke::new(2.0, theme::accent_teal()),
            );
        } else {
            ui.painter().line_segment(
                [
                    Pos2::new(x, header_rect.min.y),
                    Pos2::new(x, header_rect.max.y),
                ],
                Stroke::new(1.0, theme::border()),
            );
        }

        if resp.double_clicked() {
            match divider {
                ColumnDivider::RefsGraph => {
                    self.refs_w = auto_w.clamp(REFS_W_MIN, REFS_W_MAX);
                }
                ColumnDivider::GraphCommit => {
                    self.graph_w = auto_w.clamp(GRAPH_W_MIN, GRAPH_W_MAX);
                }
                ColumnDivider::CommitTime => {
                    self.time_w = TIME_W_DEF;
                }
                ColumnDivider::TimeHash => {
                    self.time_w = TIME_W_DEF;
                    self.hash_w = HASH_W_DEF;
                }
            }
            self.cols_dirty = true;
        } else if resp.dragged() {
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
    if parts.len() != 3 {
        return None;
    }
    let y: i64 = parts[0].parse().ok()?;
    let m: i64 = parts[1].parse().ok()?;
    let d: i64 = parts[2].parse().ok()?;
    // Simplified days-since-epoch calculation (close enough for filtering)
    let leap = |yr: i64| yr % 4 == 0 && (yr % 100 != 0 || yr % 400 == 0);
    let days_in_month = [0i64, 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut days = (y - 1970) * 365 + (y - 1969) / 4 - (y - 1901) / 100 + (y - 1601) / 400;
    for mi in 1..m {
        days += days_in_month[mi as usize];
        if mi == 2 && leap(y) {
            days += 1;
        }
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
    tag_names: &std::collections::HashSet<String>,
) {
    let clip = Rect::from_min_max(
        Pos2::new(origin_x + 8.0, row_top),
        Pos2::new(origin_x + refs_w - 8.0, row_top + ROW_H),
    );
    let painter = painter.with_clip_rect(clip);
    let mut x = origin_x + 10.0;
    for name in node.refs.iter().take(3) {
        let is_tag = tag_names.contains(name.as_str());
        let color = if is_tag {
            theme::accent_purple()
        } else {
            branch_color
        };
        let font = FontId::proportional(12.0);
        let galley = painter.layout_no_wrap(name.to_string(), font, color);
        let width = galley.size().x + 14.0;
        if x + width > origin_x + refs_w - 10.0 {
            break;
        }
        let rect = Rect::from_min_size(Pos2::new(x, row_top + 8.0), Vec2::new(width, ROW_H - 16.0));
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
