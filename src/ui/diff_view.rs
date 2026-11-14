use egui::{Color32, FontId, Pos2, Rect, Sense, Stroke, Ui, Vec2};
use std::collections::BTreeSet;
use std::sync::Arc;

use crate::git::operations::{load_diff, load_workdir_diff};
use crate::git::staging;
use crate::state::{DiffContext, FileDiff, PendingDiscard, SharedState};
use crate::theme;
use crate::ui::{kit, motion};

const ROW_HEIGHT: f32 = 22.0;
const FONT_SIZE: f32 = 13.0;
const U_GUTTER: f32 = 76.0;
const BAR_W: f32 = 3.0;
const S_GUTTER: f32 = 44.0;
const CONTENT_PADDING: f32 = 8.0;

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
    content_width: f32,
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
    comparison: crate::config::DiffComparison,
}

impl DiffView {
    pub fn open_search(&mut self) {
        self.search_open = true;
    }

    pub fn preferences(&self) -> crate::config::DiffPreferences {
        crate::config::DiffPreferences {
            comparison: self.comparison,
            whitespace: self.show_whitespace,
            split: self.split_view,
            search_open: self.search_open,
            search_query: self.search_query.clone(),
        }
    }

    pub fn restore_preferences(&mut self, preferences: &crate::config::DiffPreferences) {
        self.comparison = preferences.comparison;
        self.state.lock().unwrap().diff_comparison = self.comparison;
        self.reload_comparison();
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
            content_width: 0.0,
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
            comparison: Default::default(),
        }
    }

    fn reload_comparison(&self) {
        let (file, commit, context) = {
            let mut state = self.state.lock().unwrap();
            state.diff_comparison = self.comparison;
            (
                state.selected_file.clone(),
                state.selected_commit.clone(),
                state.diff_context,
            )
        };
        if let Some(file) = file {
            match context {
                DiffContext::CommitDiff => {
                    if let Some(oid) = commit {
                        load_diff(&oid, &file, self.state.clone());
                    }
                }
                DiffContext::StagedFile => load_workdir_diff(&file, true, self.state.clone()),
                DiffContext::UnstagedFile => load_workdir_diff(&file, false, self.state.clone()),
            }
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

        ui.set_min_height(ui.available_height());
        let file_list_w = if ui.available_width() >= 1050.0 {
            176.0
        } else {
            0.0
        };
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
        if file_list_w > 0.0 {
            ui.scope_builder(egui::UiBuilder::new().max_rect(list_rect), |ui| {
                ui.set_clip_rect(ui.clip_rect().intersect(list_rect));
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
                                        .size(12.0)
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
                                            .size(12.0)
                                            .font(FontId::monospace(12.0)),
                                    )
                                    .on_hover_text(status_tooltip(f.status));
                                    ui.add_space(3.0);
                                    let name = f.path.rsplit('/').next().unwrap_or(&f.path);
                                    let r = ui.add(
                                        egui::Button::selectable(
                                            is_sel,
                                            egui::RichText::new(name)
                                                .color(if is_sel {
                                                    theme::text_primary()
                                                } else {
                                                    theme::text_secondary()
                                                })
                                                .size(12.0),
                                        )
                                        .truncate(),
                                    );
                                    if r.clicked() {
                                        clicked = true;
                                    }
                                    r.on_hover_text(&f.path);
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
                                            "Uncommitted changes ({})",
                                            workdir_files.len()
                                        ))
                                        .color(theme::text_muted())
                                        .size(12.0)
                                        .strong(),
                                    );
                                });
                                ui.add_space(2.0);
                                for f in &workdir_files {
                                    let is_sel = selected_file.as_deref() == Some(f.path.as_str());
                                    let status =
                                        f.unstaged_status.or(f.staged_status).unwrap_or('M');
                                    let (icon, col) = status_icon(status);
                                    let mut clicked = false;
                                    ui.horizontal(|ui| {
                                        ui.add_space(8.0);
                                        ui.label(
                                            egui::RichText::new(icon)
                                                .color(col)
                                                .size(12.0)
                                                .font(FontId::monospace(12.0)),
                                        )
                                        .on_hover_text(status_tooltip(status));
                                        ui.add_space(3.0);
                                        let name = f.path.rsplit('/').next().unwrap_or(&f.path);
                                        let r = ui.add(
                                            egui::Button::selectable(
                                                is_sel,
                                                egui::RichText::new(name)
                                                    .color(if is_sel {
                                                        theme::text_primary()
                                                    } else {
                                                        theme::text_secondary()
                                                    })
                                                    .size(12.0),
                                            )
                                            .truncate(),
                                        );
                                        if r.clicked() {
                                            clicked = true;
                                        }
                                        r.on_hover_text(&f.path);
                                    });
                                    if clicked {
                                        let path = f.path.clone();
                                        let is_staged_only = f.staged_status.is_some()
                                            && f.unstaged_status.is_none();
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
                                        egui::RichText::new("No uncommitted changes")
                                            .color(theme::text_muted())
                                            .size(12.0),
                                    );
                                });
                            }
                        }
                        ui.add_space(4.0);
                    });
            });

            // ── Diff area ─────────────────────────────────────────────────────
        }
        let mut diff_child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(diff_rect)
                .layout(egui::Layout::top_down(egui::Align::LEFT)),
        );
        let actions = self.show_diff_area(
            &mut diff_child,
            &diff_opt,
            &selected_file,
            &console_log,
            diff_context,
            &blame_lines,
            repo_epoch,
        );

        // A settings change invalidates the displayed snapshot before any action.
        if self.state.lock().unwrap().diff_comparison != self.comparison {
            self.reload_comparison();
            return;
        }
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
                    || !self.last_diff.as_ref().is_some_and(|old| {
                        old.raw_patch == diff.raw_patch
                            && old.canonical_hunks.is_none()
                            && diff.canonical_hunks.is_none()
                    })
                {
                    self.selected_rows.clear();
                    self.selection_anchor = None;
                }
                self.binary_textures = [None, None];
                self.items = flatten_unified(diff);
                let glyph_width = ui
                    .painter()
                    .layout_no_wrap(
                        "M".into(),
                        FontId::monospace(FONT_SIZE),
                        theme::text_primary(),
                    )
                    .size()
                    .x
                    .ceil();
                self.content_width = widest_content_width(&self.items, glyph_width);
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
            self.content_width = 0.0;
            self.split_rows.clear();
            self.binary_textures = [None, None];
            if !same_selection {
                self.last_diff = None;
                self.selected_rows.clear();
                self.selection_anchor = None;
            }
        }

        // ── Header ────────────────────────────────────────────────────────
        let header_h = 80.0;
        let header_rect =
            Rect::from_min_size(ui.min_rect().min, Vec2::new(ui.available_width(), header_h));
        ui.painter()
            .rect_filled(header_rect, 0.0, theme::bg_panel());
        ui.painter().line_segment(
            [header_rect.left_bottom(), header_rect.right_bottom()],
            Stroke::new(1.0, theme::border()),
        );
        let tabs_rect = Rect::from_min_size(
            header_rect.min + egui::vec2(8.0, 4.0),
            egui::vec2(header_rect.width() - 16.0, 32.0),
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(tabs_rect), |ui| {
            ui.horizontal_centered(|ui| {
                for (label, mode) in [("Diff", 0), ("Blame", 1), ("Console", 2)] {
                    let active = match mode {
                        1 => self.show_blame,
                        2 => self.show_console,
                        _ => !self.show_blame && !self.show_console,
                    };
                    if ui.selectable_label(active, label).clicked() {
                        self.show_blame = mode == 1;
                        self.show_console = mode == 2;
                        self.state.lock().unwrap().show_blame = self.show_blame;
                        if self.show_blame {
                            if let Some(fp) = selected_file {
                                crate::git::operations::load_blame(fp, self.state.clone());
                            }
                        }
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if selected_file.is_some()
                        && crate::ui::widgets::icon_btn_mat(
                            ui,
                            crate::icons::CLOSE,
                            16.0,
                            "Close selected diff",
                        )
                        .clicked()
                    {
                        let mut s = self.state.lock().unwrap();
                        s.selected_file = None;
                        s.current_diff = None;
                    }
                    ui.menu_button(crate::icons::rich(crate::icons::TUNE, 18.0), |ui| {
                        ui.label("Diff display");
                        if ui
                            .selectable_value(&mut self.split_view, false, "Inline")
                            .on_hover_text("Show old and new lines together.")
                            .clicked()
                        {
                            ui.close();
                        }
                        if ui
                            .selectable_value(&mut self.split_view, true, "Side by side")
                            .on_hover_text("Show old and new lines in separate columns.")
                            .clicked()
                        {
                            ui.close();
                        }
                        ui.checkbox(&mut self.show_whitespace, "Show whitespace");
                        ui.separator();
                        ui.label("Context lines");
                        ui.horizontal(|ui| {
                            for lines in [1, 3, 5, 10] {
                                ui.selectable_value(&mut self.comparison.diff_context_lines, lines, lines.to_string());
                            }
                        });
                        ui.checkbox(&mut self.comparison.ignore_whitespace, "Ignore whitespace");
                        ui.checkbox(&mut self.comparison.full_file, "Show full file")
                            .on_hover_text("Bounded to 10,000 lines and 1 MiB. Larger files require an external viewer.");
                        if ui.button("Find in diff").clicked() {
                            self.search_open = true;
                            ui.close();
                        }
                        if self.show_console
                            && !console_log.is_empty()
                            && ui.button("Export log…").clicked()
                        {
                            if let Some(path) = rfd::FileDialog::new()
                                .set_file_name("recorde-log.txt")
                                .add_filter("Text", &["txt"])
                                .save_file()
                            {
                                let _ = std::fs::write(path, console_log.join("\n"));
                            }
                            ui.close();
                        }
                    })
                    .response
                    .on_hover_text("Diff display options");
                    if crate::ui::widgets::icon_btn_mat(
                        ui,
                        crate::icons::SEARCH,
                        17.0,
                        "Find in diff (Ctrl/Cmd+F)",
                    )
                    .clicked()
                    {
                        self.search_open = !self.search_open;
                    }
                });
            });
        });
        let file_rect = Rect::from_min_size(
            header_rect.min + egui::vec2(12.0, 42.0),
            egui::vec2(header_rect.width() - 24.0, 32.0),
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(file_rect), |ui| {
            ui.horizontal_centered(|ui| {
                let filename = selected_file.as_deref().unwrap_or("");
                let (directory, basename) = filename
                    .rfind('/')
                    .map(|index| filename.split_at(index + 1))
                    .unwrap_or(("", filename));
                let mut path = egui::text::LayoutJob::default();
                path.append(
                    directory,
                    0.0,
                    egui::TextFormat {
                        font_id: FontId::proportional(13.0),
                        color: theme::text_muted(),
                        ..Default::default()
                    },
                );
                path.append(
                    basename,
                    0.0,
                    egui::TextFormat {
                        font_id: FontId::proportional(13.0),
                        color: theme::text_primary(),
                        ..Default::default()
                    },
                );
                let filename_width =
                    (ui.available_width() - if diff_opt.is_some() { 130.0 } else { 0.0 }).max(60.0);
                ui.add_sized([filename_width, 28.0], egui::Label::new(path).truncate())
                    .on_hover_text(filename);
                if let Some(diff) = diff_opt {
                    let context = match diff_context {
                        DiffContext::StagedFile => "Staged",
                        DiffContext::UnstagedFile => "Unstaged",
                        _ => "Commit",
                    };
                    crate::ui::kit::status_chip(ui, crate::ui::kit::Status::Neutral, context)
                        .on_hover_text(match diff_context {
                            DiffContext::StagedFile => "Staged: included in your next commit",
                            DiffContext::UnstagedFile => {
                                "Unstaged: not yet included in your next commit"
                            }
                            _ => "Changes in this commit",
                        });
                    ui.label(
                        egui::RichText::new(format!("+{} −{}", diff.additions, diff.deletions))
                            .size(12.0)
                            .color(theme::text_muted()),
                    )
                    .on_hover_text(format!(
                        "{} lines added, {} removed",
                        diff.additions, diff.deletions
                    ));
                }
            });
        });

        let mut content_rect = Rect::from_min_max(
            Pos2::new(ui.min_rect().min.x, ui.min_rect().min.y + header_h),
            ui.max_rect().max,
        );

        let mut scroll_to = None;
        if self.comparison.ignore_whitespace || self.comparison.full_file {
            let notice = if self.comparison.ignore_whitespace {
                "Whitespace-only changes hidden. Selections affect visible changed lines only."
            } else {
                "Full file view (maximum 10,000 lines / 1 MiB)."
            };
            let rect = Rect::from_min_size(content_rect.min, Vec2::new(content_rect.width(), 32.0));
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                ui.label(notice);
            });
            content_rect.min.y += 32.0;
        }
        if !self.show_console && !self.show_blame {
            let search_id = ui.id().with("diff_search_reveal");
            let search_height = 36.0
                * crate::ui::motion::progress(
                    ui.ctx(),
                    search_id.with("height"),
                    self.search_open,
                    crate::ui::motion::NORMAL,
                );
            let toolbar =
                Rect::from_min_size(content_rect.min, Vec2::new(content_rect.width(), 36.0));
            content_rect.min.y += search_height;
            ui.scope_builder(egui::UiBuilder::new().max_rect(toolbar), |ui| {
                crate::ui::motion::reveal(ui, search_id, self.search_open, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Find in diff");
                        let changed = ui
                            .add(
                                crate::ui::kit::text_field(&mut self.search_query)
                                    .hint_text("Find in diff")
                                    .desired_width(180.0),
                            )
                            .changed();
                        let previous = ui
                            .button("Previous")
                            .on_hover_text("Find the previous match.")
                            .clicked();
                        let next = ui
                            .button("Next")
                            .on_hover_text("Find the next match.")
                            .clicked();
                        let change = ui
                            .button("Next change")
                            .on_hover_text("Jump to the next changed section.")
                            .clicked();
                        if ui
                            .button("Close")
                            .on_hover_text("Close diff search.")
                            .clicked()
                        {
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
            });
        }

        if !self.show_console
            && !self.show_blame
            && diff_context != DiffContext::CommitDiff
            && diff_opt
                .as_ref()
                .is_some_and(|d| !d.is_binary && !d.is_rename)
        {
            let mut apply = false;
            let mut discard = false;
            let has_selection = !self.selected_rows.is_empty();
            let toolbar = ui.scope_builder(egui::UiBuilder::new().max_rect(content_rect), |ui| {
                let id = ui.id().with("diff_line_selection");
                motion::reveal(ui, id, has_selection, |ui| {
                    // A closing reveal can still paint its contents; disable stale actions.
                    ui.add_enabled_ui(has_selection, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            let label = if diff_context == DiffContext::StagedFile {
                                "Unstage selected lines"
                            } else {
                                "Stage selected lines"
                            };
                            apply = ui
                                .add(kit::small_button(&format!(
                                    "{label} ({})",
                                    self.selected_rows.len()
                                )))
                                .on_hover_text(if diff_context == DiffContext::StagedFile {
                                    concat!(
                                        "Leave these lines out of the next commit; ",
                                        "keep their changes."
                                    )
                                } else {
                                    "Include these lines in your next commit."
                                })
                                .clicked();
                            if diff_context == DiffContext::UnstagedFile {
                                discard = ui
                                    .add(kit::danger_outline_button("Discard selected lines…"))
                                    .on_hover_text("Discard selected lines after confirmation.")
                                    .clicked();
                            }
                            if ui.add(kit::small_button("Clear selection")).clicked() {
                                self.selected_rows.clear();
                                self.selection_anchor = None;
                            }
                        });
                    });
                    ui.add_space(kit::space::XS);
                });
            });
            content_rect.min.y += toolbar.response.rect.height();
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
            ui.scope_builder(egui::UiBuilder::new().max_rect(content_rect), |ui| {
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
                                        .size(13.0),
                                );
                            });
                        } else {
                            for line in console_log {
                                let line = crate::auth::redact::text(line);
                                let color = if line.starts_with("error:") {
                                    theme::accent_red()
                                } else if line.starts_with('$') {
                                    theme::accent_teal()
                                } else {
                                    theme::text_secondary()
                                };
                                ui.horizontal(|ui| {
                                    ui.add_space(12.0);
                                    ui.label(
                                        egui::RichText::new(line)
                                            .color(color)
                                            .size(13.0)
                                            .font(FontId::monospace(13.0)),
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
            ui.scope_builder(egui::UiBuilder::new().max_rect(content_rect), |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        ui.add_space(4.0);
                        if blame_lines.is_empty() {
                            ui.centered_and_justified(|ui| {
                                let message = if selected_file.is_some() {
                                    "Loading blame..."
                                } else {
                                    concat!(
                                        "Blame shows who last changed each line ",
                                        "and in which commit. ",
                                        "Select a file to see it."
                                    )
                                };
                                ui.label(
                                    egui::RichText::new(message)
                                        .color(theme::text_muted())
                                        .size(13.0),
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
                                        theme::accent_teal()
                                    } else {
                                        theme::bg_panel()
                                    };
                                    ui.label(
                                        egui::RichText::new(&line.short_oid)
                                            .monospace()
                                            .color(oid_color)
                                            .size(12.0),
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
                                        egui::RichText::new(author).size(12.0).color(author_color),
                                    );
                                    ui.add_space(4.0);
                                    // Line number
                                    ui.label(
                                        egui::RichText::new(format!("{:>4}", line.lineno))
                                            .monospace()
                                            .color(theme::text_muted())
                                            .size(12.0),
                                    );
                                    ui.add_space(6.0);
                                    // Content
                                    ui.label(
                                        egui::RichText::new(&line.content)
                                            .monospace()
                                            .color(theme::text_secondary())
                                            .size(12.0),
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
            ui.scope_builder(egui::UiBuilder::new().max_rect(content_rect), |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("binary_preview")
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
        if let Some(diff) = diff_opt.as_ref().filter(|d| {
            !d.is_binary
                && d.hunks.is_empty()
                && !d.raw_patch.is_empty()
                && (!d.canonical_hunks.as_ref().is_some_and(|h| !h.is_empty())
                    || d.is_rename
                    || d.old_mode != d.new_mode)
        }) {
            ui.scope_builder(egui::UiBuilder::new().max_rect(content_rect), |ui| {
                egui::ScrollArea::both().show(ui, |ui| {
                    ui.label("File metadata changed");
                    if diff.is_rename {
                        ui.label(format!(
                            "Renamed: {} → {}",
                            diff.old_path.as_deref().unwrap_or("?"),
                            diff.file_path
                        ));
                    }
                    if diff.old_mode != diff.new_mode {
                        ui.label(format!(
                            "File mode: {:06o} → {:06o}",
                            diff.old_mode, diff.new_mode
                        ));
                    }
                    if !self.comparison.ignore_whitespace {
                        ui.monospace(String::from_utf8_lossy(&diff.raw_patch));
                    }
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
        } else if self.items.is_empty() && self.comparison.ignore_whitespace {
            Some("No visible changes. Whitespace-only changes may be hidden.")
        } else if self.items.is_empty() {
            Some("No changes.")
        } else {
            None
        };
        if let Some(msg) = empty_msg {
            ui.scope_builder(egui::UiBuilder::new().max_rect(content_rect), |ui| {
                if selected_file.is_none() {
                    crate::ui::kit::empty_state(
                        ui,
                        crate::icons::DIFFERENCE,
                        "No file selected",
                        "Select a file in Changes or in a commit to see what changed.",
                        None,
                    );
                } else {
                    ui.centered_and_justified(|ui| {
                        ui.label(
                            egui::RichText::new(msg)
                                .color(theme::text_muted())
                                .size(13.0),
                        );
                    });
                }
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
                self.content_width,
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
                self.content_width,
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
    content_width: f32,
    diff_context: DiffContext,
    selected: &mut std::collections::BTreeSet<usize>,
    anchor: &mut Option<usize>,
    scroll_to: Option<usize>,
    code: bool,
    whitespace: bool,
) -> Vec<DiffAction> {
    let is_workdir = diff_context != DiffContext::CommitDiff;
    let is_staged = diff_context == DiffContext::StagedFile;
    let total_h = items.len() as f32 * ROW_HEIGHT;
    let actions = std::cell::RefCell::new(Vec::<DiffAction>::new());

    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        ui.set_clip_rect(ui.clip_rect().intersect(rect));
        egui::ScrollArea::both()
            .content_margin(egui::Margin::same(0))
            .auto_shrink([false; 2])
            .show_viewport(ui, |ui, viewport| {
                let width = unified_content_width(viewport.width(), content_width);
                // Painted rows must not take keyboard focus: that would also
                // activate egui_wants_keyboard_input and suppress diff copying.
                let (resp, painter) = ui.allocate_painter(
                    Vec2::new(width, total_h.max(viewport.height())),
                    Sense::CLICK,
                );
                let origin = resp.rect.min;
                let clip = painter.clip_rect();
                if let Some(row) = scroll_to {
                    ui.scroll_to_rect(
                        Rect::from_min_size(
                            Pos2::new(clip.left(), origin.y + row as f32 * ROW_HEIGHT),
                            Vec2::new(clip.width(), ROW_HEIGHT),
                        ),
                        Some(egui::Align::Center),
                    );
                }

                let click_pos = if resp.clicked() {
                    resp.interact_pointer_pos()
                } else {
                    None
                };
                let hover_pos = resp.hover_pos();
                // How to stage single lines is explained where it applies: a
                // tooltip on the changed line under the pointer.
                let mut line_hint: Option<&str> = None;

                let first = ((clip.top() - origin.y) / ROW_HEIGHT).floor().max(0.0) as usize;
                let last = (((clip.bottom() - origin.y) / ROW_HEIGHT).ceil() as usize + 1)
                    .min(items.len());

                for i in first..last {
                    let y = origin.y + i as f32 * ROW_HEIGHT;
                    let row_rect =
                        Rect::from_min_size(Pos2::new(origin.x, y), Vec2::new(width, ROW_HEIGHT));

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
                                clip.right() - 220.0
                            } else {
                                origin.x + width - 10.0
                            };
                            painter
                                .with_clip_rect(Rect::from_min_max(
                                    Pos2::new(origin.x + 10.0, y),
                                    Pos2::new(text_end_x, y + ROW_HEIGHT),
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
                                    &[("Unstage Hunk", theme::accent_teal(), false)]
                                } else {
                                    &[
                                        ("Stage Hunk", theme::accent_teal(), false),
                                        ("Discard Hunk", theme::accent_red(), true),
                                    ]
                                };

                                let btn_w = 94.0;
                                let btn_h = ROW_HEIGHT - 4.0;
                                // Extra space keeps Discard from sitting flush against
                                // Stage, so a slip of the pointer lands on nothing.
                                let discard_gap = 16.0;
                                let gaps = if btns.iter().any(|(_, _, discard)| *discard) {
                                    discard_gap
                                } else {
                                    0.0
                                };
                                let mut btn_x =
                                    clip.right() - (btns.len() as f32 * (btn_w + 4.0)) - gaps - 4.0;

                                for (label, col, is_discard) in btns {
                                    if *is_discard {
                                        btn_x += discard_gap;
                                    }
                                    let btn_rect = Rect::from_min_size(
                                        Pos2::new(btn_x, y + 2.0),
                                        Vec2::new(btn_w, btn_h),
                                    );
                                    let button = ui
                                        .scope_builder(
                                            egui::UiBuilder::new()
                                                .id_salt(("diff_hunk_button", hi, label))
                                                .max_rect(btn_rect),
                                            |ui| {
                                                // Preserve the dense diff row's height.
                                                ui.spacing_mut().button_padding =
                                                    Vec2::new(4.0, 0.0);
                                                ui.spacing_mut().interact_size.y = btn_h;
                                                ui.put(
                                                    btn_rect,
                                                    kit::small_button("")
                                                        .min_size(Vec2::new(btn_w, btn_h)),
                                                )
                                            },
                                        )
                                        .inner;
                                    button.widget_info(|| {
                                        egui::WidgetInfo::labeled(
                                            egui::WidgetType::Button,
                                            ui.is_enabled(),
                                            *label,
                                        )
                                    });
                                    // Keep the kit surface and the action's text tint.
                                    painter.text(
                                        button.rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        *label,
                                        FontId::proportional(12.0),
                                        if ui.is_enabled() {
                                            *col
                                        } else {
                                            theme::text_muted()
                                        },
                                    );
                                    let button = button.on_hover_text(if *is_discard {
                                        "Discard this section's changes after confirmation."
                                    } else if is_staged {
                                        concat!(
                                            "Leave this section out of the next commit; ",
                                            "keep its changes."
                                        )
                                    } else {
                                        "Include this section's changes in your next commit."
                                    });
                                    if button.clicked() {
                                        let mut acts = actions.borrow_mut();
                                        if *is_discard {
                                            acts.push(DiffAction::DiscardHunk(hi));
                                        } else if is_staged {
                                            acts.push(DiffAction::UnstageHunk(hi));
                                        } else {
                                            acts.push(DiffAction::StageHunk(hi));
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
                                    theme::accent_blue().gamma_multiply(0.25),
                                );
                            }
                            let cy = y + ROW_HEIGHT / 2.0;
                            painter.text(
                                Pos2::new(origin.x + BAR_W + 4.0 + 30.0, cy),
                                egui::Align2::RIGHT_CENTER,
                                old_no,
                                FontId::monospace(12.0),
                                theme::text_muted(),
                            );
                            painter.text(
                                Pos2::new(origin.x + BAR_W + 4.0 + 68.0, cy),
                                egui::Align2::RIGHT_CENTER,
                                new_no,
                                FontId::monospace(12.0),
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
                                    Pos2::new(origin.x + width - 4.0, y + ROW_HEIGHT),
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
                                    let over_numbers =
                                        hover_pos.is_some_and(|p| p.x < origin.x + U_GUTTER);
                                    line_hint = Some(line_hint_text(over_numbers, is_staged));
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
                                                true,
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
                            if !is_workdir && click_pos.is_some_and(|p| row_rect.contains(p)) {
                                select_line(
                                    items,
                                    selected,
                                    anchor,
                                    i,
                                    ui.input(|input| input.modifiers),
                                    false,
                                );
                            }
                        }

                        Item::Binary => {
                            painter.text(
                                Pos2::new(origin.x + 12.0, y + ROW_HEIGHT / 2.0),
                                egui::Align2::LEFT_CENTER,
                                "Binary file",
                                FontId::proportional(13.0),
                                theme::text_muted(),
                            );
                        }
                    }
                }
                let pointer_row = resp.interact_pointer_pos().and_then(|p| {
                    let row = ((p.y - origin.y) / ROW_HEIGHT).floor() as usize;
                    matches!(items.get(row), Some(Item::Line { .. })).then_some(row)
                });
                copy_menu(ui, &resp, items, selected, pointer_row);
                copy_shortcut(ui, items, selected, resp.hovered());
                if let Some(hint) = line_hint {
                    resp.on_hover_text_at_pointer(hint);
                }
            });
    });

    actions.into_inner()
}

/// Tooltip for a changed line in a working-tree diff. Clicking the line numbers
/// stages (or unstages) that one line at once; clicking the code selects it.
fn line_hint_text(over_numbers: bool, staged: bool) -> &'static str {
    match (over_numbers, staged) {
        (true, false) => "Click to stage this line now.",
        (true, true) => "Click to unstage this line now.",
        (false, _) => {
            "Click to select this line, then stage or discard the selection.              Ctrl/Cmd adds a line; Shift selects a range."
        }
    }
}

// ── Split renderer ────────────────────────────────────────────────────────────

fn select_line(
    items: &[Item],
    selected: &mut std::collections::BTreeSet<usize>,
    anchor: &mut Option<usize>,
    row: usize,
    modifiers: egui::Modifiers,
    changes_only: bool,
) {
    if modifiers.shift {
        let start = anchor.unwrap_or(row);
        if !modifiers.command && !modifiers.ctrl {
            selected.clear();
        }
        for index in start.min(row)..=start.max(row) {
            if matches!(
                items.get(index),
                Some(Item::Line { origin, .. }) if !changes_only || matches!(*origin, '+' | '-')
            ) {
                selected.insert(index);
            }
        }
    } else if modifiers.command || modifiers.ctrl {
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
    content_width: f32,
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
    let total_h = rows.len() as f32 * ROW_HEIGHT;

    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        ui.set_clip_rect(ui.clip_rect().intersect(rect));
        egui::ScrollArea::both()
            .content_margin(egui::Margin::same(0))
            .auto_shrink([false; 2])
            .show_viewport(ui, |ui, viewport| {
                let half = split_column_width(viewport.width(), content_width);
                let (resp, painter) = ui.allocate_painter(
                    Vec2::new(half * 2.0, total_h.max(viewport.height())),
                    Sense::CLICK,
                );
                let origin = resp.rect.min;
                let clip = painter.clip_rect();
                if let Some(row) = scroll_to {
                    ui.scroll_to_rect(
                        Rect::from_min_size(
                            Pos2::new(clip.left(), origin.y + row as f32 * ROW_HEIGHT),
                            Vec2::new(clip.width(), ROW_HEIGHT),
                        ),
                        Some(egui::Align::Center),
                    );
                }

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
                                Vec2::new(half * 2.0, ROW_HEIGHT),
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
                                let Some(line) = side.as_ref() else {
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
                                        theme::accent_blue().gamma_multiply(0.25),
                                    );
                                }
                                if context != DiffContext::CommitDiff && !line.changed {
                                    continue;
                                }
                                let response = ui.interact(
                                    side_rect.intersect(clip),
                                    ui.id().with(("split_line", i, is_left)),
                                    Sense::CLICK,
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
                                    if context != DiffContext::CommitDiff
                                        && response
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
                                            context != DiffContext::CommitDiff,
                                        );
                                    }
                                }
                                copy_menu(ui, &response, items, selected, Some(line.unified_row));
                            }
                        }
                    }
                }
                let pointer_row = resp.interact_pointer_pos().and_then(|p| {
                    let row = ((p.y - origin.y) / ROW_HEIGHT).floor() as usize;
                    match rows.get(row) {
                        Some(SplitRow::Pair { left, right }) => {
                            let side = if p.x < origin.x + half { left } else { right };
                            side.as_ref().map(|line| line.unified_row)
                        }
                        _ => None,
                    }
                });
                copy_menu(ui, &resp, items, selected, pointer_row);
                let hovered = ui.input(|i| {
                    i.pointer
                        .hover_pos()
                        .is_some_and(|p| rect.intersect(clip).contains(p))
                });
                copy_shortcut(ui, items, selected, hovered);
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
            theme::accent_red(),
            theme::text_primary(),
        ),
        Some(sl) if sl.changed && !is_left => (
            theme::bg_diff_add(),
            theme::accent_green(),
            theme::text_primary(),
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
            FontId::monospace(12.0),
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

/// Use a rounded-up monospace advance so ASCII lines always fit. Tabs and wide
/// Unicode are conservatively approximated, including the whitespace display.
fn widest_content_width(items: &[Item], glyph_width: f32) -> f32 {
    items
        .iter()
        .filter_map(|item| match item {
            Item::Line { content, .. } => Some(
                content
                    .chars()
                    .map(|ch| match ch {
                        '\t' => 4,
                        ch if ch.is_ascii() => 1,
                        _ => 2,
                    })
                    .sum::<usize>(),
            ),
            _ => None,
        })
        .max()
        .unwrap_or(0) as f32
        * glyph_width
}

fn unified_content_width(viewport_width: f32, content_width: f32) -> f32 {
    viewport_width.max(U_GUTTER + 18.0 + content_width + CONTENT_PADDING)
}

fn split_column_width(viewport_width: f32, content_width: f32) -> f32 {
    (viewport_width / 2.0).max(S_GUTTER + 16.0 + content_width + CONTENT_PADDING)
}

fn selected_text(items: &[Item], rows: &BTreeSet<usize>) -> String {
    rows.iter()
        .filter_map(|&row| match items.get(row) {
            Some(Item::Line { content, .. }) => Some(content.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Rows are selected with the pointer, so copying requires it over the diff; this
/// leaves Ctrl/Cmd+C elsewhere (labels, other panes) to their own handlers.
fn copy_shortcut(ui: &Ui, items: &[Item], selected: &BTreeSet<usize>, hovered: bool) {
    if ui.ctx().egui_wants_keyboard_input() || !hovered || selected.is_empty() {
        return;
    }
    if ui.input(|i| {
        i.events
            .iter()
            .any(|event| matches!(event, egui::Event::Copy))
    }) {
        ui.ctx().copy_text(selected_text(items, selected));
    }
}

fn copy_menu(
    ui: &Ui,
    response: &egui::Response,
    items: &[Item],
    selected: &BTreeSet<usize>,
    pointer_row: Option<usize>,
) {
    let id = response.id.with("copy_line");
    if response.secondary_clicked() {
        // Capture the content when the menu opens, before the pointer moves to
        // a menu item (or a newly loaded diff replaces the original rows).
        let line = pointer_row.and_then(|row| match items.get(row) {
            Some(Item::Line { content, .. }) => Some(content.clone()),
            _ => None,
        });
        ui.data_mut(|data| data.insert_temp(id, line));
    }
    response.context_menu(|ui| {
        if ui
            .add_enabled(
                !selected.is_empty(),
                egui::Button::new("Copy selected lines"),
            )
            .clicked()
        {
            ui.ctx().copy_text(selected_text(items, selected));
            ui.close();
        }
        let line = ui.data_mut(|data| data.get_temp::<Option<String>>(id).flatten());
        if ui
            .add_enabled(line.is_some(), egui::Button::new("Copy line"))
            .clicked()
        {
            if let Some(line) = line {
                ui.ctx().copy_text(line);
            }
            ui.close();
        }
    });
}

fn line_colors(ch: char) -> (Color32, Color32, Color32) {
    match ch {
        '+' => (
            theme::bg_diff_add(),
            theme::accent_green(),
            theme::text_primary(),
        ),
        '-' => (
            theme::bg_diff_del(),
            theme::accent_red(),
            theme::text_primary(),
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
            theme::bg_diff_del(),
            theme::accent_red(),
            theme::text_primary(),
        ))
    } else if content.starts_with("=======") || content.starts_with("|||||||") {
        Some((
            theme::bg_diff_hunk(),
            theme::accent_yellow(),
            theme::text_primary(),
        ))
    } else if content.starts_with(">>>>>>>") {
        Some((
            theme::bg_diff_add(),
            theme::accent_green(),
            theme::text_primary(),
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
        'A' | '?' => ("A", theme::accent_green()),
        'D' => ("D", theme::accent_red()),
        'R' => ("R", theme::accent_yellow()),
        _ => ("M", theme::accent_yellow()),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{DiffHunk, DiffLine};

    fn diff(lines: &[(char, &str)]) -> FileDiff {
        FileDiff {
            canonical_hunks: None,
            binary_preview: None,
            raw_patch: Vec::new(),
            old_path: None,
            is_rename: false,
            old_mode: 0o100644,
            new_mode: 0o100644,
            commit_oid: String::new(),
            file_path: "example.rs".into(),
            hunks: vec![DiffHunk {
                header: "@@ -1,3 +1,3 @@".into(),
                lines: lines
                    .iter()
                    .enumerate()
                    .map(|(index, &(origin, content))| DiffLine {
                        origin,
                        content: content.into(),
                        old_lineno: (origin != '+').then_some(index as u32 + 1),
                        new_lineno: (origin != '-').then_some(index as u32 + 1),
                    })
                    .collect(),
            }],
            is_binary: false,
            additions: lines.iter().filter(|(origin, _)| *origin == '+').count(),
            deletions: lines.iter().filter(|(origin, _)| *origin == '-').count(),
        }
    }

    #[test]
    fn diff_controls_hidden_whitespace_notice_never_renders_canonical_text() {
        for metadata in [false, true] {
            let ctx = egui::Context::default();
            ctx.set_fonts(crate::icons::font_definitions());
            let mut view = DiffView::new(crate::state::new_shared_state());
            view.comparison.ignore_whitespace = true;
            let mut hidden = diff(&[('-', "sentinel hidden old"), ('+', "sentinel hidden new")]);
            hidden.canonical_hunks = Some(hidden.hunks.clone());
            hidden.hunks.clear();
            hidden.raw_patch = b"sentinel canonical whitespace patch".to_vec();
            if metadata {
                hidden.new_mode = 0o100755;
                hidden.is_rename = true;
                hidden.old_path = Some("before.rs".into());
            }
            let file_diff = Some(Arc::new(hidden));
            let selected = Some("example.rs".into());
            let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 500.0));
            ctx.begin_pass(egui::RawInput {
                screen_rect: Some(rect),
                ..Default::default()
            });
            let mut ui = Ui::new(
                ctx.clone(),
                egui::Id::new("hidden_whitespace"),
                egui::UiBuilder::new().max_rect(rect),
            );
            let actions = view.show_diff_area(
                &mut ui,
                &file_diff,
                &selected,
                &[],
                DiffContext::UnstagedFile,
                &[],
                0,
            );
            assert!(actions.is_empty());
            let mut output = ctx.end_pass();
            output.textures_delta.clear();
            let text = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) => Some(text.galley.text()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            assert!(text.contains("Whitespace-only changes hidden"));
            assert!(!text.contains("sentinel"));
            assert_eq!(text.contains("File metadata changed"), metadata);
            assert_eq!(text.contains("File mode: 100644"), metadata);
            assert_eq!(text.contains("100755"), metadata);
            assert_eq!(text.contains("Renamed: before.rs"), metadata);
        }
    }

    #[test]
    fn selected_text_skips_headers_and_preserves_contents_order_and_empty_lines() {
        let items = flatten_unified(&diff(&[
            (' ', " context\r\n"),
            ('-', "-source\n"),
            ('+', "not selected\n"),
            ('+', "\n"),
            (' ', "last\n"),
            ('+', ""),
        ]));
        let rows = BTreeSet::from([6, 0, 5, 2, 1, 4, 99]);
        assert_eq!(selected_text(&items, &rows), " context\n-source\n\nlast\n");
        assert_eq!(selected_text(&items, &BTreeSet::from([0, 99])), "");
        assert_eq!(selected_text(&items, &BTreeSet::new()), "");
    }

    #[test]
    fn long_lines_expand_both_layouts_and_short_lines_fit_the_viewport() {
        let long = "M".repeat(400);
        let items = flatten_unified(&diff(&[('+', &long)]));
        let width = widest_content_width(&items, 8.0);
        assert!(width > 800.0);
        assert!(unified_content_width(800.0, width) > 800.0);
        assert!(split_column_width(800.0, width) > 400.0);

        let short = flatten_unified(&diff(&[('+', "short")]));
        let width = widest_content_width(&short, 8.0);
        assert_eq!(unified_content_width(800.0, width), 800.0);
        assert_eq!(split_column_width(800.0, width), 400.0);
        let tabs = flatten_unified(&diff(&[(' ', "\t a")]));
        assert_eq!(widest_content_width(&tabs, 8.0), 48.0);
    }

    #[test]
    fn commit_range_selection_includes_context_but_workdir_selection_keeps_changes_only() {
        let items = flatten_unified(&diff(&[('-', "old"), (' ', "context"), ('+', "new")]));
        let mut selected = BTreeSet::new();
        let mut anchor = Some(1);
        let shift = egui::Modifiers {
            shift: true,
            ..Default::default()
        };
        select_line(&items, &mut selected, &mut anchor, 3, shift, false);
        assert_eq!(selected_text(&items, &selected), "old\ncontext\nnew");
        select_line(&items, &mut selected, &mut anchor, 3, shift, true);
        assert_eq!(selected_text(&items, &selected), "old\nnew");

        let ctrl = egui::Modifiers {
            ctrl: true,
            ..Default::default()
        };
        select_line(&items, &mut selected, &mut anchor, 1, ctrl, false);
        assert_eq!(selected, BTreeSet::from([3]));
        let command = egui::Modifiers {
            command: true,
            ..Default::default()
        };
        select_line(&items, &mut selected, &mut anchor, 2, command, false);
        assert_eq!(selected, BTreeSet::from([2, 3]));
    }

    #[test]
    fn unified_renderer_allocates_long_rows_beyond_the_viewport() {
        for (text, should_scroll) in [("M".repeat(400), true), ("short".into(), false)] {
            let ctx = egui::Context::default();
            // The diff toolbar draws Material icons, as in the app.
            ctx.set_fonts(crate::icons::font_definitions());
            let state = crate::state::new_shared_state();
            {
                let mut state = state.lock().unwrap();
                state.selected_file = Some("example.rs".into());
                state.current_diff = Some(Arc::new(diff(&[('+', &text)])));
                state.diff_context = DiffContext::CommitDiff;
            }
            let mut view = DiffView::new(state);
            let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 200.0));
            ctx.begin_pass(egui::RawInput {
                screen_rect: Some(rect),
                ..Default::default()
            });
            let mut ui = Ui::new(
                ctx.clone(),
                egui::Id::new("diff_width_test"),
                egui::UiBuilder::new().max_rect(rect),
            );
            view.show(&mut ui);
            assert_eq!(view.content_width > rect.width(), should_scroll);
            let mut output = ctx.end_pass();
            output.textures_delta.clear();
            // Row backgrounds use exactly the allocated painter width. The
            // clipping viewport stays narrow even when that allocation grows.
            let (row_width, clip_width) = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Rect(row)
                        if row.fill == theme::bg_diff_add() && row.rect.width() > BAR_W =>
                    {
                        Some((row.rect.width(), shape.clip_rect.width()))
                    }
                    _ => None,
                })
                .expect("added line background");
            assert_eq!(row_width > rect.width(), should_scroll);
            assert!(clip_width <= rect.width());
        }
    }

    #[test]
    fn selection_toolbar_replaces_the_hint_and_uses_a_distinct_discard_label() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::icons::font_definitions());
        ctx.global_style_mut(|style| style.animation_time = 0.0);
        let mut view = DiffView::new(crate::state::new_shared_state());
        let file_diff = Some(Arc::new(diff(&[('-', "old"), ('+', "new")])));
        let selected_file = Some("example.rs".to_owned());
        let draw = |view: &mut DiffView| {
            let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 300.0));
            ctx.begin_pass(egui::RawInput {
                screen_rect: Some(rect),
                ..Default::default()
            });
            let mut ui = Ui::new(
                ctx.clone(),
                egui::Id::new("selection_toolbar_test"),
                egui::UiBuilder::new().max_rect(rect),
            );
            let actions = view.show_diff_area(
                &mut ui,
                &file_diff,
                &selected_file,
                &[],
                DiffContext::UnstagedFile,
                &[],
                0,
            );
            assert!(actions.is_empty());
            let mut output = ctx.end_pass();
            output.textures_delta.clear();
            output
        };
        let has_text = |output: &egui::FullOutput, label: &str| {
            output.shapes.iter().any(|shape| {
                matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == label)
            })
        };
        // Guidance is a pointer tooltip on changed lines, never a banner.
        let output = draw(&mut view);
        assert!(!output.shapes.iter().any(|shape| matches!(
            &shape.shape,
            egui::Shape::Text(text) if text.galley.text().starts_with("Tip:")
        )));
        assert!(!has_text(&output, "Discard selected lines…"));
        view.selected_rows.insert(1);
        let output = draw(&mut view);
        assert!(has_text(&output, "Stage selected lines (1)"));
        assert!(has_text(&output, "Discard selected lines…"));
        assert!(!has_text(&output, "Discard"));
        view.selected_rows.clear();
        let output = draw(&mut view);
        assert!(!has_text(&output, "Discard selected lines…"));
    }

    #[test]
    fn line_tooltips_match_what_a_click_does() {
        // Line numbers stage or unstage immediately; the code area selects.
        assert!(line_hint_text(true, false).contains("stage this line"));
        assert!(line_hint_text(true, true).contains("unstage this line"));
        assert!(line_hint_text(false, false).starts_with("Click to select"));
    }

    #[test]
    fn hunk_buttons_emit_the_existing_actions_without_selecting_lines() {
        for (context, label) in [
            (DiffContext::UnstagedFile, "Stage Hunk"),
            (DiffContext::UnstagedFile, "Discard Hunk"),
            (DiffContext::StagedFile, "Unstage Hunk"),
        ] {
            let ctx = egui::Context::default();
            ctx.set_fonts(crate::icons::font_definitions());
            let items = flatten_unified(&diff(&[('-', "old"), ('+', "new")]));
            let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 200.0));
            let mut selected = BTreeSet::new();
            let mut anchor = None;
            let mut draw = |events| {
                ctx.begin_pass(egui::RawInput {
                    screen_rect: Some(rect),
                    events,
                    ..Default::default()
                });
                let mut ui = Ui::new(
                    ctx.clone(),
                    egui::Id::new("hunk_buttons_test"),
                    egui::UiBuilder::new().max_rect(rect),
                );
                let actions = render_unified(
                    &mut ui,
                    rect,
                    &items,
                    100.0,
                    context,
                    &mut selected,
                    &mut anchor,
                    None,
                    false,
                    false,
                );
                let mut output = ctx.end_pass();
                output.textures_delta.clear();
                (actions, output)
            };
            let (_, output) = draw(vec![]);
            let position = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == label => {
                        Some(text.galley.rect.translate(text.pos.to_vec2()).center())
                    }
                    _ => None,
                })
                .expect("Hunk action label is rendered");
            let mut actions = Vec::new();
            for pressed in [true, false] {
                let (emitted, _) = draw(vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ]);
                actions.extend(emitted);
            }
            assert!(matches!(
                (label, actions.as_slice()),
                ("Stage Hunk", [DiffAction::StageHunk(0)])
                    | ("Discard Hunk", [DiffAction::DiscardHunk(0)])
                    | ("Unstage Hunk", [DiffAction::UnstageHunk(0)])
            ));
            assert!(selected.is_empty());
            assert!(anchor.is_none());
        }
    }

    #[test]
    fn copy_shortcut_copies_selected_rows_only_over_the_diff_without_text_focus() {
        for (focus_text, hovered) in [(false, true), (true, true), (false, false)] {
            let ctx = egui::Context::default();
            let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 200.0));
            ctx.begin_pass(egui::RawInput {
                screen_rect: Some(rect),
                events: vec![egui::Event::Copy],
                ..Default::default()
            });
            let mut ui = Ui::new(
                ctx.clone(),
                egui::Id::new("diff_copy_test"),
                egui::UiBuilder::new().max_rect(rect),
            );
            if focus_text {
                let mut text = "editor text".to_string();
                ui.add(crate::ui::kit::text_field(&mut text))
                    .request_focus();
                assert!(ctx.egui_wants_keyboard_input());
            }
            let items = flatten_unified(&diff(&[('-', "old"), ('+', "new")]));
            copy_shortcut(&ui, &items, &BTreeSet::from([1, 2]), hovered);
            let mut output = ctx.end_pass();
            output.textures_delta.clear();
            let copied_diff = output.platform_output.commands.iter().any(|command| {
                matches!(command, egui::OutputCommand::CopyText(text) if text == "old\nnew")
            });
            assert_eq!(copied_diff, hovered && !focus_text);
        }
    }
}
