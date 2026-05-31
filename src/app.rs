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

#[derive(PartialEq, Clone, Copy)]
enum SettingsTab { App, GitCredentials, Repository, About }

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
    settings_tab: SettingsTab,
    git_name_input: String,
    git_email_input: String,
    repo_git_name_input: String,
    repo_git_email_input: String,
    default_editor_input: String,
    pull_requests_open: bool,
    // Interactive rebase
    rebase_modal: Option<RebaseModal>,
}

impl RecordeApp {
    pub fn new(cc: &eframe::CreationContext<'_>, initial_repo: Option<std::path::PathBuf>) -> Self {
        let config = Config::load();
        let dark_theme = config.dark_theme;
        let default_editor = config.default_editor.clone();

        // Load Noto Symbols as egui fallback font for full Unicode symbol coverage.
        // This fixes missing-glyph squares for ⎇ ⚑ ⊞ ⇅ ☀ ⌘ and similar.
        let mut fonts = egui::FontDefinitions::default();
        const NOTO_SYMBOLS2: &str =
            "/usr/share/fonts/truetype/noto/NotoSansSymbols2-Regular.ttf";
        if let Ok(bytes) = std::fs::read(NOTO_SYMBOLS2) {
            fonts.font_data.insert("noto_symbols2".into(), egui::FontData::from_owned(bytes));
            fonts.families.entry(egui::FontFamily::Proportional).or_default().push("noto_symbols2".into());
            fonts.families.entry(egui::FontFamily::Monospace).or_default().push("noto_symbols2".into());
        }
        // Material Icons — named font family for icon rendering
        if let Ok(bytes) = std::fs::read("assets/MaterialIcons.ttf") {
            fonts.font_data.insert("material-icons".into(), egui::FontData::from_owned(bytes));
            fonts.families.insert(
                egui::FontFamily::Name("material-icons".into()),
                vec!["material-icons".into()],
            );
        }
        cc.egui_ctx.set_fonts(fonts);

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
            settings_tab: SettingsTab::App,
            git_name_input: String::new(),
            git_email_input: String::new(),
            repo_git_name_input: String::new(),
            repo_git_email_input: String::new(),
            default_editor_input: default_editor,
            pull_requests_open: false,
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
            ToolbarAction::OpenCommandPalette => self.command_palette_open = !self.command_palette_open,
            ToolbarAction::OpenBranchSwitcher => self.branch_switcher_open = true,
            ToolbarAction::OpenUserMenu => self.open_settings(SettingsTab::GitCredentials),
            ToolbarAction::ToggleAllBranches => {
                let mut s = self.state.lock().unwrap();
                s.show_all_branches = !s.show_all_branches;
            }
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

    fn open_settings(&mut self, tab: SettingsTab) {
        self.settings_tab = tab;
        self.settings_open = true;
        self.load_git_identity_inputs();
    }

    fn load_git_identity_inputs(&mut self) {
        let global = git_identity(None);
        self.git_name_input = global.0;
        self.git_email_input = global.1;

        let repo_path = self.state.lock().unwrap().repo_path.clone();
        let repo = git_identity(repo_path.as_deref());
        self.repo_git_name_input = repo.0;
        self.repo_git_email_input = repo.1;
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

        if self.pull_requests_open {
            self.render_pull_requests(ctx);
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
            ("⌕  Search Commits", "Ctrl+F"),
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
                        8 => {
                            self.dark_theme = !self.dark_theme;
                            self.config.dark_theme = self.dark_theme;
                            self.config.save();
                            theme::apply(ctx, self.dark_theme);
                        }
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
            .resizable(true)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(egui::Frame::window(&ctx.style()).inner_margin(egui::Margin::same(16.0)))
            .show(ctx, |ui| {
                ui.set_min_size(egui::vec2(520.0, 360.0));
                ui.label(RichText::new("Settings").color(theme::TEXT_PRIMARY).size(15.0).strong());
                ui.add_space(12.0);

                ui.horizontal(|ui| {
                    settings_tab_button(ui, &mut self.settings_tab, SettingsTab::App, "App");
                    settings_tab_button(ui, &mut self.settings_tab, SettingsTab::GitCredentials, "Git Credentials");
                    settings_tab_button(ui, &mut self.settings_tab, SettingsTab::Repository, "Repository");
                    settings_tab_button(ui, &mut self.settings_tab, SettingsTab::About, "About");
                });
                ui.add(egui::Separator::default().spacing(0.0));
                ui.add_space(8.0);

                match self.settings_tab {
                    SettingsTab::App => self.render_app_settings(ctx, ui),
                    SettingsTab::GitCredentials => self.render_git_settings(ui),
                    SettingsTab::Repository => self.render_repo_settings(ui),
                    SettingsTab::About => self.render_about(ui),
                }

                ui.add_space(16.0);
                if ui.add(secondary_btn("Close")).clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                    self.settings_open = false;
                }
            });
    }

    fn render_app_settings(&mut self, ctx: &Context, ui: &mut egui::Ui) {
        if ui.checkbox(&mut self.dark_theme, RichText::new("Dark theme").color(theme::TEXT_SECONDARY)).changed() {
            self.config.dark_theme = self.dark_theme;
            self.config.save();
            theme::apply(ctx, self.dark_theme);
        }
        ui.add_space(8.0);
        ui.label(RichText::new("Default editor").color(theme::TEXT_SECONDARY).size(12.5).strong());
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.default_editor_input)
                .desired_width(340.0)
                .hint_text("code, codium, zed, vim, ..."));
            if ui.add(secondary_btn("Save")).clicked() {
                self.config.default_editor = self.default_editor_input.trim().to_string();
                self.config.save();
            }
            if ui.add(secondary_btn("Open Repo")).clicked() {
                if let Err(e) = open_repo_in_editor(self.state.lock().unwrap().repo_path.clone(), &self.config.default_editor) {
                    self.state.lock().unwrap().error = Some(e);
                }
            }
        });
        ui.label(styled_muted("Used by Repository > Open in Editor. Arguments are supported, e.g. code --reuse-window."));
        ui.add_space(8.0);
        ui.label(styled_muted("Split diff view is controlled from the diff header."));
    }

    fn render_git_settings(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Global Git identity").color(theme::TEXT_SECONDARY).size(12.5).strong());
        ui.add_space(6.0);
        settings_text_field(ui, "Name", &mut self.git_name_input);
        settings_text_field(ui, "Email", &mut self.git_email_input);
        ui.add_space(6.0);
        if ui.add(primary_btn("Save Global Identity")).clicked() {
            match save_git_identity(None, &self.git_name_input, &self.git_email_input) {
                Ok(()) => self.state.lock().unwrap().error = None,
                Err(e) => self.state.lock().unwrap().error = Some(e),
            }
        }
        ui.add_space(14.0);
        ui.label(RichText::new("Credential helper").color(theme::TEXT_SECONDARY).size(12.5).strong());
        ui.label(styled_muted(&git_config_value(None, "credential.helper").unwrap_or_else(|| "Not configured".into())));
        ui.add_space(6.0);
        ui.label(styled_muted("Recorde uses Git's credential helper and SSH agent/key files. It does not store passwords."));
        ui.add_space(10.0);
        ui.label(RichText::new("SSH keys").color(theme::TEXT_SECONDARY).size(12.5).strong());
        for line in ssh_key_status() {
            ui.label(styled_muted(&line));
        }
    }

    fn render_repo_settings(&mut self, ui: &mut egui::Ui) {
        let repo_path = self.state.lock().unwrap().repo_path.clone();
        let Some(path) = repo_path else {
            ui.label(styled_muted("Open a repository to edit repository settings."));
            return;
        };

        ui.label(RichText::new("Repository").color(theme::TEXT_SECONDARY).size(12.5).strong());
        ui.label(styled_muted(&path.display().to_string()));
        ui.add_space(10.0);
        ui.label(RichText::new("Repository Git identity").color(theme::TEXT_SECONDARY).size(12.5).strong());
        settings_text_field(ui, "Name", &mut self.repo_git_name_input);
        settings_text_field(ui, "Email", &mut self.repo_git_email_input);
        ui.add_space(6.0);
        if ui.add(primary_btn("Save Repository Identity")).clicked() {
            match save_git_identity(Some(&path), &self.repo_git_name_input, &self.repo_git_email_input) {
                Ok(()) => self.state.lock().unwrap().error = None,
                Err(e) => self.state.lock().unwrap().error = Some(e),
            }
        }
        ui.add_space(14.0);
        ui.label(RichText::new("Remotes").color(theme::TEXT_SECONDARY).size(12.5).strong());
        for remote in repo_remotes(&path) {
            ui.label(styled_muted(&remote));
        }
    }

    fn render_about(&self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Recorde").color(theme::TEXT_PRIMARY).size(14.0).strong());
        ui.label(styled_muted("A desktop Git client built with Rust, egui, and eframe."));
        ui.add_space(8.0);
        ui.label(styled_muted("Keyboard: Ctrl+K command palette, Ctrl+B branch switcher, Ctrl+F search."));
    }

    fn render_pull_requests(&mut self, ctx: &Context) {
        let (repo_path, branch) = {
            let s = self.state.lock().unwrap();
            (s.repo_path.clone(), s.head_branch.clone().unwrap_or_else(|| "HEAD".into()))
        };
        let pr_target = repo_path.as_deref().and_then(|path| pull_request_target(path, &branch));

        egui::Window::new("Pull Requests")
            .collapsible(false)
            .resizable(true)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(egui::Frame::window(&ctx.style()).inner_margin(egui::Margin::same(16.0)))
            .show(ctx, |ui| {
                ui.set_min_size(egui::vec2(460.0, 220.0));
                ui.label(RichText::new("Pull Requests").color(theme::TEXT_PRIMARY).size(15.0).strong());
                ui.add_space(8.0);

                match pr_target {
                    Some((ref remote_url, ref pr_url)) => {
                        ui.label(styled_muted(&format!("Remote: {remote_url}")));
                        ui.label(styled_muted(&format!("Current branch: {branch}")));
                        ui.add_space(10.0);
                        ui.label(RichText::new("Preview").color(theme::TEXT_SECONDARY).size(12.5).strong());
                        ui.label(styled_muted("Recorde can open the hosting provider's compare / pull request page for this branch."));
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if ui.add(primary_btn("Open Pull Request Page")).clicked() {
                                if let Err(e) = open_url(pr_url) {
                                    self.state.lock().unwrap().error = Some(e);
                                }
                            }
                            if ui.add(secondary_btn("Close")).clicked() {
                                self.pull_requests_open = false;
                            }
                        });
                    }
                    None => {
                        ui.label(styled_muted("No GitHub-style remote URL was found for this repository."));
                        ui.label(styled_muted("Add an origin remote or open the pull request page in your browser."));
                        ui.add_space(12.0);
                        if ui.add(secondary_btn("Close")).clicked() {
                            self.pull_requests_open = false;
                        }
                    }
                }
            });
    }
}

// ── eframe::App ───────────────────────────────────────────────────────────────

impl eframe::App for RecordeApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
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
        // Ctrl+1/2/3/4: focus sidebar / graph / changes / diff
        if ctx.input(|i| i.modifiers.ctrl && !i.modifiers.shift && i.key_pressed(Key::Num1)) {
            // Focus sidebar — nothing to scroll to, just a visual hint via state
            self.state.lock().unwrap().focused_pane = 1;
        }
        if ctx.input(|i| i.modifiers.ctrl && !i.modifiers.shift && i.key_pressed(Key::Num2)) {
            self.state.lock().unwrap().focused_pane = 2;
        }
        if ctx.input(|i| i.modifiers.ctrl && !i.modifiers.shift && i.key_pressed(Key::Num3)) {
            self.right_tab = RightTab::Changes;
            self.state.lock().unwrap().focused_pane = 3;
        }
        if ctx.input(|i| i.modifiers.ctrl && !i.modifiers.shift && i.key_pressed(Key::Num4)) {
            self.state.lock().unwrap().focused_pane = 4;
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
                    self.show_menu_contents(ctx, ui);

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(16.0);
                        ui.label(RichText::new("Recorde").color(theme::TEXT_MUTED).size(12.5));
                    });
                });
            });

        // Toolbar
        let (search_active, show_all_branches) = {
            let s = self.state.lock().unwrap();
            (s.search_active, s.show_all_branches)
        };
        let toolbar_action = self.toolbar.show(
            ctx,
            &self.config.recent_repos,
            self.dark_theme,
            search_active,
            show_all_branches,
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
                if self.state.lock().unwrap().focused_pane == 4 {
                    focus_ring(ui);
                }
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
                if self.state.lock().unwrap().focused_pane == 1 {
                    focus_ring(ui);
                }
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
                if self.state.lock().unwrap().focused_pane == 3 {
                    focus_ring(ui);
                }
            });

        // Central panel — commit graph (always)
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(theme::BG_APP))
            .show(ctx, |ui| {
                self.commit_graph_view.show(ui);
                if self.state.lock().unwrap().focused_pane == 2 {
                    focus_ring(ui);
                }
            });

        self.show_operation_overlays(ctx);
    }
}

impl RecordeApp {
    fn show_operation_overlays(&mut self, ctx: &Context) {
        let (op, error) = {
            let s = self.state.lock().unwrap();
            (s.remote_op.clone(), s.error.clone())
        };

        if let Some(op) = op {
            ctx.request_repaint_after(std::time::Duration::from_millis(120));
            egui::Area::new(egui::Id::new("operation_overlay"))
                .anchor(egui::Align2::CENTER_TOP, [0.0, 104.0])
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    egui::Frame::none()
                        .fill(theme::BG_ELEVATED)
                        .stroke(Stroke::new(1.0, theme::ACCENT_TEAL))
                        .rounding(8.0)
                        .inner_margin(egui::Margin::symmetric(14.0, 10.0))
                        .show(ui, |ui| {
                            ui.horizontal_centered(|ui| {
                                ui.spinner();
                                ui.add_space(8.0);
                                ui.label(RichText::new(op).color(theme::TEXT_PRIMARY).size(13.0).strong());
                            });
                        });
                });
        }

        if let Some(error) = error {
            egui::Area::new(egui::Id::new("error_overlay"))
                .anchor(egui::Align2::CENTER_TOP, [0.0, 146.0])
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    egui::Frame::none()
                        .fill(Color32::from_rgb(74, 24, 28))
                        .stroke(Stroke::new(1.5, theme::ACCENT_RED))
                        .rounding(8.0)
                        .inner_margin(egui::Margin::symmetric(16.0, 12.0))
                        .show(ui, |ui| {
                            ui.set_max_width(720.0);
                            ui.horizontal_centered(|ui| {
                                ui.label(RichText::new("Operation blocked").color(Color32::WHITE).size(13.5).strong());
                                ui.add_space(10.0);
                                ui.label(RichText::new(error).color(Color32::WHITE).size(12.5));
                                ui.add_space(10.0);
                                if ui.add(secondary_btn("Dismiss")).clicked() {
                                    self.state.lock().unwrap().error = None;
                                }
                            });
                        });
                });
        }
    }

    fn show_menu_contents(&mut self, ctx: &Context, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        egui::menu::bar(ui, |ui| {
                    ui.menu_button("File", |ui| {
                        if ui.button("Open Repository...").clicked() {
                            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                                self.config.add_recent(path.clone());
                                load_repo(path, self.state.clone());
                            }
                            ui.close_menu();
                        }
                        if !self.config.recent_repos.is_empty() {
                            ui.menu_button("Recent Repositories", |ui| {
                                for path in &self.config.recent_repos {
                                    let label = path.file_name()
                                        .map(|n| n.to_string_lossy().into_owned())
                                        .unwrap_or_else(|| path.display().to_string());
                                    if ui.button(label).clicked() {
                                        load_repo(path.clone(), self.state.clone());
                                        ui.close_menu();
                                    }
                                }
                            });
                        }
                        ui.separator();
                        if ui.button("Settings").clicked() {
                            self.open_settings(SettingsTab::App);
                            ui.close_menu();
                        }
                        if ui.button("Quit").clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            ui.close_menu();
                        }
                    });

                    ui.menu_button("Edit", |ui| {
                        if ui.button("Copy Selected Commit SHA").clicked() {
                            if let Some(oid) = self.state.lock().unwrap().selected_commit.clone() {
                                ui.output_mut(|o| o.copied_text = oid);
                            }
                            ui.close_menu();
                        }
                        if ui.button("Find Commits").clicked() {
                            self.state.lock().unwrap().search_active = true;
                            ui.close_menu();
                        }
                        if ui.button("Clear Selection").clicked() {
                            let mut s = self.state.lock().unwrap();
                            s.selected_commit = None;
                            s.selected_commit_detail = None;
                            s.selected_file = None;
                            s.current_diff = None;
                            ui.close_menu();
                        }
                    });

                    ui.menu_button("View", |ui| {
                        let show_all = self.state.lock().unwrap().show_all_branches;
                        if ui.button(if show_all { "Show Current Branch Only" } else { "Show All Branches" }).clicked() {
                            self.state.lock().unwrap().show_all_branches = !show_all;
                            ui.close_menu();
                        }
                        if ui.button("Toggle Theme").clicked() {
                            self.handle_toolbar(ctx, ToolbarAction::ToggleTheme);
                            ui.close_menu();
                        }
                        ui.separator();
                        for (label, pane) in [("Focus Sidebar", 1), ("Focus Commit Graph", 2), ("Focus Changes", 3), ("Focus Diff", 4)] {
                            if ui.button(label).clicked() {
                                self.state.lock().unwrap().focused_pane = pane;
                                ui.close_menu();
                            }
                        }
                    });

                    ui.menu_button("Repository", |ui| {
                        let has_repo = self.state.lock().unwrap().repo_path.is_some();
                        if ui.add_enabled(has_repo, egui::Button::new("Open in Editor")).clicked() {
                            if let Err(e) = open_repo_in_editor(self.state.lock().unwrap().repo_path.clone(), &self.config.default_editor) {
                                self.state.lock().unwrap().error = Some(e);
                            }
                            ui.close_menu();
                        }
                        if ui.add_enabled(has_repo, egui::Button::new("Pull Requests...")).clicked() {
                            self.pull_requests_open = true;
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.add_enabled(has_repo, egui::Button::new("Fetch")).clicked() {
                            remote::fetch_all(self.state.clone());
                            ui.close_menu();
                        }
                        if ui.add_enabled(has_repo, egui::Button::new("Pull")).clicked() {
                            remote::pull_branch(self.state.clone());
                            ui.close_menu();
                        }
                        if ui.add_enabled(has_repo, egui::Button::new("Push")).clicked() {
                            remote::push_branch(self.state.clone());
                            ui.close_menu();
                        }
                        if ui.add_enabled(has_repo, egui::Button::new("Refresh")).clicked() {
                            if let Some(path) = self.state.lock().unwrap().repo_path.clone() {
                                load_repo(path, self.state.clone());
                            }
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.add_enabled(has_repo, egui::Button::new("Repository Settings")).clicked() {
                            self.open_settings(SettingsTab::Repository);
                            ui.close_menu();
                        }
                    });

                    ui.menu_button("Branch", |ui| {
                        let has_repo = self.state.lock().unwrap().repo_path.is_some();
                        if ui.add_enabled(has_repo, egui::Button::new("Switch Branch...")).clicked() {
                            self.branch_switcher_open = true;
                            ui.close_menu();
                        }
                        if ui.add_enabled(has_repo, egui::Button::new("New Branch...")).clicked() {
                            self.state.lock().unwrap().modal = ModalKind::CreateBranch { from_oid: None };
                            ui.close_menu();
                        }
                        if ui.add_enabled(has_repo, egui::Button::new("Stash Changes")).clicked() {
                            branches::stash_save(self.state.clone());
                            ui.close_menu();
                        }
                    });

                    ui.menu_button("Help", |ui| {
                        if ui.button("About Recorde").clicked() {
                            self.open_settings(SettingsTab::About);
                            ui.close_menu();
                        }
                    });
                });
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Draw a subtle teal focus ring around the panel to indicate keyboard focus.
fn focus_ring(ui: &egui::Ui) {
    ui.painter().rect_stroke(
        ui.max_rect().shrink(1.0),
        0.0,
        egui::Stroke::new(1.5, theme::ACCENT_TEAL.gamma_multiply(0.5)),
    );
}

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
        .min_size(egui::Vec2::new(0.0, 28.0))
}

fn secondary_btn(label: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(label).color(theme::TEXT_SECONDARY))
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::new(1.0, theme::BORDER))
        .rounding(6.0)
        .min_size(egui::Vec2::new(0.0, 28.0))
}

fn styled_label(text: &str) -> RichText {
    RichText::new(text).color(theme::TEXT_SECONDARY).size(12.0)
}

fn styled_muted(text: &str) -> RichText {
    RichText::new(text).color(theme::TEXT_MUTED).size(11.5)
}

fn settings_tab_button(ui: &mut egui::Ui, tab: &mut SettingsTab, value: SettingsTab, label: &str) {
    let selected = *tab == value;
    let color = if selected { theme::TEXT_PRIMARY } else { theme::TEXT_MUTED };
    if ui.add(egui::Button::new(RichText::new(label).color(color).size(12.0))
        .fill(if selected { theme::BG_SELECTED } else { Color32::TRANSPARENT })
        .stroke(Stroke::new(1.0, if selected { theme::BORDER_STRONG } else { theme::BORDER }))
        .rounding(6.0)
        .min_size(egui::Vec2::new(0.0, 28.0))).clicked()
    {
        *tab = value;
    }
}

fn settings_text_field(ui: &mut egui::Ui, label: &str, value: &mut String) {
    ui.horizontal(|ui| {
        ui.set_width(460.0);
        ui.label(RichText::new(label).color(theme::TEXT_MUTED).font(egui::FontId::monospace(11.0)));
        ui.add_space(8.0);
        ui.add(egui::TextEdit::singleline(value).desired_width(340.0));
    });
}

fn git_identity(repo_path: Option<&std::path::Path>) -> (String, String) {
    let config = match repo_path {
        Some(path) => git2::Repository::open(path).ok().and_then(|r| r.config().ok()),
        None => git2::Config::open_default().ok(),
    };
    let Some(config) = config else { return (String::new(), String::new()); };
    (
        config.get_string("user.name").unwrap_or_default(),
        config.get_string("user.email").unwrap_or_default(),
    )
}

fn save_git_identity(repo_path: Option<&std::path::Path>, name: &str, email: &str) -> Result<(), String> {
    let mut config = match repo_path {
        Some(path) => git2::Repository::open(path)
            .map_err(|e| e.message().to_string())?
            .config()
            .map_err(|e| e.message().to_string())?,
        None => git2::Config::open_default().map_err(|e| e.message().to_string())?,
    };
    if !name.trim().is_empty() {
        config.set_str("user.name", name.trim()).map_err(|e| e.message().to_string())?;
    }
    if !email.trim().is_empty() {
        config.set_str("user.email", email.trim()).map_err(|e| e.message().to_string())?;
    }
    Ok(())
}

fn git_config_value(repo_path: Option<&std::path::Path>, key: &str) -> Option<String> {
    match repo_path {
        Some(path) => git2::Repository::open(path).ok()?.config().ok()?.get_string(key).ok(),
        None => git2::Config::open_default().ok()?.get_string(key).ok(),
    }
}

fn repo_remotes(path: &std::path::Path) -> Vec<String> {
    let Ok(repo) = git2::Repository::open(path) else { return vec!["Could not open repository.".into()]; };
    let Ok(names) = repo.remotes() else { return vec!["No remotes configured.".into()]; };
    let mut remotes = Vec::new();
    for name in names.iter().flatten() {
        let url = repo.find_remote(name).ok().and_then(|r| r.url().map(str::to_owned)).unwrap_or_default();
        remotes.push(format!("{name}: {url}"));
    }
    if remotes.is_empty() {
        remotes.push("No remotes configured.".into());
    }
    remotes
}

fn ssh_key_status() -> Vec<String> {
    let home = std::env::var("HOME").unwrap_or_default();
    let ssh_dir = std::path::Path::new(&home).join(".ssh");
    ["id_ed25519", "id_ecdsa", "id_rsa"]
        .iter()
        .map(|name| {
            let path = ssh_dir.join(name);
            format!("{} {}", if path.exists() { "Found" } else { "Missing" }, path.display())
        })
        .collect()
}

fn open_repo_in_editor(repo_path: Option<std::path::PathBuf>, configured_editor: &str) -> Result<(), String> {
    let path = repo_path.ok_or_else(|| "Open a repository before launching an editor.".to_string())?;
    let editor = if !configured_editor.trim().is_empty() {
        configured_editor.trim().to_string()
    } else {
        std::env::var("VISUAL")
            .or_else(|_| std::env::var("EDITOR"))
            .map_err(|_| "Set a default editor in App Settings first.".to_string())?
    };

    let mut parts = editor.split_whitespace();
    let program = parts.next().ok_or_else(|| "Set a default editor in App Settings first.".to_string())?;
    let mut command = std::process::Command::new(program);
    for arg in parts {
        command.arg(arg);
    }
    command.arg(path);
    command.spawn().map(|_| ()).map_err(|e| format!("Could not open editor: {e}"))
}

fn pull_request_target(repo_path: &std::path::Path, branch: &str) -> Option<(String, String)> {
    let repo = git2::Repository::open(repo_path).ok()?;
    let remote = repo.find_remote("origin").ok()
        .or_else(|| repo.remotes().ok()?.iter().flatten().next().and_then(|name| repo.find_remote(name).ok()))?;
    let remote_url = remote.url()?.to_string();
    let base = github_web_url(&remote_url)?;
    let url = if branch == "HEAD" {
        format!("{base}/pulls")
    } else {
        format!("{base}/compare/{branch}?expand=1")
    };
    Some((remote_url, url))
}

fn github_web_url(remote_url: &str) -> Option<String> {
    let trimmed = remote_url.trim_end_matches(".git");
    if let Some(rest) = trimmed.strip_prefix("git@github.com:") {
        return Some(format!("https://github.com/{rest}"));
    }
    if let Some(rest) = trimmed.strip_prefix("ssh://git@github.com/") {
        return Some(format!("https://github.com/{rest}"));
    }
    if trimmed.starts_with("https://github.com/") || trimmed.starts_with("http://github.com/") {
        return Some(trimmed.replacen("http://", "https://", 1));
    }
    None
}

fn open_url(url: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("Could not open browser: {e}"))
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(url)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("Could not open browser: {e}"))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("Could not open browser: {e}"))
    }
}
