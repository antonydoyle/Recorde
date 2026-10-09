use egui::{Color32, FontId, Pos2, Rect, Sense, Stroke, Ui, Vec2};
use std::sync::Arc;

use crate::git::operations::{load_diff, load_workdir_diff};
use crate::git::staging;
use crate::state::{DiffContext, FileDiff, PendingDiscard, SharedState};
use crate::theme;

const ROW_HEIGHT: f32 = 19.0;
const FONT_SIZE: f32 = 12.0;
const U_GUTTER: f32 = 76.0;
const BAR_W: f32 = 3.0;
const S_GUTTER: f32 = 44.0;

// ── Unified items ─────────────────────────────────────────────────────────────

#[derive(Clone)]
enum Item {
    HunkHeader {
        text: String,
        hunk_index: usize,
    },
    Line {
        emphasis: Option<std::ops::Range<usize>>,
        origin: char,
        old_no: String,
        new_no: String,
        content: String,
        hunk_index: usize,
        line_index: usize,
    },
    Binary,
}

// ── Split items ───────────────────────────────────────────────────────────────

#[derive(Clone)]
struct SideLine {
    emphasis: Option<std::ops::Range<usize>>,
    unified_row: usize,
    lineno: String,
    content: String,
    changed: bool,
}

#[derive(Clone)]
enum SplitRow {
    HunkHeader(String),
    Pair {
        left: Option<SideLine>,
        right: Option<SideLine>,
    },
}

// ── Actions emitted by the diff renderer ─────────────────────────────────────

#[derive(Debug)]
enum DiffAction {
    DiscardSelection(Vec<(usize, usize)>),
    ApplySelection(Vec<(usize, usize)>, bool),
    StageHunk(usize),
    UnstageHunk(usize),
    DiscardHunk(usize),
    StageLine(usize, usize),
    UnstageLine(usize, usize),
}

// ── DiffView ──────────────────────────────────────────────────────────────────

pub struct DiffView {
    state: SharedState,
    items: Vec<Item>,
    split_rows: Vec<SplitRow>,
    last_diff: Option<Arc<FileDiff>>,
    binary_textures: [Option<egui::TextureHandle>; 2],
    last_context: DiffContext,
    last_repo_epoch: u64,
    selected_rows: std::collections::BTreeSet<usize>,
    selection_anchor: Option<usize>,
    search_open: bool,
    search_query: String,
    search_row: Option<usize>,
    show_console: bool,
    split_view: bool,
    show_whitespace: bool,
    show_blame: bool,
}

impl DiffView {
    pub fn preferences(&self) -> crate::config::DiffPreferences {
        crate::config::DiffPreferences {
            whitespace: self.show_whitespace,
            split: self.split_view,
            search_open: self.search_open,
            search_query: self.search_query.clone(),
        }
    }

    pub fn restore_preferences(&mut self, preferences: &crate::config::DiffPreferences) {
        self.split_view = preferences.split;
        self.show_whitespace = preferences.whitespace;
        self.search_open = preferences.search_open;
        self.search_query = preferences.search_query.clone();
        self.search_row = None;
        self.show_console = false;
    }

    pub fn new(state: SharedState) -> Self {
        Self {
            state,
            items: Vec::new(),
            split_rows: Vec::new(),
            last_diff: None,
            binary_textures: [None, None],
            last_context: DiffContext::default(),
            last_repo_epoch: 0,
            selected_rows: Default::default(),
            selection_anchor: None,
            search_open: false,
            search_query: String::new(),
            search_row: None,
            show_console: false,
            split_view: false,
            show_whitespace: false,
            show_blame: false,
        }
    }

    pub fn show(&mut self, ui: &mut Ui) {
        let (
            diff_opt,
            selected_file,
            console_log,
            commit_files,
            workdir_files,
            selected_commit,
            diff_context,
            blame_lines,
            state_show_blame,
            repo_epoch,
        ) = {
            let s = self.state.lock().unwrap();
            (
                s.current_diff.clone(),
                s.selected_file.clone(),
                s.console_log.clone(),
                s.selected_commit_detail
                    .as_ref()
                    .map(|d| d.changed_files.clone()),
                s.working_dir_files.clone(),
                s.selected_commit.clone(),
                s.diff_context,
                s.blame_lines.clone(),
                s.show_blame,
                s.repo_epoch,
            )
        };
        // Sync show_blame from state (set externally when user clicks "Blame")
        if state_show_blame && !self.show_blame {
            self.show_console = false;
        }
        self.show_blame = state_show_blame;

        ui.painter()
            .rect_filled(ui.max_rect(), 0.0, theme::bg_app());

        let file_list_w = 230.0;
        let full_rect = ui.max_rect();
        let list_rect = Rect::from_min_max(
            full_rect.min,
            egui::pos2(full_rect.min.x + file_list_w, full_rect.max.y),
        );
        let diff_rect = Rect::from_min_max(
            egui::pos2(full_rect.min.x + file_list_w, full_rect.min.y),
            full_rect.max,
        );

        ui.painter().line_segment(
            [
                egui::pos2(full_rect.min.x + file_list_w, full_rect.min.y),
                egui::pos2(full_rect.min.x + file_list_w, full_rect.max.y),
            ],
            Stroke::new(1.0, theme::border()),
        );

        // ── Left file list ────────────────────────────────────────────────
        ui.allocate_ui_at_rect(list_rect, |ui| {
            ui.painter()
                .rect_filled(ui.max_rect(), 0.0, theme::bg_panel());
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    ui.add_space(4.0);
                    if let Some(ref files) = commit_files {
                        let oid = selected_commit.clone().unwrap_or_default();
                        ui.horizontal(|ui| {
                            ui.add_space(8.0);
                            ui.label(
                                egui::RichText::new(format!("Changed files ({})", files.len()))
                                    .color(theme::text_muted())
                                    .size(10.5)
                                    .strong(),
                            );
                        });
                        ui.add_space(2.0);
                        for f in files {
                            let is_sel = selected_file.as_deref() == Some(f.path.as_str());
                            let (icon, col) = status_icon(f.status);
                            let mut clicked = false;
                            ui.horizontal(|ui| {
                                ui.add_space(8.0);
                                ui.label(
                                    egui::RichText::new(icon)
                                        .color(col)
                                        .size(11.0)
                                        .font(FontId::monospace(11.0)),
                                )
                                .on_hover_text(status_tooltip(f.status));
                                ui.add_space(3.0);
                                let name = f.path.rsplit('/').next().unwrap_or(&f.path);
                                let r = ui.selectable_label(
                                    is_sel,
                                    egui::RichText::new(name)
                                        .color(if is_sel {
                                            theme::bg_app()
                                        } else {
                                            theme::text_secondary()
                                        })
                                        .size(11.5),
                                );
                                if r.clicked() {
                                    clicked = true;
                                }
                                if f.path != name {
                                    r.on_hover_text(&f.path);
                                }
                            });
                            if clicked {
                                let path = f.path.clone();
                                let oid2 = oid.clone();
                                let state = self.state.clone();
                                {
                                    let mut s = self.state.lock().unwrap();
                                    s.selected_file = Some(path.clone());
                                    s.diff_context = DiffContext::CommitDiff;
                                }
                                load_diff(&oid2, &path, state);
                            }
                        }
                    } else {
                        let has_any = !workdir_files.is_empty();
                        if has_any {
                            ui.horizontal(|ui| {
                                ui.add_space(8.0);
                                ui.label(
                                    egui::RichText::new(format!(
                                        "Working tree ({})",
                                        workdir_files.len()
                                    ))
                                    .color(theme::text_muted())
                                    .size(10.5)
                                    .strong(),
                                );
                            });
                            ui.add_space(2.0);
                            for f in &workdir_files {
                                let is_sel = selected_file.as_deref() == Some(f.path.as_str());
                                let status = f.unstaged_status.or(f.staged_status).unwrap_or('M');
                                let (icon, col) = status_icon(status);
                                let mut clicked = false;
                                ui.horizontal(|ui| {
                                    ui.add_space(8.0);
                                    ui.label(
                                        egui::RichText::new(icon)
                                            .color(col)
                                            .size(11.0)
                                            .font(FontId::monospace(11.0)),
                                    )
                                    .on_hover_text(status_tooltip(status));
                                    ui.add_space(3.0);
                                    let name = f.path.rsplit('/').next().unwrap_or(&f.path);
                                    let r = ui.selectable_label(
                                        is_sel,
                                        egui::RichText::new(name)
                                            .color(if is_sel {
                                                theme::bg_app()
                                            } else {
                                                theme::text_secondary()
                                            })
                                            .size(11.5),
                                    );
                                    if r.clicked() {
                                        clicked = true;
                                    }
                                    if f.path != name {
                                        r.on_hover_text(&f.path);
                                    }
                                });
                                if clicked {
                                    let path = f.path.clone();
                                    let is_staged_only =
                                        f.staged_status.is_some() && f.unstaged_status.is_none();
                                    let ctx = if is_staged_only {
                                        DiffContext::StagedFile
                                    } else {
                                        DiffContext::UnstagedFile
                                    };
                                    let state = self.state.clone();
                                    {
                                        let mut s = self.state.lock().unwrap();
                                        s.selected_file = Some(path.clone());
                                        s.diff_context = ctx;
                                    }
                                    load_workdir_diff(&path, is_staged_only, state);
                                }
                            }
                        } else {
                            ui.add_space(16.0);
                            ui.horizontal(|ui| {
                                ui.add_space(12.0);
                                ui.label(
                                    egui::RichText::new("Working tree clean")
                                        .color(theme::text_muted())
                                        .size(11.5),
                                );
                            });
                        }
                    }
                    ui.add_space(4.0);
                });
        });

        // ── Diff area ─────────────────────────────────────────────────────
        let mut diff_child =
            ui.child_ui(diff_rect, egui::Layout::top_down(egui::Align::LEFT), None);
        let actions = self.show_diff_area(
            &mut diff_child,
            &diff_opt,
            &selected_file,
            &console_log,
            diff_context,
            &blame_lines,
            repo_epoch,
        );

        // Apply actions from the diff renderer
        for action in actions {
            match action {
                DiffAction::ApplySelection(lines, reverse) => {
                    staging::apply_selected_lines(lines, reverse, self.state.clone())
                }
                DiffAction::StageHunk(i) => staging::stage_hunk(i, self.state.clone()),
                DiffAction::UnstageHunk(i) => staging::unstage_hunk(i, self.state.clone()),
                DiffAction::DiscardHunk(i) => {
                    let mut s = self.state.lock().unwrap();
                    if let (Some(repo), Some(diff)) = (s.repo_path.clone(), s.current_diff.clone())
                    {
                        if let Some(hunk) = diff.hunks.get(i) {
                            let lines = hunk
                                .lines
                                .iter()
                                .enumerate()
                                .filter(|(_, l)| matches!(l.origin, '+' | '-'))
                                .map(|(l, _)| (i, l))
                                .collect();
                            s.pending_discard =
                                Some(PendingDiscard::Selection { repo, diff, lines });
                        }
                    }
                }
                DiffAction::DiscardSelection(lines) => {
                    let mut s = self.state.lock().unwrap();
                    if let (Some(repo), Some(diff)) = (s.repo_path.clone(), s.current_diff.clone())
                    {
                        s.pending_discard = Some(PendingDiscard::Selection { repo, diff, lines });
                    }
                }
                DiffAction::StageLine(h, l) => staging::stage_lines(h, vec![l], self.state.clone()),
                DiffAction::UnstageLine(h, l) => {
                    staging::unstage_lines(h, vec![l], self.state.clone())
                }
            }
        }
    }

    fn show_diff_area(
        &mut self,
        ui: &mut Ui,
        diff_opt: &Option<Arc<FileDiff>>,
        selected_file: &Option<String>,
        console_log: &[String],
        diff_context: DiffContext,
        blame_lines: &[crate::state::BlameLine],
        repo_epoch: u64,
    ) -> Vec<DiffAction> {
        // Preserve selection across identical background refreshes, including the
        // temporary loading state; repository/context/content changes invalidate it.
        let same_selection = self.last_repo_epoch == repo_epoch
            && self.last_context == diff_context
            && self
                .last_diff
                .as_ref()
                .is_some_and(|d| selected_file.as_deref() == Some(d.file_path.as_str()));
        if let Some(ref diff) = diff_opt {
            if !same_selection
                || self.items.is_empty()
                || !self
                    .last_diff
                    .as_ref()
                    .is_some_and(|previous| Arc::ptr_eq(previous, diff))
            {
                if !same_selection
                    || !self
                        .last_diff
                        .as_ref()
                        .is_some_and(|old| old.raw_patch == diff.raw_patch)
                {
                    self.selected_rows.clear();
                    self.selection_anchor = None;
                }
                self.binary_textures = [None, None];
                self.items = flatten_unified(diff);
                self.split_rows = flatten_split(diff);
                for row in &self.split_rows {
                    if let SplitRow::Pair { left, right } = row {
                        for line in [left, right].into_iter().flatten() {
                            if let Some(Item::Line { emphasis, .. }) =
                                self.items.get_mut(line.unified_row)
                            {
                                *emphasis = line.emphasis.clone();
                            }
                        }
                    }
                }
                self.last_diff = Some(Arc::clone(diff));
                self.last_context = diff_context;
                self.last_repo_epoch = repo_epoch;
            }
        } else {
            self.items.clear();
            self.split_rows.clear();
            self.binary_textures = [None, None];
            if !same_selection {
                self.last_diff = None;
                self.selected_rows.clear();
                self.selection_anchor = None;
            }
        }

        // ── Header ────────────────────────────────────────────────────────
        if self.state.lock().unwrap().focused_pane == 4
            && ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::F))
        {
            self.search_open = true;
        }
        let header_h = 36.0;
        let header_rect =
            Rect::from_min_size(ui.min_rect().min, Vec2::new(ui.available_width(), header_h));
        ui.painter()
            .rect_filled(header_rect, 0.0, theme::bg_panel());
        ui.painter().line_segment(
            [
                Pos2::new(header_rect.min.x, header_rect.max.y),
                Pos2::new(header_rect.max.x, header_rect.max.y),
            ],
            Stroke::new(1.0, theme::border()),
        );

        ui.allocate_ui_at_rect(header_rect, |ui| {
            ui.horizontal_centered(|ui| {
                ui.add_space(12.0);
                // Tab: File Diff
                {
                    let active = !self.show_console && !self.show_blame;
                    let tc = if active {
                        theme::text_primary()
                    } else {
                        theme::text_muted()
                    };
                    let btn = ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("File Diff").size(12.5).color(tc),
                            )
                            .fill(Color32::TRANSPARENT)
                            .stroke(Stroke::NONE),
                        )
                        .on_hover_text("Show the selected file diff");
                    if btn.clicked() {
                        self.show_console = false;
                        self.show_blame = false;
                        self.state.lock().unwrap().show_blame = false;
                    }
                    if active {
                        let br = btn.rect;
                        ui.painter().line_segment(
                            [br.left_bottom(), br.right_bottom()],
                            Stroke::new(2.0, theme::ACCENT_TEAL),
                        );
                    }
                    ui.add_space(10.0);
                }
                // Tab: Blame
                {
                    let active = self.show_blame;
                    let tc = if active {
                        theme::text_primary()
                    } else {
                        theme::text_muted()
                    };
                    let btn = ui
                        .add(
                            egui::Button::new(egui::RichText::new("Blame").size(12.5).color(tc))
                                .fill(Color32::TRANSPARENT)
                                .stroke(Stroke::NONE),
                        )
                        .on_hover_text("Show line blame for the selected file");
                    if btn.clicked() {
                        self.show_blame = true;
                        self.show_console = false;
                        self.state.lock().unwrap().show_blame = true;
                        if let Some(ref fp) = *selected_file {
                            crate::git::operations::load_blame(fp, self.state.clone());
                        }
                    }
                    if active {
                        let br = btn.rect;
                        ui.painter().line_segment(
                            [br.left_bottom(), br.right_bottom()],
                            Stroke::new(2.0, theme::ACCENT_TEAL),
                        );
                    }
                    ui.add_space(10.0);
                }
                // Tab: Console
                {
                    let active = self.show_console;
                    let tc = if active {
                        theme::text_primary()
                    } else {
                        theme::text_muted()
                    };
                    let btn = ui
                        .add(
                            egui::Button::new(egui::RichText::new("Console").size(12.5).color(tc))
                                .fill(Color32::TRANSPARENT)
                                .stroke(Stroke::NONE),
                        )
                        .on_hover_text("Show Git operation output");
                    if btn.clicked() {
                        self.show_console = true;
                        self.show_blame = false;
                        self.state.lock().unwrap().show_blame = false;
                    }
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
                if let Some(ref diff) = diff_opt {
                    let filename = diff.file_path.rsplit('/').next().unwrap_or(&diff.file_path);
                    ui.label(
                        egui::RichText::new(filename)
                            .color(theme::text_primary())
                            .size(12.5)
                            .strong(),
                    );
                    if diff_context != DiffContext::CommitDiff {
                        let ctx_label = if diff_context == DiffContext::StagedFile {
                            "staged"
                        } else {
                            "unstaged"
                        };
                        ui.label(
                            egui::RichText::new(ctx_label)
                                .color(theme::ACCENT_TEAL)
                                .size(10.5),
                        );
                    }
                }
                if ui.small_button("Find").clicked() {
                    self.search_open = !self.search_open;
                }
                ui.checkbox(&mut self.show_whitespace, "Whitespace")
                    .on_hover_text("Show spaces and tabs; patch bytes are unchanged");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(10.0);
                    if self.show_console && !console_log.is_empty() {
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("Export Log")
                                        .size(11.5)
                                        .color(theme::text_secondary()),
                                )
                                .fill(Color32::TRANSPARENT)
                                .stroke(Stroke::NONE),
                            )
                            .on_hover_text("Export console log to a text file")
                            .clicked()
                        {
                            if let Some(path) = rfd::FileDialog::new()
                                .set_file_name("recorde-log.txt")
                                .add_filter("Text", &["txt"])
                                .save_file()
                            {
                                let _ = std::fs::write(path, console_log.join("\n"));
                            }
                        }
                    }
                    if let Some(ref diff) = diff_opt {
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("✕")
                                        .color(theme::text_muted())
                                        .size(12.0),
                                )
                                .fill(Color32::TRANSPARENT)
                                .stroke(Stroke::NONE),
                            )
                            .on_hover_text("Close selected diff")
                            .clicked()
                        {
                            let mut s = self.state.lock().unwrap();
                            s.selected_file = None;
                            s.current_diff = None;
                        }
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new(format!(
                                "+{}  −{}",
                                diff.additions, diff.deletions
                            ))
                            .color(theme::text_secondary())
                            .size(11.5),
                        );
                        if !self.show_console {
                            ui.add_space(8.0);
                            ui.add(egui::Separator::default().vertical().spacing(3.0));
                            ui.add_space(6.0);
                            for (label, is_split) in &[("Unified", false), ("Split", true)] {
                                let active = *is_split == self.split_view;
                                let tc = if active {
                                    theme::ACCENT_TEAL
                                } else {
                                    theme::text_muted()
                                };
                                let btn = ui
                                    .add(
                                        egui::Button::new(
                                            egui::RichText::new(*label).size(11.5).color(tc),
                                        )
                                        .fill(Color32::TRANSPARENT)
                                        .stroke(Stroke::NONE),
                                    )
                                    .on_hover_text(if *is_split {
                                        "Show split diff"
                                    } else {
                                        "Show unified diff"
                                    });
                                if btn.clicked() {
                                    self.split_view = *is_split;
                                }
                                ui.add_space(2.0);
                            }
                        }
                    }
                });
            });
        });

        let mut content_rect = Rect::from_min_max(
            Pos2::new(ui.min_rect().min.x, ui.min_rect().min.y + header_h),
            ui.max_rect().max,
        );

        let mut scroll_to = None;
        if self.search_open && !self.show_console && !self.show_blame {
            let toolbar =
                Rect::from_min_size(content_rect.min, Vec2::new(content_rect.width(), 28.0));
            content_rect.min.y += 28.0;
            ui.allocate_ui_at_rect(toolbar, |ui| {
                ui.horizontal(|ui| {
                    let changed = ui
                        .add(
                            egui::TextEdit::singleline(&mut self.search_query)
                                .hint_text("Find in diff")
                                .desired_width(180.0),
                        )
                        .changed();
                    let previous = ui.button("Previous").clicked();
                    let next = ui.button("Next").clicked();
                    let change = ui.button("Next change").clicked();
                    if ui.button("Close").clicked() {
                        self.search_open = false;
                    }
                    if changed || previous || next || change {
                        let query = self.search_query.to_lowercase();
                        let matches: Vec<_> = self
                            .items
                            .iter()
                            .enumerate()
                            .filter_map(|(row, item)| match item {
                                Item::HunkHeader { .. } if change => Some(row),
                                Item::Line { content, .. }
                                    if !change
                                        && !query.is_empty()
                                        && content.to_lowercase().contains(&query) =>
                                {
                                    Some(row)
                                }
                                _ => None,
                            })
                            .collect();
                        self.search_row = if changed {
                            matches.first().copied()
                        } else if previous {
                            matches
                                .iter()
                                .rev()
                                .copied()
                                .find(|row| *row < self.search_row.unwrap_or(usize::MAX))
                                .or_else(|| matches.last().copied())
                        } else {
                            matches
                                .iter()
                                .copied()
                                .find(|row| *row > self.search_row.unwrap_or(0))
                                .or_else(|| matches.first().copied())
                        };
                        scroll_to = self.search_row;
                    }
                });
            });
        }

        if !self.show_console
            && !self.show_blame
            && diff_context != DiffContext::CommitDiff
            && diff_opt
                .as_ref()
                .is_some_and(|d| !d.is_binary && !d.is_rename)
        {
            let toolbar =
                Rect::from_min_size(content_rect.min, Vec2::new(content_rect.width(), 28.0));
            content_rect.min.y += 28.0;
            let mut apply = false;
            let mut discard = false;
            ui.allocate_ui_at_rect(toolbar, |ui| {
                ui.horizontal(|ui| {
                    let label = if diff_context == DiffContext::StagedFile { "Unstage selection" } else { "Stage selection" };
                    apply = ui.add_enabled(!self.selected_rows.is_empty(), egui::Button::new(format!("{label} ({})", self.selected_rows.len()))).clicked();
                    if diff_context == DiffContext::UnstagedFile {
                        discard = ui.add_enabled(!self.selected_rows.is_empty(), egui::Button::new("Discard selection")).clicked();
                    }
                    if ui.button("Clear").clicked() { self.selected_rows.clear(); self.selection_anchor = None; }
                    ui.small("Click lines; Ctrl/Cmd toggles, Shift selects a range. Gutter applies one line.");
                });
            });
            if apply || discard {
                let lines = self
                    .selected_rows
                    .iter()
                    .filter_map(|&row| match self.items.get(row) {
                        Some(Item::Line {
                            hunk_index,
                            line_index,
                            ..
                        }) => Some((*hunk_index, *line_index)),
                        _ => None,
                    })
                    .collect();
                return vec![if discard {
                    DiffAction::DiscardSelection(lines)
                } else {
                    DiffAction::ApplySelection(lines, diff_context == DiffContext::StagedFile)
                }];
            }
        }

        // ── Console ───────────────────────────────────────────────────────
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
                                ui.label(
                                    egui::RichText::new("No git operations yet.")
                                        .color(theme::text_muted())
                                        .size(12.0),
                                );
                            });
                        } else {
                            for line in console_log {
                                let line = crate::auth::redact::text(line);
                                let color = if line.starts_with("error:") {
                                    theme::ACCENT_RED
                                } else if line.starts_with('$') {
                                    theme::ACCENT_TEAL
                                } else {
                                    theme::text_secondary()
                                };
                                ui.horizontal(|ui| {
                                    ui.add_space(12.0);
                                    ui.label(
                                        egui::RichText::new(line)
                                            .color(color)
                                            .size(12.0)
                                            .font(FontId::monospace(12.0)),
                                    );
                                });
                            }
                        }
                        ui.add_space(6.0);
                    });
            });
            return Vec::new();
        }

        // ── Blame ─────────────────────────────────────────────────────────
        if self.show_blame {
            ui.allocate_ui_at_rect(content_rect, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        ui.add_space(4.0);
                        if blame_lines.is_empty() {
                            ui.centered_and_justified(|ui| {
                                let message = if selected_file.is_some() {
                                    "Loading blame..."
                                } else {
                                    "Select a file to view blame."
                                };
                                ui.label(
                                    egui::RichText::new(message)
                                        .color(theme::text_muted())
                                        .size(12.0),
                                );
                            });
                        } else {
                            let mut last_oid = "";
                            for line in blame_lines {
                                let show_oid = line.short_oid.as_str() != last_oid;
                                last_oid = &line.short_oid;
                                ui.horizontal(|ui| {
                                    ui.add_space(4.0);
                                    // SHA column (show only on first line of a hunk)
                                    let oid_color = if show_oid {
                                        theme::ACCENT_TEAL
                                    } else {
                                        theme::bg_panel()
                                    };
                                    ui.label(
                                        egui::RichText::new(&line.short_oid)
                                            .monospace()
                                            .color(oid_color)
                                            .size(10.5),
                                    );
                                    ui.add_space(4.0);
                                    // Author column (truncated)
                                    let author = if line.author.len() > 14 {
                                        &line.author[..14]
                                    } else {
                                        &line.author
                                    };
                                    let author_color = if show_oid {
                                        theme::text_muted()
                                    } else {
                                        theme::bg_panel()
                                    };
                                    ui.label(
                                        egui::RichText::new(author).size(10.5).color(author_color),
                                    );
                                    ui.add_space(4.0);
                                    // Line number
                                    ui.label(
                                        egui::RichText::new(format!("{:>4}", line.lineno))
                                            .monospace()
                                            .color(theme::text_muted())
                                            .size(10.5),
                                    );
                                    ui.add_space(6.0);
                                    // Content
                                    ui.label(
                                        egui::RichText::new(&line.content)
                                            .monospace()
                                            .color(theme::text_secondary())
                                            .size(11.5),
                                    );
                                });
                            }
                        }
                        ui.add_space(4.0);
                    });
            });
            return Vec::new();
        }

        // ── Empty / binary / no-changes ───────────────────────────────────
        if let Some(preview) = diff_opt.as_ref().and_then(|d| d.binary_preview.as_ref()) {
            ui.allocate_ui_at_rect(content_rect, |ui| {
                egui::ScrollArea::vertical()
                    .id_source("binary_preview")
                    .show(ui, |ui| {
                        crate::ui::binary_preview::show(
                            ui,
                            preview,
                            &mut self.binary_textures,
                            diff_context,
                        );
                    });
            });
            return Vec::new();
        }
        if let Some(diff) = diff_opt
            .as_ref()
            .filter(|d| !d.is_binary && d.hunks.is_empty() && !d.raw_patch.is_empty())
        {
            ui.allocate_ui_at_rect(content_rect, |ui| {
                egui::ScrollArea::both().show(ui, |ui| {
                    ui.label("File metadata changed");
                    ui.monospace(String::from_utf8_lossy(&diff.raw_patch));
                });
            });
            return Vec::new();
        }
        let empty_msg = if diff_opt.is_none() {
            Some(if selected_file.is_some() {
                "Loading diff…"
            } else {
                "Select a file to view its diff."
            })
        } else if diff_opt.as_ref().map_or(false, |d| d.is_binary) {
            Some("Binary file — no diff available.")
        } else if self.items.is_empty() {
            Some("No changes.")
        } else {
            None
        };
        if let Some(msg) = empty_msg {
            ui.allocate_ui_at_rect(content_rect, |ui| {
                ui.centered_and_justified(|ui| {
                    ui.label(
                        egui::RichText::new(msg)
                            .color(theme::text_muted())
                            .size(12.0),
                    );
                });
            });
            return Vec::new();
        }

        // ── Dispatch to renderer ──────────────────────────────────────────
        if self.split_view {
            render_split(
                ui,
                content_rect,
                &self.split_rows,
                &self.items,
                diff_context,
                &mut self.selected_rows,
                &mut self.selection_anchor,
                scroll_to,
                selected_file
                    .as_ref()
                    .is_some_and(|path| super::highlight::supported(path)),
                self.show_whitespace,
            )
        } else {
            render_unified(
                ui,
                content_rect,
                &self.items,
                diff_context,
                &mut self.selected_rows,
                &mut self.selection_anchor,
                scroll_to,
                selected_file
                    .as_ref()
                    .is_some_and(|path| super::highlight::supported(path)),
                self.show_whitespace,
            )
        }
    }
}

// ── Unified renderer ──────────────────────────────────────────────────────────

fn render_unified(
    ui: &mut Ui,
    rect: Rect,
    items: &[Item],
    diff_context: DiffContext,
    selected: &mut std::collections::BTreeSet<usize>,
    anchor: &mut Option<usize>,
    scroll_to: Option<usize>,
    code: bool,
    whitespace: bool,
) -> Vec<DiffAction> {
    let is_workdir = diff_context != DiffContext::CommitDiff;
    let is_staged = diff_context == DiffContext::StagedFile;
    let avail_w = rect.width();
    let total_h = items.len() as f32 * ROW_HEIGHT;
    let actions = std::cell::RefCell::new(Vec::<DiffAction>::new());

    ui.allocate_ui_at_rect(rect, |ui| {
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                let sense = if is_workdir {
                    Sense::click()
                } else {
                    Sense::hover()
                };
                let (resp, painter) = ui.allocate_painter(Vec2::new(avail_w, total_h), sense);
                let origin = resp.rect.min;
                if let Some(row) = scroll_to {
                    ui.scroll_to_rect(
                        Rect::from_min_size(
                            Pos2::new(origin.x, origin.y + row as f32 * ROW_HEIGHT),
                            Vec2::new(avail_w, ROW_HEIGHT),
                        ),
                        Some(egui::Align::Center),
                    );
                }
                let clip = painter.clip_rect();

                let click_pos = if resp.clicked() {
                    resp.interact_pointer_pos()
                } else {
                    None
                };
                let hover_pos = resp.hover_pos();

                let first = ((clip.top() - origin.y) / ROW_HEIGHT).floor().max(0.0) as usize;
                let last = (((clip.bottom() - origin.y) / ROW_HEIGHT).ceil() as usize + 1)
                    .min(items.len());

                for i in first..last {
                    let y = origin.y + i as f32 * ROW_HEIGHT;
                    let row_rect =
                        Rect::from_min_size(Pos2::new(origin.x, y), Vec2::new(avail_w, ROW_HEIGHT));

                    match &items[i] {
                        Item::HunkHeader { text, hunk_index } => {
                            painter.rect_filled(row_rect, 0.0, theme::bg_diff_hunk());
                            painter.line_segment(
                                [
                                    Pos2::new(row_rect.min.x, row_rect.max.y - 0.5),
                                    Pos2::new(row_rect.max.x, row_rect.max.y - 0.5),
                                ],
                                Stroke::new(0.5, theme::border()),
                            );
                            let text_end_x = if is_workdir {
                                avail_w - 220.0
                            } else {
                                avail_w - 10.0
                            };
                            painter
                                .with_clip_rect(Rect::from_min_max(
                                    Pos2::new(origin.x + 10.0, y),
                                    Pos2::new(origin.x + text_end_x, y + ROW_HEIGHT),
                                ))
                                .text(
                                    Pos2::new(origin.x + 10.0, y + ROW_HEIGHT / 2.0),
                                    egui::Align2::LEFT_CENTER,
                                    text,
                                    FontId::monospace(FONT_SIZE),
                                    theme::text_muted(),
                                );

                            if is_workdir {
                                let hi = *hunk_index;
                                let btns: &[(&str, Color32, bool)] = if is_staged {
                                    &[("Unstage Hunk", theme::ACCENT_TEAL, false)]
                                } else {
                                    &[
                                        ("Stage Hunk", theme::ACCENT_TEAL, false),
                                        ("Discard Hunk", theme::ACCENT_RED, true),
                                    ]
                                };

                                let btn_w = 94.0;
                                let btn_h = ROW_HEIGHT - 4.0;
                                let mut btn_x =
                                    origin.x + avail_w - (btns.len() as f32 * (btn_w + 4.0)) - 4.0;

                                for (label, col, is_discard) in btns {
                                    let btn_rect = Rect::from_min_size(
                                        Pos2::new(btn_x, y + 2.0),
                                        Vec2::new(btn_w, btn_h),
                                    );
                                    let hovered = hover_pos.map_or(false, |p| btn_rect.contains(p));
                                    painter.rect_filled(
                                        btn_rect,
                                        3.0,
                                        if hovered {
                                            col.gamma_multiply(0.25)
                                        } else {
                                            Color32::TRANSPARENT
                                        },
                                    );
                                    painter.rect_stroke(btn_rect, 3.0, Stroke::new(1.0, *col));
                                    painter.text(
                                        btn_rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        *label,
                                        FontId::proportional(10.5),
                                        *col,
                                    );

                                    if let Some(cp) = click_pos {
                                        if btn_rect.contains(cp) {
                                            let mut acts = actions.borrow_mut();
                                            if *is_discard {
                                                acts.push(DiffAction::DiscardHunk(hi));
                                            } else if is_staged {
                                                acts.push(DiffAction::UnstageHunk(hi));
                                            } else {
                                                acts.push(DiffAction::StageHunk(hi));
                                            }
                                        }
                                    }
                                    btn_x += btn_w + 4.0;
                                }
                            }
                        }

                        Item::Line {
                            emphasis,
                            origin: ch,
                            old_no,
                            new_no,
                            content,
                            hunk_index,
                            line_index,
                        } => {
                            let (row_bg, bar_col, text_col) =
                                conflict_marker_colors(content).unwrap_or_else(|| line_colors(*ch));
                            if row_bg != Color32::TRANSPARENT {
                                painter.rect_filled(row_rect, 0.0, row_bg);
                            }
                            if bar_col != Color32::TRANSPARENT {
                                painter.rect_filled(
                                    Rect::from_min_size(
                                        Pos2::new(origin.x, y),
                                        Vec2::new(BAR_W, ROW_HEIGHT),
                                    ),
                                    0.0,
                                    bar_col,
                                );
                            }
                            if selected.contains(&i) {
                                painter.rect_filled(
                                    row_rect,
                                    0.0,
                                    theme::ACCENT_BLUE.gamma_multiply(0.25),
                                );
                            }
                            let cy = y + ROW_HEIGHT / 2.0;
                            painter.text(
                                Pos2::new(origin.x + BAR_W + 4.0 + 30.0, cy),
                                egui::Align2::RIGHT_CENTER,
                                old_no,
                                FontId::monospace(10.5),
                                theme::text_muted(),
                            );
                            painter.text(
                                Pos2::new(origin.x + BAR_W + 4.0 + 68.0, cy),
                                egui::Align2::RIGHT_CENTER,
                                new_no,
                                FontId::monospace(10.5),
                                theme::text_muted(),
                            );
                            painter.line_segment(
                                [
                                    Pos2::new(origin.x + U_GUTTER, y),
                                    Pos2::new(origin.x + U_GUTTER, y + ROW_HEIGHT),
                                ],
                                Stroke::new(0.5, theme::border()),
                            );
                            if *ch != ' ' && *ch != '\\' {
                                painter.text(
                                    Pos2::new(origin.x + U_GUTTER + 8.0, cy),
                                    egui::Align2::LEFT_CENTER,
                                    &ch.to_string(),
                                    FontId::monospace(FONT_SIZE),
                                    bar_col,
                                );
                            }
                            let cx = origin.x + U_GUTTER + 18.0;
                            super::highlight::paint(
                                &painter.with_clip_rect(Rect::from_min_max(
                                    Pos2::new(cx, y),
                                    Pos2::new(origin.x + avail_w - 4.0, y + ROW_HEIGHT),
                                )),
                                Pos2::new(cx, cy),
                                content,
                                text_col,
                                code,
                                emphasis.as_ref(),
                                whitespace,
                            );

                            // Line-level staging: show clickable glyph in gutter on hover for +/- lines
                            if is_workdir && (*ch == '+' || *ch == '-') {
                                let is_hovered = hover_pos.map_or(false, |p| row_rect.contains(p));
                                if is_hovered {
                                    let gutter_rect = Rect::from_min_size(
                                        Pos2::new(origin.x, y),
                                        Vec2::new(BAR_W + 4.0, ROW_HEIGHT),
                                    );
                                    // Highlight gutter for click hint
                                    painter.rect_filled(
                                        gutter_rect,
                                        0.0,
                                        bar_col.gamma_multiply(0.5),
                                    );

                                    if let Some(cp) = click_pos {
                                        if row_rect.contains(cp) && cp.x >= origin.x + U_GUTTER {
                                            select_line(
                                                items,
                                                selected,
                                                anchor,
                                                i,
                                                ui.input(|input| input.modifiers),
                                            );
                                        } else if row_rect.contains(cp) {
                                            let hi = *hunk_index;
                                            let li = *line_index;
                                            let mut acts = actions.borrow_mut();
                                            if is_staged {
                                                acts.push(DiffAction::UnstageLine(hi, li));
                                            } else {
                                                acts.push(DiffAction::StageLine(hi, li));
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        Item::Binary => {
                            painter.text(
                                Pos2::new(origin.x + 12.0, y + ROW_HEIGHT / 2.0),
                                egui::Align2::LEFT_CENTER,
                                "Binary file",
                                FontId::proportional(12.0),
                                theme::text_muted(),
                            );
                        }
                    }
                }
            });
    });

    actions.into_inner()
}

// ── Split renderer ────────────────────────────────────────────────────────────

fn select_line(
    items: &[Item],
    selected: &mut std::collections::BTreeSet<usize>,
    anchor: &mut Option<usize>,
    row: usize,
    modifiers: egui::Modifiers,
) {
    if modifiers.shift {
        let start = anchor.unwrap_or(row);
        if !modifiers.command {
            selected.clear();
        }
        for index in start.min(row)..=start.max(row) {
            if matches!(
                items.get(index),
                Some(Item::Line {
                    origin: '+' | '-',
                    ..
                })
            ) {
                selected.insert(index);
            }
        }
    } else if modifiers.command {
        if !selected.remove(&row) {
            selected.insert(row);
        }
        *anchor = Some(row);
    } else {
        selected.clear();
        selected.insert(row);
        *anchor = Some(row);
    }
}

fn render_split(
    ui: &mut Ui,
    rect: Rect,
    rows: &[SplitRow],
    items: &[Item],
    context: DiffContext,
    selected: &mut std::collections::BTreeSet<usize>,
    anchor: &mut Option<usize>,
    scroll_to: Option<usize>,
    code: bool,
    whitespace: bool,
) -> Vec<DiffAction> {
    let scroll_to = scroll_to.and_then(|target| {
        rows.iter().position(|row| match row {
            SplitRow::Pair { left, right } => {
                left.as_ref().is_some_and(|line| line.unified_row >= target)
                    || right
                        .as_ref()
                        .is_some_and(|line| line.unified_row >= target)
            }
            _ => false,
        })
    });
    let mut actions = Vec::new();
    let avail_w = rect.width();
    let half = (avail_w / 2.0).floor();
    let total_h = rows.len() as f32 * ROW_HEIGHT;

    ui.allocate_ui_at_rect(rect, |ui| {
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                let (resp, painter) =
                    ui.allocate_painter(Vec2::new(avail_w, total_h), Sense::hover());
                let origin = resp.rect.min;
                if let Some(row) = scroll_to {
                    ui.scroll_to_rect(
                        Rect::from_min_size(
                            Pos2::new(origin.x, origin.y + row as f32 * ROW_HEIGHT),
                            Vec2::new(avail_w, ROW_HEIGHT),
                        ),
                        Some(egui::Align::Center),
                    );
                }
                let clip = painter.clip_rect();

                let div_x = origin.x + half;
                painter.line_segment(
                    [
                        Pos2::new(div_x, clip.top()),
                        Pos2::new(div_x, clip.bottom()),
                    ],
                    Stroke::new(1.0, theme::border()),
                );

                let first = ((clip.top() - origin.y) / ROW_HEIGHT).floor().max(0.0) as usize;
                let last =
                    (((clip.bottom() - origin.y) / ROW_HEIGHT).ceil() as usize + 1).min(rows.len());

                for i in first..last {
                    let y = origin.y + i as f32 * ROW_HEIGHT;
                    match &rows[i] {
                        SplitRow::HunkHeader(text) => {
                            let full = Rect::from_min_size(
                                Pos2::new(origin.x, y),
                                Vec2::new(avail_w, ROW_HEIGHT),
                            );
                            painter.rect_filled(full, 0.0, theme::bg_diff_hunk());
                            painter.line_segment(
                                [
                                    Pos2::new(full.min.x, full.max.y - 0.5),
                                    Pos2::new(full.max.x, full.max.y - 0.5),
                                ],
                                Stroke::new(0.5, theme::border()),
                            );
                            painter.text(
                                Pos2::new(origin.x + 10.0, y + ROW_HEIGHT / 2.0),
                                egui::Align2::LEFT_CENTER,
                                text,
                                FontId::monospace(FONT_SIZE),
                                theme::text_muted(),
                            );
                        }
                        SplitRow::Pair { left, right } => {
                            for (side, x, is_left) in
                                [(left, origin.x, true), (right, origin.x + half, false)]
                            {
                                render_split_side(
                                    &painter, x, y, half, side, is_left, code, whitespace,
                                );
                                let Some(line) = side.as_ref().filter(|line| line.changed) else {
                                    continue;
                                };
                                let side_rect = Rect::from_min_size(
                                    Pos2::new(x, y),
                                    Vec2::new(half, ROW_HEIGHT),
                                );
                                if selected.contains(&line.unified_row) {
                                    painter.rect_filled(
                                        side_rect,
                                        0.0,
                                        theme::ACCENT_BLUE.gamma_multiply(0.25),
                                    );
                                }
                                if context == DiffContext::CommitDiff {
                                    continue;
                                }
                                let response = ui.interact(
                                    side_rect.intersect(clip),
                                    ui.id().with(("split_line", i, is_left)),
                                    Sense::click(),
                                );
                                response.widget_info(|| {
                                    egui::WidgetInfo::selected(
                                        egui::WidgetType::SelectableLabel,
                                        true,
                                        selected.contains(&line.unified_row),
                                        format!("{} {}", line.lineno, line.content),
                                    )
                                });
                                if response.clicked() {
                                    if response
                                        .interact_pointer_pos()
                                        .is_some_and(|p| p.x < x + S_GUTTER)
                                    {
                                        if let Some(Item::Line {
                                            hunk_index,
                                            line_index,
                                            ..
                                        }) = items.get(line.unified_row)
                                        {
                                            actions.push(if context == DiffContext::StagedFile {
                                                DiffAction::UnstageLine(*hunk_index, *line_index)
                                            } else {
                                                DiffAction::StageLine(*hunk_index, *line_index)
                                            });
                                        }
                                    } else {
                                        select_line(
                                            items,
                                            selected,
                                            anchor,
                                            line.unified_row,
                                            ui.input(|i| i.modifiers),
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            });
    });
    actions
}

fn render_split_side(
    painter: &egui::Painter,
    x0: f32,
    y: f32,
    width: f32,
    side: &Option<SideLine>,
    is_left: bool,
    code: bool,
    whitespace: bool,
) {
    let cy = y + ROW_HEIGHT / 2.0;
    let (bg, bar_col, text_col) = match side {
        Some(sl) if sl.changed && is_left => (
            theme::bg_diff_del(),
            theme::ACCENT_RED,
            Color32::from_rgb(0xF0, 0xA0, 0xA0),
        ),
        Some(sl) if sl.changed && !is_left => (
            theme::bg_diff_add(),
            theme::ACCENT_GREEN,
            Color32::from_rgb(0xAB, 0xE0, 0xAB),
        ),
        Some(_) => (
            Color32::TRANSPARENT,
            Color32::TRANSPARENT,
            theme::text_primary(),
        ),
        None => (
            theme::bg_panel_alt(),
            Color32::TRANSPARENT,
            theme::text_primary(),
        ),
    };

    let row_rect = Rect::from_min_size(Pos2::new(x0, y), Vec2::new(width, ROW_HEIGHT));
    if bg != Color32::TRANSPARENT {
        painter.rect_filled(row_rect, 0.0, bg);
    }
    if bar_col != Color32::TRANSPARENT {
        painter.rect_filled(
            Rect::from_min_size(Pos2::new(x0, y), Vec2::new(BAR_W, ROW_HEIGHT)),
            0.0,
            bar_col,
        );
    }

    if let Some(sl) = side {
        painter.text(
            Pos2::new(x0 + BAR_W + 4.0 + S_GUTTER - 6.0, cy),
            egui::Align2::RIGHT_CENTER,
            &sl.lineno,
            FontId::monospace(10.5),
            theme::text_muted(),
        );
        painter.line_segment(
            [
                Pos2::new(x0 + S_GUTTER, y),
                Pos2::new(x0 + S_GUTTER, y + ROW_HEIGHT),
            ],
            Stroke::new(0.5, theme::border()),
        );
        if sl.changed {
            let sigil = if is_left { "-" } else { "+" };
            painter.text(
                Pos2::new(x0 + S_GUTTER + 6.0, cy),
                egui::Align2::LEFT_CENTER,
                sigil,
                FontId::monospace(FONT_SIZE),
                bar_col,
            );
        }
        let cx = x0 + S_GUTTER + 16.0;
        super::highlight::paint(
            &painter.with_clip_rect(Rect::from_min_max(
                Pos2::new(cx, y),
                Pos2::new(x0 + width - 2.0, y + ROW_HEIGHT),
            )),
            Pos2::new(cx, cy),
            &sl.content,
            text_col,
            code,
            sl.emphasis.as_ref(),
            whitespace,
        );
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn line_colors(ch: char) -> (Color32, Color32, Color32) {
    match ch {
        '+' => (
            theme::bg_diff_add(),
            theme::ACCENT_GREEN,
            Color32::from_rgb(0xAB, 0xE0, 0xAB),
        ),
        '-' => (
            theme::bg_diff_del(),
            theme::ACCENT_RED,
            Color32::from_rgb(0xF0, 0xA0, 0xA0),
        ),
        _ => (
            Color32::TRANSPARENT,
            Color32::TRANSPARENT,
            theme::text_primary(),
        ),
    }
}

/// Override colours for git conflict marker lines (<<< === >>>).
/// Returns (bg, bar, text) if the content is a conflict marker, else None.
fn conflict_marker_colors(content: &str) -> Option<(Color32, Color32, Color32)> {
    if content.starts_with("<<<<<<<") {
        Some((
            Color32::from_rgb(0x3A, 0x10, 0x08),
            Color32::from_rgb(0xFF, 0x60, 0x50),
            Color32::from_rgb(0xFF, 0x90, 0x80),
        ))
    } else if content.starts_with("=======") || content.starts_with("|||||||") {
        Some((
            Color32::from_rgb(0x2A, 0x22, 0x00),
            Color32::from_rgb(0xFF, 0xCC, 0x00),
            Color32::from_rgb(0xFF, 0xE0, 0x60),
        ))
    } else if content.starts_with(">>>>>>>") {
        Some((
            Color32::from_rgb(0x08, 0x22, 0x10),
            Color32::from_rgb(0x40, 0xCC, 0x70),
            Color32::from_rgb(0x70, 0xFF, 0x90),
        ))
    } else {
        None
    }
}

fn flatten_unified(diff: &FileDiff) -> Vec<Item> {
    if diff.is_binary {
        return vec![Item::Binary];
    }
    let mut items = Vec::new();
    for (hunk_index, hunk) in diff.hunks.iter().enumerate() {
        items.push(Item::HunkHeader {
            text: hunk.header.clone(),
            hunk_index,
        });
        for (line_index, line) in hunk.lines.iter().enumerate() {
            items.push(Item::Line {
                emphasis: None,
                origin: line.origin,
                old_no: line.old_lineno.map(|n| n.to_string()).unwrap_or_default(),
                new_no: line.new_lineno.map(|n| n.to_string()).unwrap_or_default(),
                content: line.content.trim_end_matches(['\r', '\n']).to_string(),
                hunk_index,
                line_index,
            });
        }
    }
    items
}

fn flatten_split(diff: &FileDiff) -> Vec<SplitRow> {
    if diff.is_binary {
        return vec![SplitRow::HunkHeader("Binary file".into())];
    }

    let mut rows = Vec::new();
    let mut unified_row = 0;
    for hunk in &diff.hunks {
        unified_row += 1;
        rows.push(SplitRow::HunkHeader(hunk.header.clone()));
        let mut dels: Vec<SideLine> = Vec::new();
        let mut adds: Vec<SideLine> = Vec::new();

        let flush = |rows: &mut Vec<SplitRow>,
                     dels: &mut Vec<SideLine>,
                     adds: &mut Vec<SideLine>| {
            let n = dels.len().max(adds.len());
            for i in 0..n {
                let mut left = dels.get(i).cloned();
                let mut right = adds.get(i).cloned();
                if let (Some(left), Some(right)) = (&mut left, &mut right) {
                    let (a, b) = super::highlight::changed_ranges(&left.content, &right.content);
                    left.emphasis = Some(a);
                    right.emphasis = Some(b);
                }
                rows.push(SplitRow::Pair { left, right });
            }
            dels.clear();
            adds.clear();
        };

        for line in &hunk.lines {
            let content = line.content.trim_end_matches(['\r', '\n']).to_string();
            match line.origin {
                '-' => dels.push(SideLine {
                    emphasis: None,
                    unified_row,
                    lineno: line.old_lineno.map(|n| n.to_string()).unwrap_or_default(),
                    content,
                    changed: true,
                }),
                '+' => adds.push(SideLine {
                    emphasis: None,
                    unified_row,
                    lineno: line.new_lineno.map(|n| n.to_string()).unwrap_or_default(),
                    content,
                    changed: true,
                }),
                _ => {
                    flush(&mut rows, &mut dels, &mut adds);
                    let sl = SideLine {
                        emphasis: None,
                        unified_row,
                        lineno: line.old_lineno.map(|n| n.to_string()).unwrap_or_default(),
                        content: content.clone(),
                        changed: false,
                    };
                    let sl2 = SideLine {
                        emphasis: None,
                        unified_row,
                        lineno: line.new_lineno.map(|n| n.to_string()).unwrap_or_default(),
                        content,
                        changed: false,
                    };
                    rows.push(SplitRow::Pair {
                        left: Some(sl),
                        right: Some(sl2),
                    });
                }
            }
            unified_row += 1;
        }
        flush(&mut rows, &mut dels, &mut adds);
    }
    rows
}

fn status_icon(status: char) -> (&'static str, egui::Color32) {
    match status {
        'A' | '?' => ("A", theme::ACCENT_GREEN),
        'D' => ("D", theme::ACCENT_RED),
        'R' => ("R", theme::ACCENT_YELLOW),
        _ => ("M", theme::ACCENT_YELLOW),
    }
}

fn status_tooltip(status: char) -> &'static str {
    match status {
        'A' => "Added",
        '?' => "Untracked",
        'D' => "Deleted",
        'R' => "Renamed",
        _ => "Modified",
    }
}
