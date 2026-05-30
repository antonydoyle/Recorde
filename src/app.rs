use egui::{Color32, Context, Key, Modifiers, RichText, Stroke};

use crate::config::Config;
use crate::git::{branches, operations::load_repo, rebase, remote};
use crate::state::{ModalKind, RebaseEntry, new_shared_state, SharedState};
use crate::theme;
use crate::ui::{
    commit_graph::CommitGraphView,
    detail_panel::DetailPanel,
    diff_view::DiffView,
    rebase_modal::{RebaseModal, RebaseModalResult},
    sidebar::{Sidebar, SidebarAction},
    staging_panel::StagingPanel,
    status_bar::StatusBar,
    toolbar::{Toolbar, ToolbarAction},
};

#[derive(PartialEq, Clone, Copy)]
pub enum RightTab { Changes, Details }

pub struct RecordeApp {
    state: SharedState,
    toolbar: Toolbar,
    sidebar: Sidebar,
    commit_graph_view: CommitGraphView,
    diff_view: DiffView,
    detail_panel: DetailPanel,
    staging_panel: StagingPanel,
    status_bar: StatusBar,
    right_tab: RightTab,
    config: Config,
    dark_theme: bool,
    // Modal form inputs
    create_branch_name: String,
    create_branch_checkout: bool,
    create_tag_name: String,
    // New modals
    command_palette_open: bool,
    command_palette_query: String,
    branch_switcher_open: bool,
    branch_switcher_query: String,
    settings_open: bool,
    // Interactive rebase
    rebase_modal: Option<RebaseModal>,
}

impl RecordeApp {
    pub fn new(cc: &eframe::CreationContext<'_>, initial_repo: Option<std::path::PathBuf>) -> Self {
        let config = Config::load();
        let dark_theme = config.dark_theme;
        theme::apply(&cc.egui_ctx, dark_theme);

        let state = new_shared_state();
        let to_open = initial_repo.or_else(|| config.recent_repos.first().cloned());
        if let Some(path) = to_open {
            load_repo(path, state.clone());
        }

        Self {
            toolbar: Toolbar::new(state.clone()),
            sidebar: Sidebar::new(state.clone()),
            commit_graph_view: CommitGraphView::new(state.clone()),
            diff_view: DiffView::new(state.clone()),
            detail_panel: DetailPanel::new(state.clone()),
            staging_panel: StagingPanel::new(state.clone()),
            status_bar: StatusBar::new(state.clone()),
            right_tab: RightTab::Changes,
            config,
            dark_theme,
            state,
            create_branch_name: String::new(),
            create_branch_checkout: true,
            create_tag_name: String::new(),
            command_palette_open: false,
            command_palette_query: String::new(),
            branch_switcher_open: false,
            branch_switcher_query: String::new(),
            settings_open: false,
            rebase_modal: None,
        }
    }

    // ── Action handlers ───────────────────────────────────────────────────────

    fn handle_toolbar(&mut self, ctx: &Context, action: ToolbarAction) {
        match action {
            ToolbarAction::OpenRepo(path) => {
                self.config.add_recent(path.clone());
                load_repo(path, self.state.clone());
            }
            ToolbarAction::Stash        => branches::stash_save(self.state.clone()),
            ToolbarAction::CreateBranch => {
                self.state.lock().unwrap().modal = ModalKind::CreateBranch { from_oid: None };
            }
            ToolbarAction::Fetch => remote::fetch_all(self.state.clone()),
            ToolbarAction::Pull  => remote::pull_branch(self.state.clone()),
            ToolbarAction::Push  => remote::push_branch(self.state.clone()),
            ToolbarAction::ToggleTheme => {
                self.dark_theme = !self.dark_theme;
                self.config.dark_theme = self.dark_theme;
                self.config.save();
                theme::apply(ctx, self.dark_theme);
            }
            ToolbarAction::ToggleSearch => {
                let mut s = self.state.lock().unwrap();
                s.search_active = !s.search_active;
                if !s.search_active { s.search_query.clear(); }
            }
            ToolbarAction::OpenCommandPalette => self.command_palette_open = true,
            ToolbarAction::OpenBranchSwitcher => self.branch_switcher_open = true,
        }
    }

    fn handle_sidebar(&mut self, action: SidebarAction) {
        match action {
            SidebarAction::CheckoutBranch(n)       => branches::checkout_branch(&n, self.state.clone()),
            SidebarAction::OpenCreateBranch { from_oid } =>
                self.state.lock().unwrap().modal = ModalKind::CreateBranch { from_oid },
            SidebarAction::DeleteBranch(n)  =>
                self.state.lock().unwrap().modal = ModalKind::ConfirmDeleteBranch { branch_name: n },
            SidebarAction::CreateTag(_)     =>
                self.state.lock().unwrap().modal = ModalKind::CreateTag { from_oid: None },
            SidebarAction::DeleteTag(n)     =>
                self.state.lock().unwrap().modal = ModalKind::ConfirmDeleteTag { tag_name: n },
            SidebarAction::StashApply(i) => branches::stash_apply(i, self.state.clone()),
            SidebarAction::StashPop(i)   => branches::stash_pop(i, self.state.clone()),
            SidebarAction::StashDrop(i)  => branches::stash_drop(i, self.state.clone()),
        }
    }

    // ── Open rebase modal ─────────────────────────────────────────────────────

    fn open_rebase_modal(&mut self, upstream_oid: String) {
        let repo_path = self.state.lock().unwrap().repo_path.clone();
        let Some(path) = repo_path else { return };
        let upstream_summary = (|| -> Option<String> {
            let repo   = git2::Repository::open(&path).ok()?;
            let oid    = git2::Oid::from_str(&upstream_oid).ok()?;
            let commit = repo.find_commit(oid).ok()?;
            commit.summary().map(str::to_owned)
        })().unwrap_or_default();
        let entries = rebase::load_rebase_entries(&path, &upstream_oid);
        match entries {
            Some(e) if !e.is_empty() =>
                self.rebase_modal = Some(RebaseModal::new(upstream_oid, upstream_summary, e)),
            _ =>
                self.state.lock().unwrap().error =
                    Some("No commits to rebase between this commit and HEAD.".into()),
        }
    }

    // ── Modal rendering ───────────────────────────────────────────────────────

    fn render_modals(&mut self, ctx: &Context) {
        let modal_kind = self.state.lock().unwrap().modal.clone();
        match modal_kind {
            ModalKind::None => {}

            ModalKind::CreateBranch { from_oid } => modal_window(ctx, "Create Branch", |ui| {
                ui.label(styled_label("Branch name"));
                ui.add_space(4.0);
                let r = ui.add(egui::TextEdit::singleline(&mut self.create_branch_name)
                    .desired_width(f32::INFINITY).hint_text("e.g. feat/my-feature"));
                r.request_focus();
                if let Some(ref oid) = from_oid {
                    ui.add_space(4.0);
                    ui.label(styled_muted(&format!("From  {}", &oid[..7.min(oid.len())])));
                }
                ui.add_space(8.0);
                ui.checkbox(&mut self.create_branch_checkout, styled_label("Checkout immediately"));
                ui.add_space(16.0);
                let can = !self.create_branch_name.trim().is_empty();
                ui.horizontal(|ui| {
                    let ok = ui.add_enabled(can, primary_btn("Create Branch")).clicked()
                        || (can && ui.input(|i| i.key_pressed(Key::Enter)));
                    let cancel = ui.add(secondary_btn("Cancel")).clicked()
                        || ui.input(|i| i.key_pressed(Key::Escape));
                    if ok {
                        branches::create_branch(self.create_branch_name.trim(), from_oid.as_deref(), self.create_branch_checkout, self.state.clone());
                        self.state.lock().unwrap().modal = ModalKind::None;
                        self.create_branch_name.clear();
                    } else if cancel {
                        self.state.lock().unwrap().modal = ModalKind::None;
                        self.create_branch_name.clear();
                    }
                });
            }),

            ModalKind::ConfirmDeleteBranch { branch_name } => modal_window(ctx, "Delete Branch", |ui| {
                ui.label(RichText::new(format!("Delete \"{}\"?", branch_name)).color(theme::TEXT_PRIMARY).size(13.0));
                ui.label(styled_muted("Unmerged commits will be lost."));
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    let ok     = ui.add(danger_btn("Delete")).clicked();
                    let cancel = ui.add(secondary_btn("Cancel")).clicked() || ui.input(|i| i.key_pressed(Key::Escape));
                    if ok     { branches::delete_branch(&branch_name, self.state.clone()); self.state.lock().unwrap().modal = ModalKind::None; }
                    if cancel { self.state.lock().unwrap().modal = ModalKind::None; }
                });
            }),

            ModalKind::CreateTag { from_oid } => modal_window(ctx, "Create Tag", |ui| {
                ui.label(styled_label("Tag name"));
                ui.add_space(4.0);
                let r = ui.add(egui::TextEdit::singleline(&mut self.create_tag_name)
                    .desired_width(f32::INFINITY).hint_text("e.g. v1.0.0"));
                r.request_focus();
                if let Some(ref oid) = from_oid {
                    ui.add_space(4.0);
                    ui.label(styled_muted(&format!("At commit  {}", &oid[..7.min(oid.len())])));
                }
                ui.add_space(16.0);
                let can = !self.create_tag_name.trim().is_empty();
                ui.horizontal(|ui| {
                    let ok     = ui.add_enabled(can, primary_btn("Create Tag")).clicked() || (can && ui.input(|i| i.key_pressed(Key::Enter)));
                    let cancel = ui.add(secondary_btn("Cancel")).clicked() || ui.input(|i| i.key_pressed(Key::Escape));
                    if ok {
                        branches::create_tag(self.create_tag_name.trim(), from_oid.as_deref(), self.state.clone());
                        self.state.lock().unwrap().modal = ModalKind::None;
                        self.create_tag_name.clear();
                    }
                    if cancel { self.state.lock().unwrap().modal = ModalKind::None; self.create_tag_name.clear(); }
                });
            }),

            ModalKind::ConfirmDeleteTag { tag_name } => modal_window(ctx, "Delete Tag", |ui| {
                ui.label(RichText::new(format!("Delete tag \"{}\"?", tag_name)).color(theme::TEXT_PRIMARY).size(13.0));
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    let ok     = ui.add(danger_btn("Delete")).clicked();
                    let cancel = ui.add(secondary_btn("Cancel")).clicked() || ui.input(|i| i.key_pressed(Key::Escape));
                    if ok     { branches::delete_tag(&tag_name, self.state.clone()); self.state.lock().unwrap().modal = ModalKind::None; }
                    if cancel { self.state.lock().unwrap().modal = ModalKind::None; }
                });
            }),
        }

        // ── Command palette ───────────────────────────────────────────────
        if self.command_palette_open {
            self.render_command_palette(ctx);
        }

        // ── Branch switcher ───────────────────────────────────────────────
        if self.branch_switcher_open {
            self.render_branch_switcher(ctx);
        }

        // ── Settings ─────────────────────────────────────────────────────
        if self.settings_open {
            self.render_settings(ctx);
        }
    }

    fn render_command_palette(&mut self, ctx: &Context) {
        let actions: &[(&str, &str)] = &[
            ("↓  Fetch", "Ctrl+Shift+F"),
            ("⇅  Pull", "Ctrl+Shift+P"),
            ("↑  Push", "Ctrl+Shift+U"),
            ("⎇  New Branch", ""),
            ("⎇  Switch Branch…", "Ctrl+B"),
            ("⚑  Stash Changes", ""),
            ("🔍  Search Commits", "Ctrl+F"),
            ("⊞  Open Repository", ""),
            ("☀  Toggle Theme", ""),
        ];

        egui::Window::new("Command Palette")
            .collapsible(false)
            .resizable(false)
            .title_bar(false)
            .anchor(egui::Align2::CENTER_TOP, [0.0, 60.0])
            .frame(egui::Frame::none()
                .fill(theme::BG_PANEL)
                .stroke(Stroke::new(1.0, theme::BORDER_STRONG))
                .rounding(10.0)
                .inner_margin(egui::Margin::same(0.0)))
            .show(ctx, |ui| {
                ui.set_min_width(480.0);

                // Search input
                let input_bg = egui::Frame::none()
                    .fill(theme::BG_PANEL_ALT)
                    .inner_margin(egui::Margin::symmetric(12.0, 10.0));
                input_bg.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("⌘").color(theme::TEXT_MUTED).size(15.0));
                        ui.add_space(6.0);
                        let r = ui.add(
                            egui::TextEdit::singleline(&mut self.command_palette_query)
                                .desired_width(f32::INFINITY)
                                .hint_text("Type a command…")
                                .font(egui::FontId::proportional(14.0)),
                        );
                        r.request_focus();
                    });
                });

                ui.add(egui::Separator::default().spacing(0.0));

                let q = self.command_palette_query.to_lowercase();
                let mut selected: Option<usize> = None;

                for (i, (label, shortcut)) in actions.iter().enumerate() {
                    let lq = label.to_lowercase();
                    if !q.is_empty() && !lq.contains(&q) { continue; }

                    let row = ui.horizontal(|ui| {
                        ui.add_space(14.0);
                        ui.label(RichText::new(*label).color(theme::TEXT_PRIMARY).size(13.0));
                        if !shortcut.is_empty() {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.add_space(14.0);
                                ui.label(RichText::new(*shortcut).color(theme::TEXT_MUTED).size(11.0).monospace());
                            });
                        }
                    }).response;

                    let row_resp = row.interact(egui::Sense::click());
                    if row_resp.hovered() {
                        ui.painter().rect_filled(row_resp.rect, 0.0, theme::BG_HOVER);
                    }
                    if row_resp.clicked() { selected = Some(i); }
                }

                ui.add_space(6.0);

                if ui.input(|i| i.key_pressed(Key::Escape)) || ctx.input(|i| i.key_pressed(Key::Escape)) {
                    self.command_palette_open = false;
                    self.command_palette_query.clear();
                }

                if let Some(i) = selected {
                    self.command_palette_open = false;
                    self.command_palette_query.clear();
                    match i {
                        0 => remote::fetch_all(self.state.clone()),
                        1 => remote::pull_branch(self.state.clone()),
                        2 => remote::push_branch(self.state.clone()),
                        3 => self.state.lock().unwrap().modal = ModalKind::CreateBranch { from_oid: None },
                        4 => self.branch_switcher_open = true,
                        5 => branches::stash_save(self.state.clone()),
                        6 => { let mut s = self.state.lock().unwrap(); s.search_active = true; }
                        7 => { if let Some(p) = rfd::FileDialog::new().pick_folder() { self.config.add_recent(p.clone()); load_repo(p, self.state.clone()); } }
                        8 => { self.dark_theme = !self.dark_theme; self.config.dark_theme = self.dark_theme; self.config.save(); }
                        _ => {}
                    }
                }
            });
    }

    fn render_branch_switcher(&mut self, ctx: &Context) {
        let branches_list: Vec<String> = {
            let s = self.state.lock().unwrap();
            s.commit_graph.as_ref()
                .map(|g| {
                    let mut refs: Vec<&str> = g.nodes.iter()
                        .flat_map(|n| n.refs.iter().map(|r| r.as_str()))
                        .collect();
                    refs.sort_unstable(); refs.dedup();
                    refs.iter().filter(|r| !r.contains('/'))
                        .map(|s| s.to_string()).collect()
                })
                .unwrap_or_default()
        };

        let mut close = false;
        let mut checkout: Option<String> = None;

        egui::Window::new("Switch Branch")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(egui::Frame::window(&ctx.style()).inner_margin(egui::Margin::same(16.0)))
            .show(ctx, |ui| {
                ui.set_min_width(340.0);
                ui.label(RichText::new("Switch Branch").color(theme::TEXT_PRIMARY).size(15.0).strong());
                ui.add_space(10.0);
                let r = ui.add(
                    egui::TextEdit::singleline(&mut self.branch_switcher_query)
                        .desired_width(f32::INFINITY)
                        .hint_text("Filter branches…"),
                );
                r.request_focus();
                ui.add_space(6.0);

                let q = self.branch_switcher_query.to_lowercase();
                egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                    for name in branches_list.iter().filter(|n| q.is_empty() || n.to_lowercase().contains(&q)) {
                        let r = ui.selectable_label(false, RichText::new(name).color(theme::TEXT_SECONDARY).size(12.5));
                        if r.clicked() { checkout = Some(name.clone()); }
                    }
                });

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if let Some(ref name) = checkout {
                        let _ = name; // handled below
                    }
                    if ui.add(secondary_btn("Cancel")).clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                        close = true;
                    }
                });
            });

        if let Some(name) = checkout {
            branches::checkout_branch(&name, self.state.clone());
            self.branch_switcher_open = false;
            self.branch_switcher_query.clear();
        }
        if close {
            self.branch_switcher_open = false;
            self.branch_switcher_query.clear();
        }
    }

    fn render_settings(&mut self, ctx: &Context) {
        egui::Window::new("Settings")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(egui::Frame::window(&ctx.style()).inner_margin(egui::Margin::same(16.0)))
            .show(ctx, |ui| {
                ui.set_min_width(300.0);
                ui.label(RichText::new("Settings").color(theme::TEXT_PRIMARY).size(15.0).strong());
                ui.add_space(12.0);
                if ui.checkbox(&mut self.dark_theme, RichText::new("Dark theme").color(theme::TEXT_SECONDARY)).changed() {
                    self.config.dark_theme = self.dark_theme;
                    self.config.save();
                    theme::apply(ctx, self.dark_theme);
                }
                ui.add_space(8.0);
                ui.label(styled_muted("Split diff view — coming in a future update"));
                ui.add_space(16.0);
                if ui.add(secondary_btn("Close")).clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                    self.settings_open = false;
                }
            });
    }
}

// ── eframe::App ───────────────────────────────────────────────────────────────

impl eframe::App for RecordeApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        // Kill egui debug overlays every frame
        ctx.style_mut(theme::kill_debug);
        ctx.options_mut(|o| o.warn_on_id_clash = false);

        // ── Global keyboard shortcuts ─────────────────────────────────────
        if ctx.input(|i| i.modifiers == Modifiers::COMMAND && i.key_pressed(Key::K)) {
            self.command_palette_open = !self.command_palette_open;
        }
        if ctx.input(|i| i.modifiers == Modifiers::COMMAND && i.key_pressed(Key::B)) {
            self.branch_switcher_open = !self.branch_switcher_open;
        }
        if ctx.input(|i| i.modifiers.ctrl && i.modifiers.shift && i.key_pressed(Key::F)) {
            remote::fetch_all(self.state.clone());
        }
        if ctx.input(|i| i.modifiers.ctrl && i.modifiers.shift && i.key_pressed(Key::P)) {
            remote::pull_branch(self.state.clone());
        }
        if ctx.input(|i| i.modifiers.ctrl && i.modifiers.shift && i.key_pressed(Key::U)) {
            remote::push_branch(self.state.clone());
        }

        // ── Pending actions from graph context menu ───────────────────────
        let pending_rebase = self.state.lock().unwrap().pending_open_rebase.take();
        if let Some(oid) = pending_rebase {
            self.open_rebase_modal(oid);
        }

        // ── Modals ────────────────────────────────────────────────────────
        self.render_modals(ctx);

        // Interactive rebase window
        if self.rebase_modal.is_some() {
            let mut close = false;
            let mut execute: Option<(String, Vec<RebaseEntry>)> = None;
            egui::Window::new("Interactive Rebase")
                .collapsible(false)
                .resizable(true)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .frame(egui::Frame::window(&ctx.style()).inner_margin(egui::Margin::same(16.0)))
                .show(ctx, |ui| {
                    if let Some(ref mut m) = self.rebase_modal {
                        match m.show(ui) {
                            Some(RebaseModalResult::Execute) => { execute = Some((m.upstream_oid.clone(), m.entries.clone())); close = true; }
                            Some(RebaseModalResult::Cancel)  => close = true,
                            None => {}
                        }
                    }
                });
            if close { self.rebase_modal = None; }
            if let Some((oid, entries)) = execute {
                rebase::start_rebase(&oid, entries, self.state.clone());
            }
        }

        // ── Panels ────────────────────────────────────────────────────────
        self.status_bar.show(ctx);

        // Title bar
        egui::TopBottomPanel::top("title_bar")
            .exact_height(36.0)
            .frame(egui::Frame::none().fill(theme::BG_TITLEBAR)
                .inner_margin(egui::Margin::symmetric(16.0, 0.0)))
            .show(ctx, |ui| {
                let r = ui.max_rect();
                ui.painter().line_segment(
                    [r.left_bottom(), r.right_bottom()],
                    Stroke::new(1.0, theme::BORDER),
                );
                ui.horizontal_centered(|ui| {
                    // Traffic-light dots (decorative)
                    for col in [Color32::from_rgb(255,95,86), Color32::from_rgb(255,189,46), Color32::from_rgb(39,201,63)] {
                        let (r, _) = ui.allocate_exact_size(egui::Vec2::splat(12.0), egui::Sense::hover());
                        ui.painter().circle_filled(r.center(), 6.0, col);
                    }
                    ui.add_space(12.0);
                    // Repo name
                    let repo_name = self.state.lock().unwrap().repo_path.as_ref()
                        .and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "No repository".into());
                    ui.label(RichText::new(repo_name).color(theme::TEXT_SECONDARY).size(13.0));

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(16.0);
                        ui.label(RichText::new("Recorde").color(theme::TEXT_MUTED).size(12.5));
                    });
                });
            });

        // Toolbar
        let toolbar_action = self.toolbar.show(
            ctx,
            &self.config.recent_repos,
            self.dark_theme,
            self.state.lock().unwrap().search_active,
        );
        if let Some(a) = toolbar_action { self.handle_toolbar(ctx, a); }

        // Bottom diff panel (always visible)
        egui::TopBottomPanel::bottom("diff_panel")
            .resizable(true)
            .default_height(300.0)
            .height_range(160.0..=480.0)
            .frame(egui::Frame::none().fill(theme::BG_APP))
            .show(ctx, |ui| {
                let r = ui.max_rect();
                ui.painter().line_segment(
                    [r.left_top(), r.right_top()],
                    Stroke::new(1.0, theme::BORDER),
                );
                self.diff_view.show(ui);
            });

        // Left sidebar
        egui::SidePanel::left("sidebar")
            .resizable(true)
            .default_width(250.0)
            .width_range(200.0..=340.0)
            .frame(egui::Frame::none())
            .show(ctx, |ui| {
                let a = self.sidebar.show(ui);
                if let Some(sa) = a { self.handle_sidebar(sa); }
            });

        // Right panel (Changes + Details)
        egui::SidePanel::right("right_panel")
            .resizable(true)
            .default_width(430.0)
            .width_range(340.0..=560.0)
            .frame(egui::Frame::none())
            .show(ctx, |ui| {
                let has_commit = self.state.lock().unwrap().selected_commit.is_some();
                let has_dirty  = self.state.lock().unwrap().dirty_count > 0;

                // Tab bar
                let tab_h = 38.0;
                let tab_rect = egui::Rect::from_min_size(
                    ui.min_rect().min,
                    egui::Vec2::new(ui.available_width(), tab_h),
                );
                ui.painter().rect_filled(tab_rect, 0.0, theme::BG_PANEL);
                ui.painter().line_segment(
                    [tab_rect.left_bottom(), tab_rect.right_bottom()],
                    Stroke::new(1.0, theme::BORDER),
                );

                ui.allocate_ui_at_rect(tab_rect, |ui| {
                    ui.horizontal_centered(|ui| {
                        ui.add_space(12.0);
                        for (tab, lbl) in &[
                            (RightTab::Changes, if has_dirty { "Changes  ●" } else { "Changes" }),
                            (RightTab::Details, "Details"),
                        ] {
                            let sel = self.right_tab == *tab;
                            let text_col = if sel { theme::TEXT_PRIMARY } else { theme::TEXT_MUTED };
                            let btn = ui.add(
                                egui::Button::new(RichText::new(*lbl).size(13.0).color(text_col))
                                    .fill(Color32::TRANSPARENT)
                                    .stroke(Stroke::NONE),
                            );
                            // Teal underline for active tab (matches mockup)
                            if sel {
                                let br = btn.rect;
                                ui.painter().line_segment(
                                    [br.left_bottom(), br.right_bottom()],
                                    Stroke::new(2.0, theme::ACCENT_TEAL),
                                );
                            }
                            if btn.clicked() { self.right_tab = *tab; }
                            ui.add_space(16.0);
                        }
                        // Auto-switch to details when commit selected
                        if has_commit && !has_dirty {
                            self.right_tab = RightTab::Details;
                        }
                    });
                });

                let content_rect = egui::Rect::from_min_max(
                    egui::Pos2::new(ui.min_rect().min.x, ui.min_rect().min.y + tab_h),
                    ui.max_rect().max,
                );
                ui.allocate_ui_at_rect(content_rect, |ui| {
                    match self.right_tab {
                        RightTab::Changes => self.staging_panel.show(ui),
                        RightTab::Details => self.detail_panel.show(ui),
                    }
                });
            });

        // Central panel — commit graph (always)
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(theme::BG_APP))
            .show(ctx, |ui| {
                self.commit_graph_view.show(ui);
            });
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn modal_window(ctx: &Context, title: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
    egui::Window::new(title)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .frame(egui::Frame::window(&ctx.style()).inner_margin(egui::Margin::same(16.0)))
        .show(ctx, |ui| {
            ui.set_min_width(300.0);
            ui.add_space(4.0);
            add_contents(ui);
            ui.add_space(4.0);
        });
}

fn primary_btn(label: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(label).color(Color32::WHITE).size(13.0))
        .fill(theme::ACCENT_BLUE).rounding(6.0)
        .min_size(egui::Vec2::new(120.0, 28.0))
}

fn danger_btn(label: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(label).color(Color32::WHITE))
        .fill(theme::ACCENT_RED).rounding(6.0)
}

fn secondary_btn(label: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(label).color(theme::TEXT_SECONDARY))
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::new(1.0, theme::BORDER))
        .rounding(6.0)
}

fn styled_label(text: &str) -> RichText {
    RichText::new(text).color(theme::TEXT_SECONDARY).size(12.0)
}

fn styled_muted(text: &str) -> RichText {
    RichText::new(text).color(theme::TEXT_MUTED).size(11.5)
}
