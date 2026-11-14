use crate::ui::button::Button;
use egui::{Color32, FontId, Key, Pos2, Rect, Sense, Stroke, Ui, Vec2};

use crate::git::operations::load_commit_detail;
use crate::state::SharedState;
use crate::theme;

// Layout
const ROW_H: f32 = 36.0;
const REFS_W_DEF: f32 = 130.0;
const REFS_W_MIN: f32 = 90.0;
const REFS_W_MAX: f32 = 320.0;
const GRAPH_W_MIN: f32 = 80.0;
const GRAPH_W_MAX: f32 = 340.0;
const GRAPH_W_DEF: f32 = 100.0;
/// Until the user sizes the graph column it widens to fit the loaded lanes,
/// up to this width so busy histories leave room for commit messages.
const GRAPH_W_AUTO_MAX: f32 = 264.0;
const COMMIT_W_DEF: f32 = 380.0;
const COMMIT_W_MIN: f32 = 180.0;
const TIME_W_DEF: f32 = 60.0;
const TIME_W_MIN: f32 = 46.0;
const HASH_W_DEF: f32 = 62.0;
const HASH_W_MIN: f32 = 54.0;
// Each commit node is filled with its lane colour and shows the author's initials
// (9 px text), outlined in a colour unique to that author.
const LANE_W: f32 = 24.0;
const AVATAR_R: f32 = 10.5;
/// The author outline plus a one-pixel gap between it and the lane fill.
const NODE_RING: f32 = 3.0;
const NODE_R: f32 = AVATAR_R + NODE_RING;
const NODE_R_SEL: f32 = NODE_R + 2.0;
const WIP_R: f32 = AVATAR_R - 2.0;
const GRAPH_LEFT: f32 = 17.0;
const HEADER_H: f32 = 34.0;
const NAVIGATION_H: f32 = 52.0;
const RESIZE_HIT_W: f32 = 14.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GraphColumn {
    References,
    Graph,
    Commit,
    Time,
    Hash,
}

impl GraphColumn {
    const ALL: [Self; 5] = [
        Self::References,
        Self::Graph,
        Self::Commit,
        Self::Time,
        Self::Hash,
    ];
    fn label(self) -> &'static str {
        match self {
            Self::References => "Branches / Tags",
            Self::Graph => "Graph",
            Self::Commit => "Commit",
            Self::Time => "Time",
            Self::Hash => "Hash",
        }
    }
    fn index(self) -> usize {
        self as usize
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GraphDateFormat {
    #[default]
    Relative,
    Date,
    DateTime,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct GraphPreferences {
    /// Visible columns in display order; the commit summary is always retained.
    pub columns: Vec<GraphColumn>,
    pub date_format: GraphDateFormat,
}

impl Default for GraphPreferences {
    fn default() -> Self {
        Self {
            columns: GraphColumn::ALL.to_vec(),
            date_format: GraphDateFormat::Relative,
        }
    }
}

impl GraphPreferences {
    fn normalize(&mut self) {
        let mut seen = [false; 5];
        self.columns.retain(|column| {
            let old = seen[column.index()];
            seen[column.index()] = true;
            !old
        });
        if !self.columns.contains(&GraphColumn::Commit) {
            self.columns.push(GraphColumn::Commit);
        }
    }
}

struct SearchCache {
    query: String,
    graph: std::sync::Weak<crate::git::graph::CommitGraph>,
    nodes_len: usize,
    matches: std::sync::Arc<Vec<bool>>,
    count: usize,
}

impl SearchCache {
    fn is_valid(
        &self,
        query: &str,
        graph: &std::sync::Arc<crate::git::graph::CommitGraph>,
    ) -> bool {
        self.query == query
            && self.graph.as_ptr() == std::sync::Arc::as_ptr(graph)
            && self.nodes_len == graph.nodes.len()
    }

    fn build(query: &str, graph: &std::sync::Arc<crate::git::graph::CommitGraph>) -> Self {
        let q = query.to_lowercase();
        let matches: Vec<bool> = graph
            .nodes
            .iter()
            .map(|node| node_matches(node, &q))
            .collect();
        let count = matches.iter().filter(|matched| **matched).count();
        Self {
            query: query.to_owned(),
            graph: std::sync::Arc::downgrade(graph),
            nodes_len: graph.nodes.len(),
            matches: std::sync::Arc::new(matches),
            count,
        }
    }
}

fn cached_search<'a>(
    cache: &'a mut Option<SearchCache>,
    query: &str,
    graph: &std::sync::Arc<crate::git::graph::CommitGraph>,
) -> &'a SearchCache {
    if !cache
        .as_ref()
        .is_some_and(|cache| cache.is_valid(query, graph))
    {
        *cache = Some(SearchCache::build(query, graph));
    }
    cache.as_ref().unwrap()
}

/// Width that shows every lane up to `max_lane`.
fn graph_fit_width(max_lane: usize) -> f32 {
    (GRAPH_LEFT + max_lane as f32 * LANE_W + NODE_R_SEL + 12.0).clamp(GRAPH_W_MIN, GRAPH_W_MAX)
}

pub struct CommitGraphView {
    state: SharedState,
    last_selected: Option<String>,
    scroll_to_oid: Option<String>,
    refs_w: f32,
    graph_w: f32,
    /// The graph column follows its lanes until the user resizes it.
    graph_w_auto: bool,
    commit_w: f32,
    time_w: f32,
    hash_w: f32,
    cols_dirty: bool,
    last_saved_refs_w: f32,
    last_saved_graph_w: f32,
    last_saved_time_w: f32,
    last_saved_hash_w: f32,
    colorblind: bool,
    palette_repo: Option<std::path::PathBuf>,
    color_order: [usize; 8],
    range_start: String,
    range_end: String,
    ref_query: String,
    drag: Option<crate::git::graph_action::Drag>,
    pending_action: Option<crate::git::graph_action::Preview>,
    viewport_anchor: Option<(String, usize, f32)>,
    interactions_enabled: bool,
    confirmation_focus: bool,
    keyboard_row: Option<String>,
    search_was_active: bool,
    search_cache: Option<SearchCache>,
    /// Where a Shift-click range starts.
    selection_anchor: Option<String>,
    preferences: GraphPreferences,
    preferences_dirty: bool,
    sidebar_action: Option<crate::ui::sidebar::SidebarAction>,
}

impl CommitGraphView {
    pub fn new(state: SharedState) -> Self {
        Self {
            state,
            last_selected: None,
            scroll_to_oid: None,
            refs_w: REFS_W_DEF,
            graph_w: GRAPH_W_DEF,
            graph_w_auto: true,
            commit_w: COMMIT_W_DEF,
            time_w: TIME_W_DEF,
            hash_w: HASH_W_DEF,
            cols_dirty: false,
            last_saved_refs_w: REFS_W_DEF,
            last_saved_graph_w: GRAPH_W_DEF,
            last_saved_time_w: TIME_W_DEF,
            last_saved_hash_w: HASH_W_DEF,
            colorblind: false,
            palette_repo: None,
            color_order: theme::graph_color_order(None),
            range_start: String::new(),
            range_end: String::new(),
            ref_query: String::new(),
            drag: None,
            pending_action: None,
            viewport_anchor: None,
            interactions_enabled: true,
            confirmation_focus: true,
            keyboard_row: None,
            search_was_active: false,
            search_cache: None,
            selection_anchor: None,
            preferences: GraphPreferences::default(),
            preferences_dirty: false,
            sidebar_action: None,
        }
    }

    pub fn set_interactions_enabled(&mut self, enabled: bool) {
        self.interactions_enabled = enabled;
        if !enabled {
            self.drag = None;
        }
    }

    pub fn set_preferences(&mut self, mut preferences: GraphPreferences) {
        preferences.normalize();
        self.preferences = preferences;
        self.preferences_dirty = false;
    }

    pub fn take_dirty_preferences(&mut self) -> Option<GraphPreferences> {
        if std::mem::take(&mut self.preferences_dirty) {
            Some(self.preferences.clone())
        } else {
            None
        }
    }

    pub fn take_sidebar_action(&mut self) -> Option<crate::ui::sidebar::SidebarAction> {
        self.sidebar_action.take()
    }

    pub fn has_pending_action(&self) -> bool {
        self.pending_action.is_some()
    }

    /// Select `oid`, scroll it into view and load its details. Hidden commits
    /// clear graph filters so the next refresh can show them.
    pub fn reveal_commit(&mut self, oid: &str) {
        let visible = {
            let mut s = self.state.lock().unwrap();
            let graph = s.commit_graph.clone();
            let visible = graph
                .as_ref()
                .is_some_and(|graph| graph.oid_to_row.contains_key(oid));
            s.selected_commit = Some(oid.to_string());
            if !visible {
                s.graph_filter = Default::default();
                // The current-branch view always contains HEAD; other tips need all branches.
                s.show_all_branches = graph
                    .and_then(|graph| graph.head_oid.clone())
                    .is_none_or(|head| head != oid);
            }
            visible
        };
        self.scroll_to_oid = Some(oid.to_string());
        if !visible {
            crate::git::operations::refresh_repo(self.state.clone());
        }
        load_commit_detail(oid, self.state.clone());
    }
    pub fn cancel_action(&mut self) {
        self.drag = None;
        self.pending_action = None;
        self.confirmation_focus = true;
    }

    /// Keyboard equivalent of dropping a selected row onto HEAD, or dragging
    /// the current branch onto that row. Uses exactly the same preview and queue.
    pub fn preview_selected(&mut self, action: crate::git::graph_action::Action) {
        let oid = self.state.lock().unwrap().selected_commit.clone();
        if let Some(oid) = oid {
            self.preview_endpoint(
                crate::git::graph_action::Endpoint {
                    oid,
                    reference: None,
                },
                action,
            );
        }
    }

    // Both keyboard/chip actions and context menus use an explicit immutable
    // endpoint. A menu never reads a potentially different current selection.
    fn preview_endpoint(
        &mut self,
        selected: crate::git::graph_action::Endpoint,
        action: crate::git::graph_action::Action,
    ) {
        use crate::git::graph_action::{Action, Endpoint, Preview};
        let (path, epoch, head, branch) = {
            let s = self.state.lock().unwrap();
            (
                s.repo_path.clone(),
                s.repo_epoch,
                s.commit_graph.as_ref().and_then(|g| g.head_oid.clone()),
                s.head_branch.clone(),
            )
        };
        let (Some(path), Some(head), Some(branch)) = (path, head, branch) else {
            return;
        };
        let current = Endpoint {
            oid: head,
            reference: Some(format!("refs/heads/{branch}")),
        };
        let (source, destination) = if action == Action::Rebase {
            (current, selected)
        } else {
            (selected, current)
        };
        match Preview::capture_action(&path, epoch, source, destination, action) {
            Ok(preview) => {
                self.pending_action = Some(preview);
                self.confirmation_focus = true;
            }
            Err(error) => self.state.lock().unwrap().error = Some(error),
        }
    }

    /// Called by the app after all panels so confirmation remains visible even
    /// when a different pane gains focus. No mutation happens on drop.
    pub fn show_action_confirmation(&mut self, ctx: &egui::Context, protected_branches: &[String]) {
        use crate::git::graph_action::Action;
        let Some(mut preview) = self.pending_action.clone() else {
            return;
        };
        let current = {
            let s = self.state.lock().unwrap();
            s.repo_epoch == preview.epoch
                && s.repo_path.as_ref() == Some(&preview.path)
                && matches!(s.modal, crate::state::ModalKind::None)
                && s.pending_discard.is_none()
                && s.repo_load_path.is_none()
        };
        if !current {
            self.cancel_action();
            return;
        }
        preview.protected_branches = protected_branches.to_vec();
        let protected = preview.action == Action::Rebase
            && protected_branches
                .iter()
                .any(|name| preview.head_ref.strip_prefix("refs/heads/") == Some(name.as_str()));
        let mut confirm = false;
        let mut cancel = false;
        egui::Area::new(egui::Id::new("graph_confirmation_shield"))
            .order(egui::Order::Foreground)
            .fixed_pos(ctx.content_rect().min)
            .show(ctx, |ui| {
                let sense = Sense::CLICK | Sense::DRAG;
                ui.allocate_rect(ctx.content_rect(), sense);
            });
        let window = egui::Window::new("Confirm graph operation").order(egui::Order::Foreground).collapsible(false).resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO).show(ctx, |ui| {
                ui.set_max_width(540.0);
                ui.label(format!(
                    "Repository: {}",
                    crate::ui::kit::display_path(&preview.path)
                ));
                ui.label(format!("From: {}", preview.source.label()));
                ui.label(format!("Destination: {}", preview.destination.label()));
                ui.horizontal(|ui| {
                    for action in [Action::Merge, Action::Rebase, Action::CherryPick, Action::Revert] {
                        ui.add_enabled_ui(preview.allows(action), |ui| { crate::ui::kit::radio_value(ui, &mut preview.action, action, action.label()); });
                    }
                });
                ui.strong(preview.description());
                ui.label(format!("Expected HEAD: {} ({})", preview.head_ref, &preview.head_oid[..7]));
                if preview.source_is_merge { ui.label("Cherry-picking or reverting a merge commit requires choosing a mainline parent; use Git for that operation."); }
                ui.label("Branch protection prevents history rewriting by graph rebase; merges, cherry-picks and reverts remain available.");
                if preview.action == Action::Rebase { ui.colored_label(theme::accent_orange(), "Rebase rewrites commits on the current branch."); }
                ui.label("A recovery snapshot preserves staged, unstaged and untracked work. Git may refuse if changes would be overwritten. Conflicts use the existing Continue / Abort controls. Repository, HEAD and both targets are checked again when the queued operation starts.");
                ui.horizontal(|ui| {
                    confirm = ui.add_enabled(!protected, Button::new(format!("Confirm {}", preview.action.label()))).clicked();
                    if protected { ui.label("Protected branch: graph rebase is disabled"); }
                    let cancel_button = ui.button("Cancel");
                    if self.confirmation_focus { cancel_button.request_focus(); self.confirmation_focus = false; }
                    cancel = cancel_button.clicked();
                });
                cancel |= ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape));
            });
        if let Some(window) = window {
            ctx.move_to_top(window.response.layer_id);
        }
        if confirm {
            self.pending_action = None;
            self.confirmation_focus = true;
            preview.submit(self.state.clone());
        } else if cancel {
            self.cancel_action();
        } else {
            self.pending_action = Some(preview);
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
        match graph_w {
            Some(w) => {
                self.graph_w = w.clamp(GRAPH_W_MIN, GRAPH_W_MAX);
                self.graph_w_auto = false;
            }
            None => self.graph_w_auto = true,
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
        self.graph_w_auto = true;
        self.commit_w = COMMIT_W_DEF;
        self.time_w = TIME_W_DEF;
        self.hash_w = HASH_W_DEF;
        self.cols_dirty = true;
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
        if !self.interactions_enabled {
            ui.disable();
        }
        let (
            graph_opt,
            selected,
            multi,
            search_active,
            search_query,
            file_history_path,
            file_history_oids,
        ) = {
            let s = self.state.lock().unwrap();
            (
                s.commit_graph.clone(),
                s.selected_commit.clone(),
                s.selected_commits
                    .iter()
                    .cloned()
                    .collect::<std::collections::HashSet<_>>(),
                s.search_active,
                s.search_query.clone(),
                s.file_history_path.clone(),
                s.file_history_oids.clone(),
            )
        };

        ui.painter()
            .rect_filled(ui.max_rect(), 0.0, theme::bg_app());

        let Some(graph) = graph_opt else {
            crate::ui::kit::empty_state(
                ui,
                crate::icons::COMMIT,
                "No repository open",
                "Open a repository to view its history.",
                None,
            );
            return;
        };

        let avail = ui.available_width();

        // ── Search bar ────────────────────────────────────────────────────
        let search_h = if search_active { 34.0 } else { 0.0 };
        let mut new_query = search_query.clone();
        let mut close_search = false;
        let search_just_opened = search_active && !self.search_was_active;
        self.search_was_active = search_active;

        if search_active {
            let bar_rect = Rect::from_min_size(ui.min_rect().min, Vec2::new(avail, search_h));
            ui.painter().rect_filled(bar_rect, 0.0, theme::bg_panel());
            ui.painter().line_segment(
                [bar_rect.left_bottom(), bar_rect.right_bottom()],
                Stroke::new(1.0, theme::border()),
            );
            ui.scope_builder(egui::UiBuilder::new().max_rect(bar_rect), |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add_space(10.0);
                    ui.label(egui::RichText::new("⌕").size(14.0).color(theme::text_muted()));
                    ui.add_space(6.0);
                    let r = ui.add(
                        crate::ui::kit::compact_text_field(&mut new_query)
                            .id(egui::Id::new("graph_search"))
                            .desired_width(300.0)
                            .margin(egui::Vec2::new(8.0, 5.0))
                            .hint_text("Search… (message, author:, sha:, after:YYYY-MM-DD, before:YYYY-MM-DD)")
                            .font(FontId::proportional(13.0)),
                    );
                    if search_just_opened && self.interactions_enabled { r.request_focus(); }
                    if !new_query.is_empty() {
                        let n = cached_search(&mut self.search_cache, &new_query, &graph).count;
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
            let escape = self.interactions_enabled
                && self.pending_action.is_none()
                && !egui::Popup::is_any_open(ui.ctx())
                && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape));
            if close_search || escape {
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
            ui.scope_builder(egui::UiBuilder::new().max_rect(banner_rect), |ui| {
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
            Vec2::new(avail, NAVIGATION_H),
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(navigation_rect), |ui| {
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.add_space(10.0);
                let (dirty, loading) = { let s = self.state.lock().unwrap(); (s.dirty_count, s.repo_load_path.is_some()) };
                if ui
                    .button(format!("Changes ({dirty})"))
                    .on_hover_text("Go to your uncommitted changes and the commit form.")
                    .clicked()
                {
                    let mut s = self.state.lock().unwrap();
                    s.selected_commit = None; s.selected_commit_detail = None; s.current_diff = None; s.focused_pane = 3;
                }
                if ui
                    .button("Current commit")
                    .on_hover_text(
                        "Scroll history to the commit you have checked out (Git calls it HEAD).",
                    )
                    .clicked()
                {
                    if let Some(oid) = &graph.head_oid {
                        self.reveal_commit(oid);
                    }
                }
                self.filter_menu(ui, &graph);
                ui.menu_button("Columns", |ui| self.preferences_menu(ui))
                    .response
                    .on_hover_text("Choose which history columns to show.");
                // A muted count, not a button-row peer: it reports, it does nothing.
                let count = graph.nodes.len();
                ui.add_space(crate::ui::kit::space::XS);
                ui.label(
                    egui::RichText::new(format!(
                        "{count} {}{}",
                        if count == 1 { "commit" } else { "commits" },
                        if graph.has_more { " loaded" } else { "" }
                    ))
                    .size(theme::font_size::SMALL)
                    .color(theme::text_muted()),
                );
                if graph.order_may_refine && graph.has_more { ui.label("Progressive history").on_hover_text("History loads without writing an index. As more commits load, rows may move to keep children before parents when commit clocks differ. Optimize History builds Git's index for stable ordering."); }
                if graph.replacement_refs { ui.colored_label(theme::accent_yellow(), "Replacement history").on_hover_text("This graph and commit detail honor Git replacement objects. Original commit IDs are retained; replacement metadata and ancestry are shown."); }
                if graph.has_more && ui.add_enabled(!loading, Button::new("More...")).clicked() {
                    crate::git::operations::load_more_history(self.state.clone());
                }
                if graph.has_more && search_active { ui.label("Search covers loaded commits."); }
            });
        });
        let header_top = navigation_top + NAVIGATION_H;
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

        if self.graph_w_auto {
            self.graph_w = graph_fit_width(graph.max_lane).clamp(GRAPH_W_DEF, GRAPH_W_AUTO_MAX);
            // Following the lanes is not a user edit, so it is not saved.
            self.last_saved_graph_w = self.graph_w;
        }
        self.fit_columns(avail);

        let (column_starts, column_widths) = self.column_layout();
        for column in self.preferences.columns.clone() {
            let x = header_rect.min.x + column_starts[column.index()];
            let width = column_widths[column.index()];
            let cell =
                Rect::from_min_size(Pos2::new(x, header_rect.min.y), Vec2::new(width, HEADER_H));
            ui.painter()
                .with_clip_rect(cell.intersect(ui.clip_rect()))
                .text(
                    Pos2::new(x + 10.0, header_rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    column.label(),
                    FontId::proportional(12.0),
                    theme::text_muted(),
                );
            ui.interact(
                cell,
                egui::Id::new(("graph_header", column.index())),
                Sense::click(),
            )
            .on_hover_text("Right-click to hide, reorder or format columns")
            .context_menu(|ui| self.preferences_menu(ui));
        }
        let columns = self.preferences.columns.clone();
        for pair in columns.windows(2) {
            let column = pair[0];
            let next = pair[1];
            let x =
                header_rect.min.x + column_starts[column.index()] + column_widths[column.index()];
            let auto_width = match column {
                GraphColumn::References => {
                    (graph.max_ref_chars as f32 * 6.5 + 40.0).clamp(REFS_W_MIN, REFS_W_MAX)
                }
                GraphColumn::Graph => graph_fit_width(graph.max_lane),
                GraphColumn::Commit => COMMIT_W_DEF,
                GraphColumn::Time => self.time_min_width(),
                GraphColumn::Hash => HASH_W_DEF,
            };
            self.resize_handle(ui, header_rect, x, column, next, auto_width);
        }

        let scroll_start = header_top + HEADER_H;
        let scroll_rect = Rect::from_min_max(
            Pos2::new(ui.min_rect().min.x, scroll_start),
            ui.max_rect().max,
        );
        let refs_w = column_widths[GraphColumn::References.index()];
        let graph_w = column_widths[GraphColumn::Graph.index()];
        let commit_w = column_widths[GraphColumn::Commit.index()];
        let time_w = column_widths[GraphColumn::Time.index()];
        let hash_w = column_widths[GraphColumn::Hash.index()];
        let refs_offset = column_starts[GraphColumn::References.index()];
        let graph_offset = column_starts[GraphColumn::Graph.index()];
        let commit_offset = column_starts[GraphColumn::Commit.index()];
        let time_offset = column_starts[GraphColumn::Time.index()];
        let hash_offset = column_starts[GraphColumn::Hash.index()];

        let nodes = &graph.nodes;
        let show_wip = graph
            .head_oid
            .as_ref()
            .is_none_or(|oid| graph.oid_to_row.contains_key(oid));
        let wip_h = if show_wip { ROW_H } else { 0.0 };
        let total_h = nodes.len() as f32 * ROW_H + wip_h;

        let search_matches = if search_active && !new_query.is_empty() {
            Some(
                cached_search(&mut self.search_cache, &new_query, &graph)
                    .matches
                    .clone(),
            )
        } else {
            None
        };

        let mut new_selection: Option<Option<String>> = None;
        // Replacement for the multi-selection; plain clicks and keys clear it.
        let mut new_multi: Option<Vec<String>> = None;

        // Arrow-key navigation
        {
            let (up, down) =
                ui.input(|i| (i.key_pressed(Key::ArrowUp), i.key_pressed(Key::ArrowDown)));
            let focused_row = self.keyboard_row.as_ref().filter(|oid| {
                ui.memory(|memory| memory.has_focus(egui::Id::new(("commit_row", oid))))
                    && graph.oid_to_row.contains_key(*oid)
            });
            if self.interactions_enabled
                && ui.is_enabled()
                && (up || down)
                && (focused_row.is_some()
                    || (!ui.ctx().egui_wants_keyboard_input()
                        && self.state.lock().unwrap().focused_pane == 2))
            {
                let cur = focused_row
                    .or(selected.as_ref())
                    .and_then(|oid| graph.oid_to_row.get(oid).copied());
                let nxt = match (cur, up) {
                    (Some(i), true) => i.saturating_sub(1),
                    (Some(i), false) => (i + 1).min(nodes.len().saturating_sub(1)),
                    (None, true) => nodes.len().saturating_sub(1),
                    (None, false) => 0,
                };
                if !nodes.is_empty() {
                    let oid = nodes[nxt].oid.clone();
                    ui.memory_mut(|memory| {
                        // A key can arrive on the first focused frame, before the
                        // row's focus-lock filter takes effect. We handled it.
                        memory.move_focus(egui::FocusDirection::None);
                        memory.request_focus(egui::Id::new(("commit_row", &oid)));
                    });
                    self.scroll_to_oid = Some(oid.clone());
                    self.keyboard_row = Some(oid.clone());
                    self.state.lock().unwrap().focused_pane = 2;
                    new_selection = Some(Some(oid));
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
        if self.palette_repo != repo_key {
            self.color_order = theme::graph_color_order(repo_key.as_deref());
            self.palette_repo = repo_key.clone();
            self.viewport_anchor = None;
        }
        let anchor_offset = self
            .viewport_anchor
            .as_ref()
            .and_then(|(oid, old_row, fraction)| {
                let new_row = graph.oid_to_row.get(oid)?;
                if old_row == new_row {
                    None
                } else {
                    Some(wip_h + *new_row as f32 * ROW_H + fraction)
                }
            });
        ui.scope_builder(egui::UiBuilder::new().max_rect(scroll_rect), |ui| {
            let mut scroll = egui::ScrollArea::vertical()
                .id_salt(("commit_log_scroll", repo_key))
                .auto_shrink([false; 2]);
            if let Some(offset) = anchor_offset { scroll = scroll.vertical_scroll_offset(offset); }
            scroll.show(ui, |ui| {
                    ui.ctx().accesskit_node_builder(ui.id(), |node| {
                        node.set_role(egui::accesskit::Role::ListBox);
                        node.set_label("Commit history");
                    });
                    let (resp, painter) =
                        ui.allocate_painter(Vec2::new(avail, total_h), Sense::hover());
                    let origin = resp.rect.min + Vec2::new(0.0, wip_h);
                    let graph_painter = painter.with_clip_rect(Rect::from_min_max(
                        Pos2::new(origin.x + graph_offset, painter.clip_rect().top()),
                        Pos2::new(origin.x + graph_offset + graph_w, painter.clip_rect().bottom()),
                    ).intersect(painter.clip_rect()));
                    let mut wip_connector = None;
                    if show_wip {
                        let rect = Rect::from_min_size(resp.rect.min, Vec2::new(avail, ROW_H));
                        let response = ui
                            .interact(rect, egui::Id::new("wip_row"), Sense::click())
                            .on_hover_text(
                                "Your uncommitted work. Select to review changes and commit.",
                            );
                        let dirty = self.state.lock().unwrap().dirty_count;
                        let wip_label = format!("Uncommitted changes ({dirty})");
                        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, ui.is_enabled(), selected.is_none(), &wip_label));
                        ui.ctx().accesskit_node_builder(response.id, |node| {
                            node.set_role(egui::accesskit::Role::ListBoxOption);
                            node.set_selected(selected.is_none());
                            node.clear_toggled();
                        });
                        if response.clicked() {
                            new_selection = Some(None);
                        }
                        painter.rect_filled(
                            rect,
                            0.0,
                            if selected.is_none() {
                                theme::bg_selected()
                            } else {
                                crate::ui::motion::mix(
                                    theme::bg_panel(),
                                    theme::bg_hover(),
                                    crate::ui::motion::hover(ui, &response),
                                )
                            },
                        );
                        let head_row = graph
                            .head_oid
                            .as_ref()
                            .and_then(|oid| graph.oid_to_row.get(oid))
                            .copied();
                        let lane = head_row.map(|row| nodes[row].lane).unwrap_or(0);
                        let x = origin.x + graph_offset + GRAPH_LEFT + lane as f32 * LANE_W;
                        let center = Pos2::new(x, rect.center().y);
                        wip_connector = head_row.map(|row| {
                            [
                                Pos2::new(x, center.y + WIP_R),
                                Pos2::new(x, origin.y + row as f32 * ROW_H + ROW_H / 2.0),
                            ]
                        });
                        graph_painter.circle_stroke(
                            center,
                            WIP_R,
                            Stroke::new(2.0, theme::accent_yellow()),
                        );
                        // The hollow node already marks this row, so the refs
                        // column stays empty instead of naming it a second time.
                        painter.with_clip_rect(Rect::from_min_max(
                            Pos2::new(origin.x + commit_offset, rect.top()),
                            Pos2::new(origin.x + commit_offset + commit_w, rect.bottom()),
                        ).intersect(painter.clip_rect())).text(
                            Pos2::new(origin.x + commit_offset + 4.0, rect.center().y),
                            egui::Align2::LEFT_CENTER,
                            wip_label,
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
                    if let Some(node) = nodes.get(first) {
                        self.viewport_anchor = Some((node.oid.clone(), first, (clip.top() - origin.y - first as f32 * ROW_H).max(0.0)));
                    }
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
                            ui.interact(row_rect, egui::Id::new(("row_bg", &node.oid)), Sense::hover());
                        let is_sel = selected.as_deref() == Some(&node.oid) || multi.contains(&node.oid);

                        let row_bg = if is_sel {
                            theme::bg_selected()
                        } else {
                            crate::ui::motion::mix(
                                Color32::TRANSPARENT,
                                theme::bg_hover(),
                                crate::ui::motion::hover(ui, &row_resp),
                            )
                        };
                        if row_bg != Color32::TRANSPARENT {
                            painter.rect_filled(row_rect, 0.0, row_bg);
                        }

                        let is_match = search_matches.as_ref().map_or(true, |matches| matches[i]);
                        let in_file_history =
                            file_history_oids.is_empty() || file_history_oids.contains(&node.oid);
                        let dimmed = (search_matches.is_some() && !is_match)
                            || (!file_history_oids.is_empty() && !in_file_history);
                        if dimmed {
                            painter.rect_filled(
                                row_rect,
                                0.0,
                                theme::bg_app().gamma_multiply(0.32),
                            );
                        }
                    }

                    // ── Draw edges (cubic bezier) ─────────────────────
                    if let Some(segment) = wip_connector {
                        graph_painter.line_segment(segment, Stroke::new(2.0, theme::accent_yellow()));
                    }
                    for edge in graph.visible_edges(first.saturating_sub(1), last.saturating_add(1))
                    {
                        let from_y = origin.y + edge.from_row as f32 * ROW_H + ROW_H / 2.0;
                        let to_y = origin.y + edge.to_row as f32 * ROW_H + ROW_H / 2.0;
                        if to_y < clip.top() - ROW_H || from_y > clip.bottom() + ROW_H {
                            continue;
                        }

                        let color = theme::lane_color_for_palette(
                            self.color_order[edge.from_lane % self.color_order.len()],
                            self.colorblind,
                        );
                        let from_x =
                            origin.x + graph_offset + GRAPH_LEFT + edge.from_lane as f32 * LANE_W;
                        let to_x = origin.x + graph_offset + GRAPH_LEFT + edge.to_lane as f32 * LANE_W;

                        if edge.from_lane == edge.to_lane {
                            graph_painter.line_segment(
                                [Pos2::new(from_x, from_y), Pos2::new(to_x, to_y)],
                                Stroke::new(2.0, color),
                            );
                        } else {
                            // Cubic bezier for smooth branching curves
                            let dy = (to_y - from_y).abs();
                            let strength = (dy * 0.45).max(12.0).min(60.0);
                            let shape = egui::epaint::CubicBezierShape::from_points_stroke(
                                [
                                    Pos2::new(from_x, from_y + NODE_R),
                                    Pos2::new(from_x, from_y + strength),
                                    Pos2::new(to_x, to_y - strength),
                                    Pos2::new(to_x, to_y - NODE_R),
                                ],
                                false,
                                Color32::TRANSPARENT,
                                Stroke::new(2.0, color),
                            );
                            graph_painter.add(egui::Shape::CubicBezier(shape));
                        }
                    }

                    // ── Draw rows ─────────────────────────────────────
                    let mut interactive_rows: Vec<_> = (first..last).collect();
                    if let Some(row) = self.keyboard_row.as_ref()
                        .filter(|oid| ui.memory(|memory| memory.has_focus(egui::Id::new(("commit_row", oid)))))
                        .and_then(|oid| graph.oid_to_row.get(oid)).copied() {
                        if !(first..last).contains(&row) { interactive_rows.push(row); }
                    }
                    for i in interactive_rows {
                        let node = &nodes[i];
                        let row_top = origin.y + i as f32 * ROW_H;
                        let row_rect = Rect::from_min_size(
                            Pos2::new(origin.x, row_top),
                            Vec2::new(avail, ROW_H),
                        );
                        let cy = row_top + ROW_H / 2.0;

                        let is_sel = selected.as_deref() == Some(&node.oid) || multi.contains(&node.oid);
                        let row_resp = ui.interact(row_rect, egui::Id::new(("commit_row", &node.oid)), Sense::click_and_drag());
                        row_resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, is_sel, format!("Commit {} by {}, {}. Drag onto HEAD to cherry-pick.", node.summary, node.author, node.oid)));
                        ui.ctx().accesskit_node_builder(row_resp.id, |accessible| {
                            accessible.set_role(egui::accesskit::Role::ListBoxOption);
                            accessible.set_selected(is_sel);
                            accessible.clear_toggled();
                        });
                        if row_resp.has_focus() {
                            ui.memory_mut(|memory| memory.set_focus_lock_filter(row_resp.id, egui::EventFilter { vertical_arrows: true, ..Default::default() }));
                            self.keyboard_row = Some(node.oid.clone()); painter.rect_stroke(row_rect.shrink(1.0), 0.0, Stroke::new(1.5, theme::accent_blue()), egui::StrokeKind::Middle); }
                        if row_resp.drag_started() && self.pending_action.is_none() {
                            let mut s = self.state.lock().unwrap();
                            if matches!(s.modal, crate::state::ModalKind::None) {
                                if let Some(path) = &s.repo_path {
                                    match crate::git::graph_action::Drag::capture(path, s.repo_epoch, crate::git::graph_action::Endpoint { oid: node.oid.clone(), reference: None }) {
                                        Ok(drag) => self.drag = Some(drag), Err(error) => s.error = Some(error),
                                    }
                                }
                            }
                        }
                        let is_match = search_matches.as_ref().map_or(true, |matches| matches[i]);
                        let in_fh =
                            file_history_oids.is_empty() || file_history_oids.contains(&node.oid);
                        let dim = (search_matches.is_some() && !is_match)
                            || (!file_history_oids.is_empty() && !in_fh);

                        if row_resp.clicked() {
                            self.state.lock().unwrap().focused_pane = 2;
                            let modifiers = ui.input(|i| i.modifiers);
                            let (selection, set) = click_selection(
                                &graph,
                                selected.as_deref(),
                                &multi,
                                self.selection_anchor.as_deref(),
                                &node.oid,
                                modifiers,
                            );
                            if !modifiers.shift {
                                self.selection_anchor = Some(node.oid.clone());
                            }
                            new_selection = Some(selection);
                            new_multi = Some(set);
                        }

                        let fade = if dim { 0.25 } else { 1.0 };
                        let lane_col = theme::lane_color_for_palette(
                            self.color_order[node.lane % self.color_order.len()],
                            self.colorblind,
                        )
                        .gamma_multiply(fade);
                        let dot_x = origin.x + graph_offset + GRAPH_LEFT + node.lane as f32 * LANE_W;

                        // Node: the lane colour with the author's initials, outlined in
                        // that author's own colour. A thin background gap keeps the
                        // outline readable when the two colours are similar.
                        let node_pos = Pos2::new(dot_x, cy);
                        graph_painter.circle_filled(node_pos, NODE_R, theme::author_ring_color(&node.author).gamma_multiply(fade));
                        graph_painter.circle_filled(node_pos, NODE_R - 2.0, theme::bg_app());
                        match crate::ui::avatars::texture(ui.ctx(), &node.author_email) {
                            Some(texture) => {
                                graph_painter.image(
                                    texture,
                                    Rect::from_center_size(node_pos, Vec2::splat(AVATAR_R * 2.0)),
                                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                                    Color32::WHITE,
                                );
                            }
                            None => {
                                graph_painter.circle_filled(node_pos, AVATAR_R, lane_col);
                                graph_painter.text(
                                    node_pos,
                                    egui::Align2::CENTER_CENTER,
                                    theme::author_initials(&node.author),
                                    FontId::proportional(9.0),
                                    theme::contrast_text(lane_col),
                                );
                            }
                        }
                        if dim {
                            graph_painter.circle_filled(node_pos, NODE_R, theme::bg_app().gamma_multiply(0.75));
                        }
                        if is_sel {
                            graph_painter.circle_stroke(
                                node_pos,
                                NODE_R_SEL,
                                Stroke::new(2.0, theme::text_primary()),
                            );
                        }
                        if graph_w > 0.0 {
                            ui.interact(
                                Rect::from_center_size(node_pos, Vec2::splat(NODE_R * 2.0)).intersect(graph_painter.clip_rect()),
                                egui::Id::new(("author_avatar", &node.oid)),
                                Sense::hover(),
                            )
                            .on_hover_text(&node.author);
                        }

                        // Right columns
                        let msg_start = origin.x + commit_offset;
                        let time_start = origin.x + time_offset;
                        let hash_start = origin.x + hash_offset;
                        let hash_right = hash_start + hash_w - 8.0;
                        let time_right = time_start + time_w - 8.0;

                        let dim_col = |c: Color32| c.gamma_multiply(fade);

                        // Hash
                        painter.with_clip_rect(Rect::from_min_max(Pos2::new(hash_start, row_top), Pos2::new(hash_start + hash_w, row_top + ROW_H)).intersect(painter.clip_rect())).text(
                            Pos2::new(hash_right, cy),
                            egui::Align2::RIGHT_CENTER,
                            &node.oid[..7.min(node.oid.len())],
                            FontId::monospace(12.0),
                            dim_col(theme::text_muted()),
                        );
                        // Time
                        painter.with_clip_rect(Rect::from_min_max(Pos2::new(time_start, row_top), Pos2::new(time_start + time_w, row_top + ROW_H)).intersect(painter.clip_rect())).text(
                            Pos2::new(time_right, cy),
                            egui::Align2::RIGHT_CENTER,
                            format_graph_date(node.time, self.preferences.date_format),
                            FontId::proportional(12.0),
                            dim_col(theme::text_muted()),
                        );
                        ui.interact(
                            Rect::from_min_max(
                                Pos2::new(time_start, row_top),
                                Pos2::new(time_start + time_w, row_top + ROW_H),
                            ),
                            egui::Id::new(("time_cell", &node.oid)),
                            Sense::hover(),
                        )
                        .on_hover_text(format!("{} UTC", format_graph_date(node.time, GraphDateFormat::DateTime)));

                        let ref_rects = draw_ref_column(
                            &painter,
                            origin.x + refs_offset,
                            refs_w,
                            row_top,
                            cy,
                            node,
                            lane_col,
                            &graph,
                        );
                        let mut drop_reference = None;
                        for (reference, rect) in ref_rects {
                            let is_branch = reference.starts_with("refs/heads/") || reference.starts_with("refs/remotes/");
                            if is_branch && ui.input(|i| i.pointer.hover_pos().is_some_and(|p| rect.contains(p))) { drop_reference = Some(reference.clone()); }
                            let response = ui.interact(rect, egui::Id::new(("graph_ref", &reference)), Sense::click_and_drag());
                            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), format!("Reference {reference}. Click to select, double-click to check out a branch, right-click for actions.")));
                            if response.has_focus() { painter.rect_stroke(rect.expand(2.0), 4.0, Stroke::new(1.5, theme::accent_blue()), egui::StrokeKind::Middle); }
                            if is_branch && response.drag_started() && self.pending_action.is_none() {
                                let mut s = self.state.lock().unwrap();
                                if matches!(s.modal, crate::state::ModalKind::None) {
                                    if let Some(path) = &s.repo_path { match crate::git::graph_action::Drag::capture(path, s.repo_epoch, crate::git::graph_action::Endpoint { oid: node.oid.clone(), reference: Some(reference.clone()) }) { Ok(drag) => self.drag = Some(drag), Err(error) => s.error = Some(error) } }
                                }
                            }
                            if response.double_clicked() {
                                use crate::ui::sidebar::SidebarAction;
                                if let Some(name) = reference.strip_prefix("refs/heads/") {
                                    if self.state.lock().unwrap().head_branch.as_deref() != Some(name) {
                                        self.sidebar_action = Some(SidebarAction::CheckoutBranch(name.into()));
                                    }
                                } else if let Some(name) = reference.strip_prefix("refs/remotes/") {
                                    self.sidebar_action = Some(SidebarAction::CheckoutRemoteBranch(name.into()));
                                }
                            } else if response.clicked() {
                                new_selection = Some(Some(node.oid.clone()));
                                new_multi = Some(Vec::new());
                            }
                            response.context_menu(|ui| {
                                if let Some(action) = crate::ui::sidebar::graph_reference_menu(ui, &reference, &node.oid, &self.state) {
                                    self.sidebar_action = Some(action);
                                }
                            });
                            response.on_hover_text(format!("{reference}\nClick to select; double-click a branch to check out; right-click for actions; drag a branch for an operation preview"));
                        }


                        let hover_drop = self.drag.as_ref().is_some_and(|drag| drag.source.oid != node.oid)
                            && ui.input(|input| input.pointer.hover_pos().is_some_and(|p| row_rect.contains(p)));
                        if hover_drop {
                            painter.rect_stroke(row_rect.shrink(1.0), 0.0, Stroke::new(2.0, theme::accent_teal()), egui::StrokeKind::Middle);
                            if ui.input(|input| input.pointer.any_released()) {
                                if let Some(drag) = self.drag.take() {
                                    let head_ref = self.state.lock().unwrap().head_branch.as_ref().map(|name| format!("refs/heads/{name}"));
                                    let reference = drop_reference.or_else(|| head_ref.filter(|name| graph.ref_targets.get(name) == Some(&node.oid)));
                                    let destination = crate::git::graph_action::Endpoint { oid: node.oid.clone(), reference };
                                    match drag.finish(destination) {
                                        Ok(preview) => self.pending_action = Some(preview),
                                        Err(error) => self.state.lock().unwrap().error = Some(error),
                                    }
                                }
                            }
                        }

                        // Commit summary
                        let summary_start = msg_start + 8.0;
                        let summary_end = msg_start + commit_w - 8.0;
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
                        let in_multi = multi.len() > 1 && multi.contains(&node.oid);
                        if row_resp.secondary_clicked() {
                            if !in_multi {
                                new_selection = Some(Some(node.oid.clone()));
                                new_multi = Some(Vec::new());
                            }
                            row_resp.request_focus();
                            self.keyboard_row = Some(node.oid.clone());
                            self.state.lock().unwrap().focused_pane = 2;
                        }
                        row_resp.context_menu(|ui| {
                            if in_multi {
                                if multi_selection_actions(ui, &self.state) {
                                    ui.close();
                                }
                                return;
                            }
                            let oid = &node.oid;
                            let short = &oid[..7.min(oid.len())];
                            if ui.button(format!("Copy SHA  {}", short)).clicked() {
                                ui.ctx().copy_text(oid.clone());
                                ui.close();
                            }
                            ui.separator();
                            if ui.button("Cherry-pick").clicked() {
                                self.preview_endpoint(crate::git::graph_action::Endpoint { oid: oid.clone(), reference: None }, crate::git::graph_action::Action::CherryPick);
                                ui.close();
                            }
                            if ui.button("Revert Commit").clicked() {
                                self.preview_endpoint(crate::git::graph_action::Endpoint { oid: oid.clone(), reference: None }, crate::git::graph_action::Action::Revert);
                                ui.close();
                            }
                            if ui.button("Interactive Rebase from here…").clicked() {
                                self.state.lock().unwrap().pending_open_rebase = Some(oid.clone());
                                ui.close();
                            }
                            ui.separator();
                            if ui.button("Create Branch Here").clicked() {
                                self.state.lock().unwrap().modal =
                                    crate::state::ModalKind::CreateBranch {
                                        from_oid: Some(oid.clone()),
                                    };
                                ui.close();
                            }
                            if ui.button("Create Tag Here").clicked() {
                                self.state.lock().unwrap().modal =
                                    crate::state::ModalKind::CreateTag {
                                        from_oid: Some(oid.clone()),
                                    };
                                ui.close();
                            }
                            ui.separator();
                            if ui.button("Checkout (detached HEAD)…").clicked() {
                                self.state.lock().unwrap().modal =
                                    crate::state::ModalKind::ConfirmDetachedCheckout {
                                        oid: oid.clone(),
                                    };
                                ui.close();
                            }
                            if ui.button("Reset current branch to here…").clicked() {
                                self.state.lock().unwrap().modal =
                                    crate::state::ModalKind::ResetToCommit { oid: oid.clone() };
                                ui.close();
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
                                        ui.close();
                                    }
                                }
                                if ui.button("Clear Compare Base").clicked() {
                                    self.state.lock().unwrap().compare_base_oid = None;
                                    ui.close();
                                }
                            } else {
                                if ui.button("Set as Compare Base").clicked() {
                                    self.state.lock().unwrap().compare_base_oid = Some(oid.clone());
                                    ui.close();
                                }
                            }
                            ui.separator();
                            let bisecting = self.state.lock().unwrap().bisect_in_progress;
                            if bisecting {
                                for (label, step) in [
                                    ("Bisect: Mark Good", crate::git::bisect::Step::Good),
                                    ("Bisect: Mark Bad", crate::git::bisect::Step::Bad),
                                ] {
                                    if ui.button(label).clicked() {
                                        crate::git::bisect::spawn_step(step, Some(oid.clone()), self.state.clone());
                                        ui.close();
                                    }
                                }
                            }
                            if ui.button(if bisecting { "Bisect status…" } else { "Bisect…" }).clicked() {
                                self.state.lock().unwrap().modal = crate::state::ModalKind::Bisect;
                                ui.close();
                            }
                            ui.separator();
                        if ui.button("Deselect").clicked() {
                            new_selection = Some(None);
                            ui.close();
                        }
                    });
                    }

                    if nodes.is_empty()
                        && graph.head_oid.is_none()
                        && graph.ref_targets.is_empty()
                        && !graph.has_more
                        && self.state.lock().unwrap().repo_load_path.is_none()
                    {
                        crate::ui::kit::empty_state(
                            ui,
                            crate::icons::COMMIT,
                            "No commits yet",
                            "Stage some files in Changes and create your first commit.",
                            None,
                        );
                    }
                });
        });

        if ui.input(|input| input.pointer.any_released() || input.key_pressed(Key::Escape)) {
            self.drag = None;
        }

        // Apply selection change
        if let Some(new_sel) = new_selection {
            let trigger = new_sel.clone();
            {
                let mut s = self.state.lock().unwrap();
                s.selected_commits = new_multi.unwrap_or_default();
                let changed = s.selected_commit != new_sel;
                s.selected_commit = new_sel.clone();
                if new_sel.is_some() && clears_file_for_commit(changed, s.diff_context) {
                    // A working-tree diff, or another commit's file, would be
                    // shown beside this commit's details. Cancel its load too.
                    s.diff_load_id += 1;
                    s.selected_file = None;
                    s.current_diff = None;
                }
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
            ui.add(crate::ui::kit::text_field(&mut self.ref_query).hint_text("Find branch or tag"));
            egui::ScrollArea::vertical()
                .id_salt("graph_ref_filters")
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
                crate::ui::kit::text_field(&mut self.range_start).hint_text("Start ref or commit"),
            );
            ui.add(crate::ui::kit::text_field(&mut self.range_end).hint_text("End ref or commit"));
            if ui
                .add_enabled(
                    !self.range_start.trim().is_empty() && !self.range_end.trim().is_empty(),
                    Button::new("Apply range"),
                )
                .clicked()
            {
                filter.range = Some((self.range_start.trim().into(), self.range_end.trim().into()));
                filter.solo_ref = None;
                ui.close();
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

    fn preferences_menu(&mut self, ui: &mut Ui) {
        let before = self.preferences.clone();
        ui.label("Visible columns (Commit is always shown)");
        for column in GraphColumn::ALL {
            let mut visible = self.preferences.columns.contains(&column);
            if ui
                .add_enabled(
                    column != GraphColumn::Commit,
                    egui::Checkbox::new(&mut visible, column.label()),
                )
                .changed()
            {
                if visible {
                    self.preferences.columns.push(column);
                } else {
                    self.preferences.columns.retain(|item| *item != column);
                }
            }
        }
        ui.separator();
        ui.label("Column order");
        let columns = self.preferences.columns.clone();
        for (index, column) in columns.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(column.label());
                if ui
                    .add_enabled(index > 0, Button::new("Move left"))
                    .clicked()
                {
                    self.preferences.columns.swap(index, index - 1);
                }
                if ui
                    .add_enabled(index + 1 < columns.len(), Button::new("Move right"))
                    .clicked()
                {
                    self.preferences.columns.swap(index, index + 1);
                }
            });
        }
        ui.separator();
        ui.label("Date format");
        crate::ui::kit::radio_value(
            ui,
            &mut self.preferences.date_format,
            GraphDateFormat::Relative,
            "Relative (2h ago)",
        );
        crate::ui::kit::radio_value(
            ui,
            &mut self.preferences.date_format,
            GraphDateFormat::Date,
            "Date (YYYY-MM-DD, UTC)",
        );
        crate::ui::kit::radio_value(
            ui,
            &mut self.preferences.date_format,
            GraphDateFormat::DateTime,
            "Date and time (UTC)",
        );
        ui.separator();
        if ui.button("Reset columns and date format").clicked() {
            self.preferences = GraphPreferences::default();
            self.reset_column_widths();
            self.cols_dirty = true;
        }
        if self.preferences != before {
            self.preferences_dirty = true;
        }
    }

    fn time_min_width(&self) -> f32 {
        match self.preferences.date_format {
            GraphDateFormat::Relative => TIME_W_DEF,
            GraphDateFormat::Date => 94.0,
            GraphDateFormat::DateTime => 140.0,
        }
    }

    fn column_width(&self, column: GraphColumn) -> f32 {
        match column {
            GraphColumn::References => self.refs_w,
            GraphColumn::Graph => self.graph_w,
            GraphColumn::Commit => self.commit_w,
            GraphColumn::Time => self.time_w.max(self.time_min_width()),
            GraphColumn::Hash => self.hash_w,
        }
    }

    fn column_layout(&self) -> ([f32; 5], [f32; 5]) {
        let mut starts = [0.0; 5];
        let mut widths = [0.0; 5];
        let mut x = 0.0;
        for column in &self.preferences.columns {
            starts[column.index()] = x;
            widths[column.index()] = self.column_width(*column);
            x += widths[column.index()];
        }
        (starts, widths)
    }

    fn fit_columns(&mut self, avail: f32) {
        self.refs_w = self.refs_w.clamp(REFS_W_MIN, REFS_W_MAX);
        self.graph_w = self.graph_w.clamp(GRAPH_W_MIN, GRAPH_W_MAX);
        self.time_w = self.time_w.clamp(TIME_W_MIN, 240.0);
        self.hash_w = self.hash_w.clamp(HASH_W_MIN, 120.0);
        let fixed: f32 = self
            .preferences
            .columns
            .iter()
            .filter(|c| **c != GraphColumn::Commit)
            .map(|column| self.column_width(*column))
            .sum();
        self.commit_w = (avail - fixed).max(COMMIT_W_MIN);
    }

    fn set_width(&mut self, column: GraphColumn, width: f32) {
        match column {
            GraphColumn::References => self.refs_w = width.clamp(REFS_W_MIN, REFS_W_MAX),
            GraphColumn::Graph => {
                self.graph_w = width.clamp(GRAPH_W_MIN, GRAPH_W_MAX);
                self.graph_w_auto = false;
            }
            GraphColumn::Commit => self.commit_w = width.max(COMMIT_W_MIN),
            GraphColumn::Time => self.time_w = width.clamp(self.time_min_width(), 240.0),
            GraphColumn::Hash => self.hash_w = width.clamp(HASH_W_MIN, 120.0),
        }
    }

    fn resize_handle(
        &mut self,
        ui: &mut Ui,
        header_rect: Rect,
        x: f32,
        column: GraphColumn,
        next: GraphColumn,
        auto_w: f32,
    ) {
        let rect = Rect::from_min_size(
            Pos2::new(x - RESIZE_HIT_W / 2.0, header_rect.min.y),
            Vec2::new(RESIZE_HIT_W, HEADER_H),
        );
        let resp = ui
            .interact(
                rect,
                egui::Id::new(("graph_resize", column.index())),
                Sense::click_and_drag(),
            )
            .on_hover_text("Drag to resize; double-click to fit content");
        let active = resp.hovered() || resp.dragged();
        if active {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeColumn);
        }
        ui.painter().line_segment(
            [
                Pos2::new(x, header_rect.min.y + 4.0),
                Pos2::new(x, header_rect.max.y - 4.0),
            ],
            Stroke::new(
                if active { 2.0 } else { 1.0 },
                if active {
                    theme::accent_teal()
                } else {
                    theme::border()
                },
            ),
        );
        if resp.double_clicked() {
            if column == GraphColumn::Commit {
                let default = match next {
                    GraphColumn::References => REFS_W_DEF,
                    GraphColumn::Graph => GRAPH_W_DEF,
                    GraphColumn::Time => self.time_min_width(),
                    GraphColumn::Hash => HASH_W_DEF,
                    GraphColumn::Commit => COMMIT_W_DEF,
                };
                self.set_width(next, default);
            } else {
                self.set_width(column, auto_w);
            }
            self.cols_dirty = true;
        } else if resp.dragged() {
            let delta = resp.drag_delta().x;
            if column == GraphColumn::Commit {
                self.set_width(next, self.column_width(next) - delta);
            } else {
                self.set_width(column, self.column_width(column) + delta);
            }
            self.cols_dirty = true;
        }
    }
}

/// UTC dates are explicit and deterministic, including timestamps before 1970.
/// Selecting a commit keeps its own file open (the selection did not change)
/// but drops a working-tree diff or a file chosen in a different commit.
fn clears_file_for_commit(selection_changed: bool, context: crate::state::DiffContext) -> bool {
    selection_changed || context != crate::state::DiffContext::CommitDiff
}

fn format_graph_date(timestamp: i64, format: GraphDateFormat) -> String {
    if format == GraphDateFormat::Relative {
        return theme::relative_time(timestamp);
    }
    let days = timestamp.div_euclid(86_400);
    let seconds = timestamp.rem_euclid(86_400);
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    let date = format!("{year:04}-{month:02}-{day:02}");
    if format == GraphDateFormat::Date {
        date
    } else {
        format!("{date} {:02}:{:02}", seconds / 3600, seconds % 3600 / 60)
    }
}

/// Result of clicking a row: the focused commit and the multi-selection.
/// Ctrl/Cmd-click toggles a commit; Shift-click selects the rows between the
/// anchor and the clicked row; a plain click selects one commit.
fn click_selection(
    graph: &crate::git::graph::CommitGraph,
    selected: Option<&str>,
    multi: &std::collections::HashSet<String>,
    anchor: Option<&str>,
    clicked: &str,
    modifiers: egui::Modifiers,
) -> (Option<String>, Vec<String>) {
    let row = |oid: &str| graph.oid_to_row.get(oid).copied();
    let in_order = |set: &mut Vec<String>| set.sort_by_key(|oid| row(oid).unwrap_or(usize::MAX));
    if modifiers.shift {
        if let (Some(from), Some(to)) = (anchor.or(selected).and_then(row), row(clicked)) {
            let (low, high) = (from.min(to), from.max(to));
            let set: Vec<String> = graph.nodes[low..=high]
                .iter()
                .map(|n| n.oid.clone())
                .collect();
            let set = if set.len() > 1 { set } else { Vec::new() };
            return (Some(clicked.to_string()), set);
        }
    } else if modifiers.command {
        let mut set: Vec<String> = if multi.len() > 1 {
            multi.iter().cloned().collect()
        } else {
            selected.map(str::to_owned).into_iter().collect()
        };
        if let Some(index) = set.iter().position(|oid| oid == clicked) {
            set.remove(index);
            in_order(&mut set);
            return match set.len() {
                0 => (None, Vec::new()),
                1 => (set.pop(), Vec::new()),
                _ => (set.first().cloned(), set),
            };
        }
        set.push(clicked.to_string());
        in_order(&mut set);
        let set = if set.len() > 1 { set } else { Vec::new() };
        return (Some(clicked.to_string()), set);
    }
    (Some(clicked.to_string()), Vec::new())
}

/// Actions for two or more selected commits, shared by the graph menu and
/// the details panel. Returns true when an action was chosen.
pub fn multi_selection_actions(ui: &mut Ui, state: &SharedState) -> bool {
    let (oids, on_branch) = {
        let s = state.lock().unwrap();
        (s.selected_commits_oldest_first(), s.head_branch.is_some())
    };
    let count = oids.len();
    if count < 2 {
        return false;
    }
    let mut chosen = false;
    ui.label(
        egui::RichText::new(format!("{count} commits selected"))
            .color(theme::text_muted())
            .size(12.0),
    );
    if ui
        .add_enabled(
            on_branch,
            Button::new(format!("Cherry-pick {count} commits")),
        )
        .on_hover_text("Apply these commits to the current branch, oldest first")
        .clicked()
    {
        crate::git::branches::cherry_pick_many(oids.clone(), state.clone());
        chosen = true;
    }
    if ui
        .add_enabled(on_branch, Button::new(format!("Revert {count} commits")))
        .on_hover_text("Create commits that undo these, newest first")
        .clicked()
    {
        crate::git::branches::revert_many(oids.clone(), state.clone());
        chosen = true;
    }
    if ui
        .add_enabled(on_branch, Button::new(format!("Squash {count} commits…")))
        .on_hover_text("Combine adjacent commits on the current branch with an interactive rebase")
        .clicked()
    {
        state.lock().unwrap().pending_squash = Some(oids.clone());
        chosen = true;
    }
    if ui
        .add_enabled(
            on_branch,
            Button::new(format!("Drop {count} commits\u{2026}")),
        )
        .on_hover_text("Review a rebase that removes these commits from the current branch")
        .clicked()
    {
        state.lock().unwrap().pending_drop = Some(oids.clone());
        chosen = true;
    }
    if count == 2 && ui.button("Compare these 2 commits…").clicked() {
        let mut s = state.lock().unwrap();
        s.modal = crate::state::ModalKind::Compare {
            base_oid: oids[0].clone(),
            compare_oid: oids[1].clone(),
        };
        chosen = true;
    }
    ui.separator();
    if ui.button(format!("Copy {count} SHAs")).clicked() {
        ui.ctx().copy_text(oids.join("\n"));
        chosen = true;
    }
    if ui.button("Clear selection").clicked() {
        state.lock().unwrap().selected_commits.clear();
        chosen = true;
    }
    chosen
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
    if parts
        .iter()
        .any(|part| part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    let y: i64 = parts[0].parse().ok()?;
    let m: i64 = parts[1].parse().ok()?;
    let d: i64 = parts[2].parse().ok()?;
    if !(1970..=9999).contains(&y) || !(1..=12).contains(&m) {
        return None;
    }
    // Simplified days-since-epoch calculation (close enough for filtering)
    let leap = |yr: i64| yr % 4 == 0 && (yr % 100 != 0 || yr % 400 == 0);
    let days_in_month = [0i64, 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut month_days = days_in_month[m as usize];
    if m == 2 && leap(y) {
        month_days += 1;
    }
    if !(1..=month_days).contains(&d) {
        return None;
    }
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
    graph: &crate::git::graph::CommitGraph,
) -> Vec<(String, Rect)> {
    let mut rectangles = Vec::new();
    if refs_w < 24.0 {
        return rectangles;
    }
    let clip = Rect::from_min_max(
        Pos2::new(origin_x + 8.0, row_top),
        Pos2::new(origin_x + refs_w - 8.0, row_top + ROW_H),
    )
    .intersect(painter.clip_rect());
    let painter = painter.with_clip_rect(clip);
    let mut x = origin_x + 10.0;
    let mut refs = Vec::new();
    for name in &node.refs {
        for prefix in ["refs/heads/", "refs/remotes/", "refs/tags/"] {
            let reference = format!("{prefix}{name}");
            if graph.ref_targets.get(&reference) == Some(&node.oid)
                && !refs.iter().any(|(r, _, _)| r == &reference)
            {
                refs.push((reference, name, prefix == "refs/tags/"));
            }
        }
    }
    for (reference, name, is_tag) in refs.into_iter().take(3) {
        let remaining = origin_x + refs_w - 10.0 - x;
        if remaining < 28.0 {
            break;
        }
        let color = if is_tag {
            theme::accent_purple()
        } else {
            branch_color
        };
        let font = FontId::proportional(12.0);
        let mut label = if is_tag {
            format!("tag: {name}")
        } else {
            name.to_string()
        };
        let mut galley = painter.layout_no_wrap(label.clone(), font.clone(), color);
        if galley.size().x + 14.0 > remaining {
            while label.chars().count() > 1 {
                label.pop();
                galley = painter.layout_no_wrap(format!("{label}\u{2026}"), font.clone(), color);
                if galley.size().x + 14.0 <= remaining {
                    break;
                }
            }
        }
        let width = (galley.size().x + 14.0).min(remaining);
        let rect = Rect::from_min_size(Pos2::new(x, row_top + 8.0), Vec2::new(width, ROW_H - 16.0));
        painter.rect_filled(rect, 4.0, color.gamma_multiply(0.12));
        painter.rect_stroke(
            rect,
            4.0,
            Stroke::new(0.8, color.gamma_multiply(0.65)),
            egui::StrokeKind::Middle,
        );
        painter.galley(
            Pos2::new(rect.min.x + 7.0, cy - galley.size().y / 2.0),
            galley,
            color,
        );
        rectangles.push((reference, rect));
        x = rect.max.x + 4.0;
    }
    rectangles
}

#[cfg(test)]
mod interaction_tests {
    use super::*;
    use crate::git::test_support::TestRepo;

    #[test]
    fn first_commit_guidance_is_only_shown_for_loaded_empty_history() {
        for (has_head, loading, expected) in [
            (false, false, true),
            (true, false, false),
            (false, true, false),
        ] {
            let ctx = egui::Context::default();
            ctx.set_fonts(crate::icons::font_definitions());
            let state = crate::state::new_shared_state();
            let mut graph = crate::git::graph::CommitGraph::default();
            graph.head_oid = has_head.then(|| "filtered-out-commit".into());
            {
                let mut state = state.lock().unwrap();
                state.commit_graph = Some(std::sync::Arc::new(graph));
                state.repo_load_path = loading.then(|| std::path::PathBuf::from("loading"));
            }
            let mut view = CommitGraphView::new(state);
            let output = frame(&ctx, &mut view, vec![], true);
            let shown = output.shapes.iter().any(|shape| {
                matches!(
                    &shape.shape,
                    egui::Shape::Text(text) if text.galley.text() == "No commits yet"
                )
            });
            assert_eq!(shown, expected);
        }
    }

    fn search_test_graph() -> std::sync::Arc<crate::git::graph::CommitGraph> {
        let node = |oid: &str, summary: &str, author: &str, time, refs: Vec<String>| {
            crate::git::graph::CommitNode {
                oid: oid.into(),
                summary: summary.into(),
                body: None,
                author: author.into(),
                author_email: String::new(),
                time,
                parents: Vec::new(),
                lane: 0,
                refs,
            }
        };
        let mut graph = crate::git::graph::CommitGraph::default();
        graph.nodes = vec![
            node(
                "abc123",
                "Fix search",
                "Alice",
                1_709_164_800,
                vec!["Main".into()],
            ),
            node("def456", "Add feature", "Bob", 0, vec![]),
        ];
        std::sync::Arc::new(graph)
    }

    #[test]
    fn search_cache_matches_node_matches() {
        let graph = search_test_graph();
        let mut cache = None;
        for query in [
            "SEARCH",
            "author:alice",
            "after:2024-01-01",
            "before:2024-01-01",
            "sha:def",
            "main",
            "missing",
        ] {
            let expected: Vec<bool> = graph
                .nodes
                .iter()
                .map(|node| node_matches(node, &query.to_lowercase()))
                .collect();
            let result = cached_search(&mut cache, query, &graph);
            assert_eq!(result.matches.as_ref(), &expected);
            assert_eq!(
                result.count,
                expected.iter().filter(|matched| **matched).count()
            );
        }
    }

    #[test]
    fn search_cache_invalidates_on_query_or_graph_change() {
        let graph = search_test_graph();
        let mut cache = None;
        let first = cached_search(&mut cache, "alice", &graph).matches.clone();
        let unchanged = cached_search(&mut cache, "alice", &graph).matches.clone();
        assert!(std::sync::Arc::ptr_eq(&first, &unchanged));

        let changed_query = cached_search(&mut cache, "bob", &graph).matches.clone();
        assert!(!std::sync::Arc::ptr_eq(&first, &changed_query));
        assert_eq!(changed_query.as_ref(), &vec![false, true]);

        let refreshed = std::sync::Arc::new((*graph).clone());
        let refreshed_matches = cached_search(&mut cache, "bob", &refreshed).matches.clone();
        assert!(!std::sync::Arc::ptr_eq(&changed_query, &refreshed_matches));

        let mut larger = (*graph).clone();
        larger.nodes.push(larger.nodes[0].clone());
        let larger = std::sync::Arc::new(larger);
        let changed_graph = cached_search(&mut cache, "bob", &larger).matches.clone();
        assert!(!std::sync::Arc::ptr_eq(&refreshed_matches, &changed_graph));
        assert_eq!(changed_graph.len(), 3);

        // Check the length component independently of the pointer component.
        cache.as_mut().unwrap().nodes_len -= 1;
        assert!(!cache.as_ref().unwrap().is_valid("bob", &larger));
        let rebuilt = cached_search(&mut cache, "bob", &larger).matches.clone();
        assert!(!std::sync::Arc::ptr_eq(&changed_graph, &rebuilt));
    }

    #[test]
    fn parse_date_rejects_invalid_dates_and_overflowing_inputs() {
        for date in [
            "2024-31-12",
            "2024-14-01",
            "2024-00-10",
            "2024-02-30",
            "2023-02-29",
            "2024-04-31",
            "1969-12-31",
            "10000-01-01",
            "99999999999999999-01-01",
            "2024-1",
            "",
            "2024--01",
        ] {
            assert_eq!(parse_date(date), None, "{date}");
        }
    }

    #[test]
    fn parse_date_returns_midnight_utc_timestamps() {
        assert_eq!(parse_date("1970-01-01"), Some(0));
        assert_eq!(parse_date("2024-02-29"), Some(1_709_164_800));
        assert_eq!(parse_date("2000-03-01"), Some(951_868_800));
        assert_eq!(parse_date("2023-12-31"), Some(1_703_980_800));
    }

    #[test]
    fn node_matches_rejects_invalid_after_date() {
        let node = crate::git::graph::CommitNode {
            oid: String::new(),
            summary: String::new(),
            body: None,
            author: String::new(),
            author_email: String::new(),
            time: 0,
            parents: Vec::new(),
            lane: 0,
            refs: Vec::new(),
        };
        assert!(!node_matches(&node, "after:2024-31-12"));
    }

    #[test]
    fn graph_column_follows_lanes_until_the_user_sizes_it() {
        let mut view = CommitGraphView::new(crate::state::new_shared_state());
        assert!(view.graph_w_auto, "a fresh view fits its lanes");
        assert!(graph_fit_width(10) > GRAPH_W_DEF && graph_fit_width(10) <= GRAPH_W_MAX);
        assert_eq!(graph_fit_width(0), GRAPH_W_MIN.max(graph_fit_width(0)));
        view.set_column_widths(None, Some(150.0), None, None);
        assert!(!view.graph_w_auto, "a saved width is kept");
        assert_eq!(view.graph_w, 150.0);
        // Opening a repository with no saved width must not inherit the last one.
        view.set_column_widths(None, None, None, None);
        assert!(view.graph_w_auto);
        view.set_width(GraphColumn::Graph, 200.0);
        assert!(!view.graph_w_auto, "dragging the column pins its width");
        view.reset_column_widths();
        assert!(
            view.graph_w_auto,
            "resetting columns returns to fitting the lanes"
        );
    }

    #[test]
    fn column_preferences_normalize_and_layout_hidden_reordered_columns() {
        let mut view = CommitGraphView::new(crate::state::new_shared_state());
        view.set_preferences(GraphPreferences {
            columns: vec![GraphColumn::Hash, GraphColumn::Time, GraphColumn::Hash],
            date_format: GraphDateFormat::DateTime,
        });
        assert_eq!(
            view.preferences.columns,
            [GraphColumn::Hash, GraphColumn::Time, GraphColumn::Commit]
        );
        view.fit_columns(1000.0);
        let (starts, widths) = view.column_layout();
        assert_eq!(starts[GraphColumn::Hash.index()], 0.0);
        assert_eq!(starts[GraphColumn::Time.index()], HASH_W_DEF);
        assert_eq!(widths[GraphColumn::References.index()], 0.0);
        assert_eq!(widths[GraphColumn::Graph.index()], 0.0);
        assert_eq!(widths.iter().sum::<f32>(), 1000.0);
        assert!(widths[GraphColumn::Time.index()] >= 140.0);
        assert!(
            view.take_dirty_preferences().is_none(),
            "restoration is not a user edit"
        );
        let encoded = serde_json::to_string(&view.preferences).unwrap();
        assert_eq!(
            serde_json::from_str::<GraphPreferences>(&encoded).unwrap(),
            view.preferences
        );
        view.set_column_widths(Some(250.0), None, None, None);
        view.reset_column_widths();
        assert_eq!(
            view.take_dirty_widths().unwrap().0,
            REFS_W_DEF,
            "reset widths must persist"
        );
    }

    #[test]
    fn graph_dates_are_utc_and_handle_leap_days_and_pre_epoch_times() {
        assert_eq!(format_graph_date(0, GraphDateFormat::Date), "1970-01-01");
        assert_eq!(
            format_graph_date(-1, GraphDateFormat::DateTime),
            "1969-12-31 23:59"
        );
        assert_eq!(
            format_graph_date(951_782_400, GraphDateFormat::Date),
            "2000-02-29"
        );
        assert_eq!(
            format_graph_date(86_460, GraphDateFormat::DateTime),
            "1970-01-02 00:01"
        );
    }

    #[test]
    fn reordered_graph_headers_and_ref_menus_keep_reference_identity() {
        let repo = TestRepo::new();
        let oid = repo.commit("base", b"base").to_string();
        repo.git_ok(&["branch", "same"]);
        repo.git_ok(&["tag", "same"]);
        let (ctx, mut view) = setup(&repo);
        view.set_column_widths(Some(320.0), None, None, None);
        view.set_preferences(GraphPreferences {
            columns: vec![
                GraphColumn::Hash,
                GraphColumn::References,
                GraphColumn::Commit,
            ],
            date_format: GraphDateFormat::Date,
        });
        frame(&ctx, &mut view, vec![], true);
        let header = |column: GraphColumn| {
            ctx.read_response(egui::Id::new(("graph_header", column.index())))
                .unwrap()
                .rect
        };
        assert!(header(GraphColumn::Hash).left() < header(GraphColumn::References).left());
        assert!(ctx
            .read_response(egui::Id::new(("graph_header", GraphColumn::Graph.index())))
            .is_none());
        let tag = ctx
            .read_response(egui::Id::new(("graph_ref", "refs/tags/same")))
            .unwrap()
            .rect
            .center();
        let output = click_with(
            &ctx,
            &mut view,
            tag,
            egui::Modifiers::NONE,
            egui::PointerButton::Secondary,
        );
        let delete = text_position(&output, "Delete Tag…");
        click_with(
            &ctx,
            &mut view,
            delete,
            egui::Modifiers::NONE,
            egui::PointerButton::Primary,
        );
        assert!(
            matches!(view.take_sidebar_action(), Some(crate::ui::sidebar::SidebarAction::DeleteTag(name)) if name == "same")
        );
        assert_eq!(
            repo.head().to_string(),
            oid,
            "menus request guarded actions, without mutating directly"
        );
        assert!(repo.open().find_reference("refs/tags/same").is_ok());
    }
    fn frame(
        ctx: &egui::Context,
        view: &mut CommitGraphView,
        events: Vec<egui::Event>,
        enabled: bool,
    ) -> egui::FullOutput {
        ctx.begin_pass(egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 400.0))),
            events,
            ..Default::default()
        });
        view.set_interactions_enabled(enabled && !view.has_pending_action());
        egui::CentralPanel::default().show(
            &mut egui::Ui::new(
                ctx.clone(),
                egui::Id::new("test_root"),
                egui::UiBuilder::new().max_rect(ctx.content_rect()),
            ),
            |ui| view.show(ui),
        );
        view.show_action_confirmation(ctx, &[]);
        let mut output = ctx.end_pass();
        output.textures_delta.clear();
        output
    }
    fn key(key: Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }
    }
    fn pointer(pos: Pos2, button: egui::PointerButton, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: Default::default(),
        }
    }
    fn setup(repo: &TestRepo) -> (egui::Context, CommitGraphView) {
        let state = crate::state::new_shared_state();
        {
            let mut s = state.lock().unwrap();
            s.repo_path = Some(repo.path().canonicalize().unwrap());
            s.head_branch = Some("main".into());
            s.focused_pane = 2;
            s.commit_graph = Some(std::sync::Arc::new(
                crate::git::graph::CommitGraph::from_repo(&repo.open()).unwrap(),
            ));
        }
        (egui::Context::default(), CommitGraphView::new(state))
    }
    fn text_position(output: &egui::FullOutput, label: &str) -> Pos2 {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == label => {
                    Some(text.galley.rect.translate(text.pos.to_vec2()).center())
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing text: {label}"))
    }

    fn click_with(
        ctx: &egui::Context,
        view: &mut CommitGraphView,
        pos: Pos2,
        modifiers: egui::Modifiers,
        button: egui::PointerButton,
    ) -> egui::FullOutput {
        let event = |pressed| egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers,
        };
        let mut last = None;
        for events in [
            vec![
                egui::Event::ModifiersChanged(modifiers),
                egui::Event::PointerMoved(pos),
                event(true),
            ],
            vec![event(false)],
            vec![],
        ] {
            ctx.begin_pass(egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 400.0))),
                events,
                ..Default::default()
            });
            view.set_interactions_enabled(true);
            egui::CentralPanel::default().show(
                &mut egui::Ui::new(
                    ctx.clone(),
                    egui::Id::new("test_root"),
                    egui::UiBuilder::new().max_rect(ctx.content_rect()),
                ),
                |ui| view.show(ui),
            );
            let mut output = ctx.end_pass();
            output.textures_delta.clear();
            last = Some(output);
        }
        last.unwrap()
    }

    #[test]
    fn ctrl_and_shift_clicks_build_a_multi_selection_with_actions() {
        let repo = TestRepo::new();
        let a = repo.commit("file", b"a").to_string();
        let b = repo.commit("file", b"b").to_string();
        let c = repo.commit("file", b"c").to_string();
        let (ctx, mut view) = setup(&repo);
        ctx.set_fonts(crate::icons::font_definitions());
        frame(&ctx, &mut view, vec![], true);
        let row = |oid: &str| {
            ctx.read_response(egui::Id::new(("commit_row", oid.to_string())))
                .unwrap()
                .rect
                .center()
        };
        let (pa, pc) = (row(&a), row(&c));
        let primary = egui::PointerButton::Primary;
        click_with(&ctx, &mut view, pc, egui::Modifiers::NONE, primary);
        assert_eq!(
            view.state.lock().unwrap().selected_commit.as_deref(),
            Some(c.as_str())
        );
        assert!(view.state.lock().unwrap().selected_commits.is_empty());

        click_with(&ctx, &mut view, pa, egui::Modifiers::COMMAND, primary);
        assert_eq!(
            view.state.lock().unwrap().selected_commits_oldest_first(),
            [a.clone(), c.clone()]
        );

        // The Ctrl-clicked row becomes the anchor, so this selects A through C.
        click_with(&ctx, &mut view, pc, egui::Modifiers::SHIFT, primary);
        assert_eq!(
            view.state.lock().unwrap().selected_commits_oldest_first(),
            [a.clone(), b.clone(), c.clone()]
        );

        // Right-clicking a selected row offers actions for the whole selection.
        let output = click_with(
            &ctx,
            &mut view,
            row(&b),
            egui::Modifiers::NONE,
            egui::PointerButton::Secondary,
        );
        text_position(&output, "Cherry-pick 3 commits");
        text_position(&output, "Squash 3 commits…");
        assert_eq!(view.state.lock().unwrap().selected_commits.len(), 3);
        frame(&ctx, &mut view, vec![key(Key::Escape)], true);
        frame(&ctx, &mut view, vec![], true);

        // Ctrl-click removes one; a plain click goes back to one commit.
        // Click near the row's left edge, away from where the menu was.
        let pb = Pos2::new(40.0, row(&b).y);
        click_with(&ctx, &mut view, pb, egui::Modifiers::COMMAND, primary);
        assert_eq!(
            view.state.lock().unwrap().selected_commits_oldest_first(),
            [a.clone(), c.clone()]
        );
        click_with(&ctx, &mut view, pb, egui::Modifiers::NONE, primary);
        let s = view.state.lock().unwrap();
        assert!(s.selected_commits.is_empty());
        assert_eq!(s.selected_commit.as_deref(), Some(b.as_str()));
    }

    #[test]
    fn selecting_a_commit_drops_the_working_tree_diff() {
        use crate::state::DiffContext;
        let repo = TestRepo::new();
        let a = repo.commit("file", b"a").to_string();
        let b = repo.commit("file", b"b").to_string();
        let (ctx, mut view) = setup(&repo);
        ctx.set_fonts(crate::icons::font_definitions());
        frame(&ctx, &mut view, vec![], true);
        {
            let mut s = view.state.lock().unwrap();
            s.selected_commit = None;
            s.selected_file = Some("file".into());
            s.diff_context = DiffContext::UnstagedFile;
        }
        let row = |oid: &str| {
            ctx.read_response(egui::Id::new(("commit_row", oid.to_string())))
                .unwrap()
                .rect
                .center()
        };
        let primary = egui::PointerButton::Primary;
        click_with(&ctx, &mut view, row(&a), egui::Modifiers::NONE, primary);
        {
            let mut s = view.state.lock().unwrap();
            assert_eq!(s.selected_commit.as_deref(), Some(a.as_str()));
            assert!(s.selected_file.is_none() && s.current_diff.is_none());
            // A file opened from this commit's details survives re-selecting it...
            s.selected_file = Some("file".into());
            s.diff_context = DiffContext::CommitDiff;
        }
        click_with(&ctx, &mut view, row(&a), egui::Modifiers::NONE, primary);
        assert_eq!(
            view.state.lock().unwrap().selected_file.as_deref(),
            Some("file")
        );
        // ...but not moving to another commit.
        click_with(&ctx, &mut view, row(&b), egui::Modifiers::NONE, primary);
        assert!(view.state.lock().unwrap().selected_file.is_none());
    }

    #[test]
    fn clears_file_unless_the_same_commit_keeps_its_own_diff() {
        use crate::state::DiffContext::*;
        assert!(!clears_file_for_commit(false, CommitDiff));
        assert!(clears_file_for_commit(true, CommitDiff));
        assert!(clears_file_for_commit(false, UnstagedFile));
        assert!(clears_file_for_commit(false, StagedFile));
    }

    #[test]
    fn context_menu_uses_clicked_oid_and_confirmation_keeps_it_after_selection_changes() {
        use crate::git::graph_action::Action;
        for (label, action) in [
            ("Cherry-pick", Action::CherryPick),
            ("Revert Commit", Action::Revert),
        ] {
            let repo = TestRepo::new();
            let base = repo.commit("base", b"base");
            repo.git_ok(&["checkout", "-b", "topic"]);
            let topic = repo.commit("topic", b"topic");
            repo.git_ok(&["checkout", "main"]);
            let (ctx, mut view) = setup(&repo);
            view.state.lock().unwrap().selected_commit = Some(base.to_string());
            let index = std::fs::read(repo.path().join(".git/index")).unwrap();
            frame(&ctx, &mut view, vec![], true);
            let pos = ctx
                .read_response(egui::Id::new(("commit_row", topic.to_string())))
                .unwrap()
                .rect
                .center();
            frame(
                &ctx,
                &mut view,
                vec![
                    egui::Event::PointerMoved(pos),
                    pointer(pos, egui::PointerButton::Secondary, true),
                ],
                true,
            );
            frame(
                &ctx,
                &mut view,
                vec![pointer(pos, egui::PointerButton::Secondary, false)],
                true,
            );
            let output = frame(&ctx, &mut view, vec![], true);
            let button = text_position(&output, label);
            // Change selection even while the menu is open: its endpoint remains B.
            view.state.lock().unwrap().selected_commit = Some(base.to_string());
            frame(
                &ctx,
                &mut view,
                vec![
                    egui::Event::PointerMoved(button),
                    pointer(button, egui::PointerButton::Primary, true),
                ],
                true,
            );
            frame(
                &ctx,
                &mut view,
                vec![pointer(button, egui::PointerButton::Primary, false)],
                true,
            );
            let preview = view
                .pending_action
                .as_ref()
                .expect("menu action opens confirmation");
            assert_eq!(preview.source.oid, topic.to_string());
            assert_eq!(preview.action, action);
            view.state.lock().unwrap().selected_commit = Some(base.to_string());
            frame(&ctx, &mut view, vec![], true);
            assert_eq!(
                view.pending_action.as_ref().unwrap().source.oid,
                topic.to_string()
            );
            assert_eq!(repo.head(), base);
            assert!(!repo.path().join("topic").exists());
            assert_eq!(
                std::fs::read(repo.path().join(".git/index")).unwrap(),
                index
            );
            assert!(view.state.lock().unwrap().local_jobs.is_empty());
            frame(&ctx, &mut view, vec![key(Key::Escape)], true);
            assert!(view.pending_action.is_none());
            assert_eq!(repo.head(), base);
        }
    }

    #[test]
    fn search_focus_is_requested_once_and_escape_respects_confirmation() {
        let repo = TestRepo::new();
        repo.commit("base", b"base");
        let head = repo.commit("change", b"change");
        let (ctx, mut view) = setup(&repo);
        view.state.lock().unwrap().search_active = true;
        frame(&ctx, &mut view, vec![], true);
        assert_eq!(
            ctx.memory(|m| m.focused()),
            Some(egui::Id::new("graph_search"))
        );
        frame(&ctx, &mut view, vec![key(Key::Tab)], true);
        frame(&ctx, &mut view, vec![], true);
        assert_ne!(
            ctx.memory(|m| m.focused()),
            Some(egui::Id::new("graph_search"))
        );
        view.state.lock().unwrap().selected_commit = Some(head.to_string());
        view.preview_selected(crate::git::graph_action::Action::Revert);
        frame(&ctx, &mut view, vec![], true);
        frame(&ctx, &mut view, vec![key(Key::Escape)], true);
        assert!(view.pending_action.is_none());
        assert!(
            view.state.lock().unwrap().search_active,
            "Escape cancels the topmost confirmation only"
        );
    }

    #[test]
    fn modal_blocks_graph_arrows_and_offscreen_keyboard_target_keeps_focus() {
        let repo = TestRepo::new();
        for i in 0..32 {
            repo.commit(&format!("file-{i}"), b"data");
        }
        let (ctx, mut view) = setup(&repo);
        frame(&ctx, &mut view, vec![], true);
        frame(&ctx, &mut view, vec![key(Key::ArrowUp)], false);
        assert!(view.state.lock().unwrap().selected_commit.is_none());
        frame(&ctx, &mut view, vec![key(Key::ArrowUp)], true);
        let oid = view.state.lock().unwrap().selected_commit.clone().unwrap();
        let expected = view
            .state
            .lock()
            .unwrap()
            .commit_graph
            .as_ref()
            .unwrap()
            .nodes
            .last()
            .unwrap()
            .oid
            .clone();
        assert_eq!(oid, expected);
        let id = egui::Id::new(("commit_row", &oid));
        assert_eq!(ctx.memory(|m| m.focused()), Some(id));
        frame(&ctx, &mut view, vec![], true);
        assert_eq!(
            ctx.memory(|m| m.focused()),
            Some(id),
            "virtualization must retain a focused row during scrolling"
        );
    }

    #[test]
    fn drag_commit_previews_without_mutating_and_escape_cancels() {
        let repo = TestRepo::new();
        let base = repo.commit("base", b"base");
        repo.git_ok(&["checkout", "-b", "topic"]);
        let topic = repo.commit("topic", b"topic");
        repo.git_ok(&["checkout", "main"]);
        let state = crate::state::new_shared_state();
        {
            let mut s = state.lock().unwrap();
            s.repo_path = Some(repo.path().canonicalize().unwrap());
            s.head_branch = Some("main".into());
            s.commit_graph = Some(std::sync::Arc::new(
                crate::git::graph::CommitGraph::from_repo(&repo.open()).unwrap(),
            ));
        }
        let ctx = egui::Context::default();
        let mut view = CommitGraphView::new(state.clone());
        let frame = |view: &mut CommitGraphView, events| {
            ctx.begin_pass(egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 700.0))),
                events,
                ..Default::default()
            });
            view.set_interactions_enabled(!view.has_pending_action());
            egui::CentralPanel::default().show(
                &mut egui::Ui::new(
                    ctx.clone(),
                    egui::Id::new("test_root"),
                    egui::UiBuilder::new().max_rect(ctx.content_rect()),
                ),
                |ui| view.show(ui),
            );
            view.show_action_confirmation(&ctx, &[]);
            let mut output = ctx.end_pass();
            output.textures_delta.clear();
            output
        };
        let _ = frame(&mut view, vec![]);
        let source = ctx
            .read_response(egui::Id::new(("commit_row", &topic.to_string())))
            .unwrap()
            .rect
            .center();
        let target = ctx
            .read_response(egui::Id::new(("commit_row", &base.to_string())))
            .unwrap()
            .rect
            .center();
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        let _ = frame(
            &mut view,
            vec![egui::Event::PointerMoved(source), button(source, true)],
        );
        let _ = frame(
            &mut view,
            vec![egui::Event::PointerMoved(source + Vec2::new(10.0, 0.0))],
        );
        assert!(view.drag.is_some());
        let _ = frame(&mut view, vec![egui::Event::PointerMoved(target)]);
        let _ = frame(&mut view, vec![button(target, false)]);
        assert!(
            view.pending_action.is_some(),
            "drop should open a preview: {:?}",
            state.lock().unwrap().error
        );
        assert_eq!(repo.head(), base);
        assert!(state.lock().unwrap().local_jobs.is_empty());
        let _ = frame(
            &mut view,
            vec![egui::Event::Key {
                key: Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        assert!(view.pending_action.is_none());
        assert_eq!(repo.head(), base);

        // Keyboard invocation uses the same guarded preview and takes focus on
        // Cancel, so Enter cannot activate whichever background widget had focus.
        state.lock().unwrap().selected_commit = Some(topic.to_string());
        view.preview_selected(crate::git::graph_action::Action::CherryPick);
        let _ = frame(&mut view, vec![]);
        let _ = frame(
            &mut view,
            vec![egui::Event::Key {
                key: Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        assert!(view.pending_action.is_none());
        assert_eq!(repo.head(), base);

        // Clicking the shield must not lift it above the window and prevent
        // subsequent clicks on dialog buttons.
        view.preview_selected(crate::git::graph_action::Action::CherryPick);
        let _ = frame(&mut view, vec![]);
        let outside = Pos2::new(5.0, 5.0);
        let _ = frame(
            &mut view,
            vec![egui::Event::PointerMoved(outside), button(outside, true)],
        );
        let output = frame(&mut view, vec![button(outside, false)]);
        assert!(view.pending_action.is_some());
        let cancel = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == "Cancel" => {
                    Some(text.galley.rect.translate(text.pos.to_vec2()).center())
                }
                _ => None,
            })
            .expect("confirmation renders Cancel");
        let _ = frame(
            &mut view,
            vec![egui::Event::PointerMoved(cancel), button(cancel, true)],
        );
        let _ = frame(&mut view, vec![button(cancel, false)]);
        assert!(view.pending_action.is_none());
        assert_eq!(repo.head(), base);

        view.preview_selected(crate::git::graph_action::Action::CherryPick);
        state.lock().unwrap().pending_discard = Some(crate::state::PendingDiscard::ResetHard);
        let _ = frame(&mut view, vec![]);
        assert!(view.pending_action.is_none());
    }
}
