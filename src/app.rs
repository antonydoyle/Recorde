use egui::{Color32, Context, Key, Modifiers, RichText, Stroke};

use crate::auth::{ProviderKind, StoredAccount};
use crate::config::{Config, PullStrategy};
use crate::git::{branches, job::ActiveOp, job::JobProgress, lifecycle, merge, operations::load_repo, rebase, remote, runner::GitRunner, staging};
use crate::state::{ModalKind, PendingDiscard, RebaseEntry, new_shared_state, SharedState};
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
enum SettingsTab { App, Accounts, Repository, About }

pub struct RecordeApp {
    state: SharedState,
    runner: GitRunner,
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
    // Lifecycle modals
    clone_url: String,
    clone_dest: Option<std::path::PathBuf>,
    init_repo_path: Option<std::path::PathBuf>,
    origin_url_input: String,
    // Branch/tag management
    rename_branch_input: String,
    tag_message_input: String,
    // Stash
    stash_message_input: String,
    stash_branch_input: String,
    // Phase 7: History/Review
    reflog_recover_input: String,
    reflog_recover_oid: Option<String>,
    compare_file_selected: Option<String>,
    // Phase 10: Auth & Accounts
    add_account_open: bool,
    add_account_provider: ProviderKind,
    add_account_host: String,
    add_account_username: String,
    add_account_token: String,
    add_account_error: Option<String>,
    // Phase 14: External tool inputs
    terminal_input: String,
    diff_tool_input: String,
    merge_tool_input: String,
}

impl RecordeApp {
    pub fn new(cc: &eframe::CreationContext<'_>, initial_repo: Option<std::path::PathBuf>) -> Self {
        let config = Config::load();
        let dark_theme = config.dark_theme;
        let default_editor = config.default_editor.clone();
        let terminal = config.terminal.clone();
        let diff_tool = config.diff_tool.clone();
        let merge_tool = config.merge_tool.clone();

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
        let runner = GitRunner::new();
        let to_open = initial_repo.or_else(|| config.recent_repos.first().cloned());
        let initial_repo_config = to_open.as_ref().map(|p| config.get_repo_config(p));
        if let Some(path) = to_open {
            load_repo(path, state.clone());
        }

        Self {
            runner,
            toolbar: Toolbar::new(state.clone()),
            sidebar: Sidebar::new(state.clone()),
            commit_graph_view: {
                let mut cgv = CommitGraphView::new(state.clone());
                if let Some(rc) = initial_repo_config {
                    cgv.set_column_widths(rc.refs_w, rc.graph_w, rc.time_w, rc.hash_w);
                }
                cgv
            },
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
            clone_url: String::new(),
            clone_dest: None,
            init_repo_path: None,
            origin_url_input: String::new(),
            rename_branch_input: String::new(),
            tag_message_input: String::new(),
            stash_message_input: "WIP".into(),
            stash_branch_input: String::new(),
            reflog_recover_input: String::new(),
            reflog_recover_oid: None,
            compare_file_selected: None,
            add_account_open: false,
            add_account_provider: ProviderKind::GitHubCom,
            add_account_host: String::new(),
            add_account_username: String::new(),
            add_account_token: String::new(),
            add_account_error: None,
            terminal_input: terminal,
            diff_tool_input: diff_tool,
            merge_tool_input: merge_tool,
        }
    }

    // ── Action handlers ───────────────────────────────────────────────────────

    fn handle_toolbar(&mut self, ctx: &Context, action: ToolbarAction) {
        match action {
            ToolbarAction::OpenRepo(path) => {
                let rc = self.config.get_repo_config(&path);
                self.commit_graph_view.set_column_widths(rc.refs_w, rc.graph_w, rc.time_w, rc.hash_w);
                self.config.add_recent(path.clone());
                load_repo(path, self.state.clone());
            }
            ToolbarAction::CloneRepo => {
                self.state.lock().unwrap().modal = ModalKind::CloneRepo;
            }
            ToolbarAction::InitRepo => {
                self.state.lock().unwrap().modal = ModalKind::InitRepo;
            }
            ToolbarAction::RemoveFromRecent(path) => {
                self.config.remove_recent(&path);
            }
            ToolbarAction::OpenTerminal => {
                if let Some(ref rp) = self.state.lock().unwrap().repo_path.clone() {
                    lifecycle::open_terminal(rp);
                }
            }
            ToolbarAction::RevealInFolder => {
                if let Some(ref rp) = self.state.lock().unwrap().repo_path.clone() {
                    lifecycle::reveal_in_file_manager(rp);
                }
            }
            ToolbarAction::CopyRepoPath => {
                let path_str = self.state.lock().unwrap().repo_path
                    .as_ref().map(|p| p.display().to_string()).unwrap_or_default();
                ctx.output_mut(|o| o.copied_text = path_str);
            }
            ToolbarAction::Stash        => {
                self.stash_message_input = "WIP".into();
                self.state.lock().unwrap().modal = ModalKind::StashWithMessage;
            }
            ToolbarAction::CreateBranch => {
                self.state.lock().unwrap().modal = ModalKind::CreateBranch { from_oid: None };
            }
            ToolbarAction::Fetch          => remote::fetch_all(&mut self.runner, &self.state),
            ToolbarAction::Pull           => remote::pull_branch(&mut self.runner, &self.state),
            ToolbarAction::PullWithRebase => remote::pull_with_rebase(&mut self.runner, &self.state),
            ToolbarAction::PullFfOnly     => remote::pull_ff_only(&mut self.runner, &self.state),
            ToolbarAction::Push           => remote::push_branch(&mut self.runner, &self.state),
            ToolbarAction::PushWithLease  => remote::push_with_lease(&mut self.runner, &self.state),
            ToolbarAction::PushSetUpstream => remote::push_set_upstream(&mut self.runner, &self.state),
            ToolbarAction::MergeBranch    => { self.state.lock().unwrap().modal = ModalKind::MergeBranch; }
            ToolbarAction::RebaseOnto     => { self.state.lock().unwrap().modal = ModalKind::RebaseOnto; }
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
            ToolbarAction::OpenUserMenu => self.open_settings(SettingsTab::Accounts),
            ToolbarAction::ToggleAllBranches => {
                let mut s = self.state.lock().unwrap();
                s.show_all_branches = !s.show_all_branches;
            }
            ToolbarAction::OpenReflog => self.state.lock().unwrap().modal = ModalKind::Reflog,
            ToolbarAction::OpenBisect => self.state.lock().unwrap().modal = ModalKind::Bisect,
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
            SidebarAction::StashApply(i)  => branches::stash_apply(i, self.state.clone()),
            SidebarAction::StashPop(i)    => branches::stash_pop(i, self.state.clone()),
            SidebarAction::StashDrop(i)   => branches::stash_drop(i, self.state.clone()),
            SidebarAction::MergeBranch(n)      => merge::merge_branch(&n, self.state.clone()),
            SidebarAction::RebaseOnto(n)        => rebase::rebase_onto(&n, self.state.clone()),
            SidebarAction::RenameBranch(n)      => { self.state.lock().unwrap().modal = ModalKind::RenameBranch { old_name: n }; }
            SidebarAction::PublishBranch(n)     => branches::publish_branch(&n, self.state.clone()),
            SidebarAction::SetUpstream(n)       => branches::set_upstream(&n, "origin", self.state.clone()),
            SidebarAction::ForceDeleteBranch(n) => branches::force_delete_branch(&n, self.state.clone()),
            SidebarAction::PushTag(n)           => branches::push_tag(&n, self.state.clone()),
            SidebarAction::DeleteRemoteTag(n)   => branches::delete_remote_tag(&n, self.state.clone()),
            SidebarAction::PreviewStash(_, oid) => {
                {
                    let mut s = self.state.lock().unwrap();
                    s.selected_commit = Some(oid.clone());
                    s.selected_commit_detail = None;
                    s.selected_file = None;
                    s.current_diff = None;
                }
                crate::git::operations::load_commit_detail(&oid, self.state.clone());
            }
            SidebarAction::StashBranch(idx) => {
                self.stash_branch_input.clear();
                self.state.lock().unwrap().modal = ModalKind::CreateBranchFromStash { stash_index: idx };
            }
            SidebarAction::RestoreBranch(name, oid) =>
                branches::restore_deleted_branch(&name, &oid, self.state.clone()),
        }
    }

    fn open_settings(&mut self, tab: SettingsTab) {
        self.settings_tab = tab;
        self.settings_open = true;
        self.load_git_identity_inputs();
        self.load_app_settings_inputs();
    }

    fn load_app_settings_inputs(&mut self) {
        self.terminal_input   = self.config.terminal.clone();
        self.diff_tool_input  = self.config.diff_tool.clone();
        self.merge_tool_input = self.config.merge_tool.clone();
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
                ui.add_space(8.0);
                ui.label(styled_label("Message (optional — leave blank for lightweight tag)"));
                ui.add_space(4.0);
                ui.add(egui::TextEdit::multiline(&mut self.tag_message_input)
                    .desired_width(f32::INFINITY).desired_rows(2).hint_text("Annotated tag message…"));
                ui.add_space(12.0);
                let can = !self.create_tag_name.trim().is_empty();
                ui.horizontal(|ui| {
                    let ok     = ui.add_enabled(can, primary_btn("Create Tag")).clicked() || (can && ui.input(|i| i.key_pressed(Key::Enter)));
                    let cancel = ui.add(secondary_btn("Cancel")).clicked() || ui.input(|i| i.key_pressed(Key::Escape));
                    if ok {
                        let msg = if self.tag_message_input.trim().is_empty() { None } else { Some(self.tag_message_input.trim()) };
                        branches::create_tag(self.create_tag_name.trim(), from_oid.as_deref(), msg, self.state.clone());
                        self.state.lock().unwrap().modal = ModalKind::None;
                        self.create_tag_name.clear();
                        self.tag_message_input.clear();
                    }
                    if cancel {
                        self.state.lock().unwrap().modal = ModalKind::None;
                        self.create_tag_name.clear();
                        self.tag_message_input.clear();
                    }
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

            ModalKind::CloneRepo  => self.render_clone_modal(ctx),
            ModalKind::InitRepo   => self.render_init_modal(ctx),
            ModalKind::RenameBranch { old_name } => self.render_rename_branch_modal(ctx, old_name),
            ModalKind::ConfirmDetachedCheckout { oid } => self.render_detached_checkout_modal(ctx, oid),
            ModalKind::StashWithMessage => self.render_stash_message_modal(ctx),
            ModalKind::CreateBranchFromStash { stash_index } => self.render_stash_branch_modal(ctx, stash_index),

            ModalKind::MergeBranch => self.render_branch_picker(ctx, "Merge Branch",
                "Select a branch to merge into the current branch:",
                |app, branch| {
                    merge::merge_branch(&branch, app.state.clone());
                }),

            ModalKind::RebaseOnto => self.render_branch_picker(ctx, "Rebase Onto",
                "Select a branch to rebase current onto:",
                |app, branch| {
                    rebase::rebase_onto(&branch, app.state.clone());
                }),

            ModalKind::Reflog  => self.render_reflog_modal(ctx),
            ModalKind::Bisect  => self.render_bisect_modal(ctx),
            ModalKind::Compare { base_oid, compare_oid } =>
                self.render_compare_modal(ctx, base_oid, compare_oid),
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
                        0 => remote::fetch_all(&mut self.runner, &self.state),
                        1 => remote::pull_branch(&mut self.runner, &self.state),
                        2 => remote::push_branch(&mut self.runner, &self.state),
                        3 => self.state.lock().unwrap().modal = ModalKind::CreateBranch { from_oid: None },
                        4 => self.branch_switcher_open = true,
                        5 => { self.stash_message_input = "WIP".into(); self.state.lock().unwrap().modal = ModalKind::StashWithMessage; }
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
                    settings_tab_button(ui, &mut self.settings_tab, SettingsTab::Accounts, "Accounts & SSH");
                    settings_tab_button(ui, &mut self.settings_tab, SettingsTab::Repository, "Repository");
                    settings_tab_button(ui, &mut self.settings_tab, SettingsTab::About, "About");
                });
                ui.add(egui::Separator::default().spacing(0.0));
                ui.add_space(8.0);

                match self.settings_tab {
                    SettingsTab::App => self.render_app_settings(ctx, ui),
                    SettingsTab::Accounts => self.render_accounts_tab(ui),
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

        // ── Pull strategy ────────────────────────────────────────────────────
        ui.add_space(12.0);
        ui.label(RichText::new("Default pull strategy").color(theme::TEXT_SECONDARY).size(12.5).strong());
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.radio_value(&mut self.config.pull_strategy, PullStrategy::Merge, "Merge").changed() {
                self.config.save();
            }
            if ui.radio_value(&mut self.config.pull_strategy, PullStrategy::Rebase, "Rebase").changed() {
                self.config.save();
            }
            if ui.radio_value(&mut self.config.pull_strategy, PullStrategy::FfOnly, "Fast-forward only").changed() {
                self.config.save();
            }
        });
        ui.label(styled_muted("Used when pulling without an explicit flag."));

        // ── Fetch options ────────────────────────────────────────────────────
        ui.add_space(12.0);
        if ui.checkbox(&mut self.config.fetch_prune, RichText::new("Prune deleted remote branches on fetch").color(theme::TEXT_SECONDARY)).changed() {
            self.config.save();
        }
        if ui.checkbox(&mut self.config.auto_fetch, RichText::new("Auto-fetch on repo open").color(theme::TEXT_SECONDARY)).changed() {
            self.config.save();
        }
        ui.horizontal(|ui| {
            ui.add_enabled_ui(self.config.auto_fetch, |ui| {
                ui.label(RichText::new("Interval (seconds)").color(theme::TEXT_SECONDARY));
                let mut interval_str = self.config.auto_fetch_interval_secs.to_string();
                if ui.add(egui::TextEdit::singleline(&mut interval_str).desired_width(80.0)).changed() {
                    if let Ok(v) = interval_str.trim().parse::<u32>() {
                        self.config.auto_fetch_interval_secs = v;
                        self.config.save();
                    }
                }
            });
        });

        // ── External tools ───────────────────────────────────────────────────
        ui.add_space(12.0);
        ui.label(RichText::new("External tools").color(theme::TEXT_SECONDARY).size(12.5).strong());
        ui.add_space(4.0);
        ui.label(RichText::new("Terminal").color(theme::TEXT_SECONDARY));
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.terminal_input)
                .desired_width(340.0)
                .hint_text("xterm, gnome-terminal, konsole, ..."));
            if ui.add(secondary_btn("Save")).clicked() {
                self.config.terminal = self.terminal_input.trim().to_string();
                self.config.save();
            }
        });
        ui.add_space(4.0);
        ui.label(RichText::new("Diff tool").color(theme::TEXT_SECONDARY));
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.diff_tool_input)
                .desired_width(340.0)
                .hint_text("meld, kdiff3, vimdiff, ..."));
            if ui.add(secondary_btn("Save")).clicked() {
                self.config.diff_tool = self.diff_tool_input.trim().to_string();
                self.config.save();
            }
        });
        ui.add_space(4.0);
        ui.label(RichText::new("Merge tool").color(theme::TEXT_SECONDARY));
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.merge_tool_input)
                .desired_width(340.0)
                .hint_text("meld, kdiff3, ..."));
            if ui.add(secondary_btn("Save")).clicked() {
                self.config.merge_tool = self.merge_tool_input.trim().to_string();
                self.config.save();
            }
        });
    }

    fn render_accounts_tab(&mut self, ui: &mut egui::Ui) {
        // ── Provider Accounts ─────────────────────────────────────────────
        ui.label(RichText::new("Provider Accounts").color(theme::TEXT_SECONDARY).size(12.5).strong());
        ui.add_space(4.0);

        let accounts = self.config.accounts.clone();
        let mut remove_idx: Option<usize> = None;
        for (i, acc) in accounts.iter().enumerate() {
            ui.horizontal(|ui| {
                let dot_color = match acc.provider {
                    ProviderKind::GitHubCom | ProviderKind::GitHubEnterprise => theme::TEXT_PRIMARY,
                    ProviderKind::GitLabCom | ProviderKind::GitLabSelf        => theme::ACCENT_RED,
                };
                let (dot_rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                ui.painter().circle_filled(dot_rect.center(), 3.5, dot_color);
                ui.add_space(4.0);
                ui.label(RichText::new(acc.label()).color(theme::TEXT_SECONDARY).size(12.0));
                let has_token = crate::auth::store::load(&acc.host, &acc.username).is_some();
                let (badge, badge_color) = if has_token { ("token stored", theme::ACCENT_TEAL) } else { ("no token", theme::ACCENT_RED) };
                ui.label(RichText::new(badge).color(badge_color).size(10.5));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(egui::Button::new(RichText::new("Remove").color(theme::ACCENT_RED).size(11.5))
                        .fill(Color32::TRANSPARENT)).clicked() {
                        crate::auth::store::remove(&acc.host, &acc.username);
                        remove_idx = Some(i);
                    }
                });
            });
        }
        if let Some(i) = remove_idx { self.config.accounts.remove(i); self.config.save(); }

        if accounts.is_empty() && !self.add_account_open {
            ui.label(styled_muted("No provider accounts connected."));
        }
        ui.add_space(6.0);

        if self.add_account_open {
            ui.separator();
            ui.add_space(6.0);
            ui.label(RichText::new("Add Account").color(theme::TEXT_SECONDARY).size(12.5).strong());
            ui.add_space(4.0);
            // Provider picker
            ui.horizontal(|ui| {
                ui.label(styled_label("Provider"));
                ui.add_space(8.0);
                for p in [ProviderKind::GitHubCom, ProviderKind::GitHubEnterprise,
                           ProviderKind::GitLabCom, ProviderKind::GitLabSelf] {
                    let active = self.add_account_provider == p;
                    let color = if active { theme::ACCENT_TEAL } else { theme::TEXT_MUTED };
                    if ui.add(egui::Button::new(RichText::new(p.label()).size(11.5).color(color))
                        .fill(Color32::TRANSPARENT)).clicked() && !active {
                        self.add_account_host = p.default_host().to_string();
                        self.add_account_provider = p.clone();
                    }
                }
            });
            ui.add_space(4.0);
            if self.add_account_provider.needs_host() {
                settings_text_field(ui, "Host", &mut self.add_account_host);
            } else {
                ui.label(styled_muted(&format!("Host: {}", self.add_account_provider.default_host())));
            }
            settings_text_field(ui, "Username", &mut self.add_account_username);
            ui.horizontal(|ui| {
                ui.set_min_height(22.0);
                ui.label(RichText::new(format!("{:<12}", "Token (PAT)")).monospace().color(theme::TEXT_MUTED).size(11.5));
                ui.add(egui::TextEdit::singleline(&mut self.add_account_token)
                    .password(true).desired_width(260.0).hint_text("ghp_… / glpat-…"));
            });
            if let Some(ref err) = self.add_account_error.clone() {
                ui.label(RichText::new(err).color(theme::ACCENT_RED).size(11.5));
            }
            ui.add_space(6.0);
            let eff_host = if self.add_account_provider.needs_host() {
                self.add_account_host.trim().to_string()
            } else { self.add_account_provider.default_host().to_string() };
            let can_save = !self.add_account_username.trim().is_empty()
                && !self.add_account_token.trim().is_empty()
                && !eff_host.is_empty();
            ui.horizontal(|ui| {
                if ui.add_enabled(can_save, primary_btn("Save Account")).clicked() {
                    let username = self.add_account_username.trim().to_string();
                    let token    = self.add_account_token.trim().to_string();
                    match crate::auth::store::save(&eff_host, &username, &token) {
                        Ok(()) => {
                            self.config.accounts.push(StoredAccount {
                                provider: self.add_account_provider.clone(),
                                host: eff_host, username,
                            });
                            self.config.save();
                            self.add_account_open = false;
                            self.add_account_username.clear();
                            self.add_account_token.clear();
                            self.add_account_error = None;
                        }
                        Err(e) => self.add_account_error = Some(e),
                    }
                }
                if ui.add(secondary_btn("Cancel")).clicked() {
                    self.add_account_open = false;
                    self.add_account_username.clear();
                    self.add_account_token.clear();
                    self.add_account_error = None;
                }
            });
            ui.separator();
        } else if ui.add(secondary_btn("Add Account…")).clicked() {
            self.add_account_open = true;
            self.add_account_provider = ProviderKind::GitHubCom;
            self.add_account_host = ProviderKind::GitHubCom.default_host().to_string();
            self.add_account_error = None;
        }

        ui.add_space(14.0);

        // ── SSH Diagnostics ───────────────────────────────────────────────
        ui.label(RichText::new("SSH Keys").color(theme::TEXT_SECONDARY).size(12.5).strong());
        ui.add_space(4.0);
        let diag = crate::auth::ssh::diagnose();
        let (agent_text, agent_color) = if diag.agent_running {
            (if diag.agent_keys.is_empty() { "running (no keys loaded)" } else { "running" }, theme::ACCENT_TEAL)
        } else { ("not running / SSH_AUTH_SOCK unset", theme::TEXT_MUTED) };
        health_row_color(ui, "SSH agent", agent_text, agent_color);
        for key in &diag.agent_keys {
            ui.label(RichText::new(format!("  {}", key)).monospace().color(theme::TEXT_MUTED).size(10.5));
        }
        for (name, exists) in &diag.key_files {
            let (status, color) = if *exists { ("found", theme::TEXT_SECONDARY) } else { ("missing", theme::TEXT_MUTED) };
            health_row_color(ui, &format!("~/.ssh/{}", name), status, color);
        }

        ui.add_space(14.0);

        // ── Git Identity ──────────────────────────────────────────────────
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
        ui.add_space(8.0);
        ui.label(styled_muted(&format!("System credential helper: {}",
            git_config_value(None, "credential.helper").unwrap_or_else(|| "not configured".into()))));
        ui.label(styled_muted("Tokens stored by Recorde take precedence over the system helper for HTTPS remotes."));
    }

    fn render_repo_settings(&mut self, ui: &mut egui::Ui) {
        let (repo_path, head_branch, ahead_behind, dirty_count, conflict_count, caps) = {
            let s = self.state.lock().unwrap();
            (
                s.repo_path.clone(),
                s.head_branch.clone().unwrap_or_else(|| "—".into()),
                s.ahead_behind,
                s.dirty_count,
                s.conflicted_files.len(),
                s.capabilities.clone(),
            )
        };
        let Some(path) = repo_path else {
            ui.label(styled_muted("Open a repository to edit repository settings."));
            return;
        };

        ui.label(RichText::new("Repository").color(theme::TEXT_SECONDARY).size(12.5).strong());
        ui.label(styled_muted(&path.display().to_string()));
        ui.add_space(10.0);

        // ── Health ──────────────────────────────────────────────────────
        ui.label(RichText::new("Repository Health").color(theme::TEXT_SECONDARY).size(12.5).strong());
        ui.add_space(4.0);
        let ab_str = match ahead_behind {
            Some((a, b)) => format!("↑{} ↓{}", a, b),
            None => "no upstream".into(),
        };
        health_row(ui, "Branch", &format!("{} {}", head_branch, ab_str));
        let dirty_str = if dirty_count == 0 && conflict_count == 0 {
            "clean".into()
        } else {
            let mut parts = Vec::new();
            if dirty_count > 0 { parts.push(format!("{} changed", dirty_count)); }
            if conflict_count > 0 { parts.push(format!("{} conflicted", conflict_count)); }
            parts.join(", ")
        };
        health_row(ui, "Working tree", &dirty_str);
        if let Some(ref caps) = caps {
            let state_str = if caps.rebase_in_progress { "Rebasing" }
                else if caps.merge_in_progress { "Merging" }
                else if caps.cherry_pick_in_progress { "Cherry-picking" }
                else if caps.revert_in_progress { "Reverting" }
                else { "Normal" };
            health_row(ui, "State", state_str);
            // LFS row — three states
            if !caps.lfs_installed {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{:<14}", "LFS")).color(theme::TEXT_MUTED).size(11.5).monospace());
                    ui.label(RichText::new("not installed").color(theme::TEXT_MUTED).size(11.5));
                });
            } else if !caps.lfs_active {
                health_row(ui, "LFS", "installed, not used");
            } else {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{:<14}", "LFS")).color(theme::TEXT_MUTED).size(11.5).monospace());
                    ui.label(RichText::new("active").color(theme::ACCENT_TEAL).size(11.5));
                    if !caps.lfs_patterns.is_empty() {
                        ui.label(RichText::new(format!("({})", caps.lfs_patterns.join(", "))).color(theme::TEXT_SECONDARY).size(11.5));
                    }
                });
            }
            health_row(ui, "Submodules", if caps.has_submodules { "present" } else { "none" });
            health_row(ui, "Provider", &format!("{:?}", caps.provider));
        }
        let git_ver = std::process::Command::new("git").arg("--version")
            .output().ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .unwrap_or_else(|| "unknown".into());
        health_row(ui, "Git", git_ver.trim());

        ui.add_space(12.0);

        // ── Origin URL ───────────────────────────────────────────────────
        ui.label(RichText::new("Origin URL").color(theme::TEXT_SECONDARY).size(12.5).strong());
        ui.add_space(4.0);
        if self.origin_url_input.is_empty() {
            // Pre-populate on first open
            self.origin_url_input = git_config_value(Some(&path), "remote.origin.url")
                .unwrap_or_default();
        }
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.origin_url_input)
                .desired_width(340.0).hint_text("https://github.com/owner/repo.git"));
            if ui.add(secondary_btn("Save")).clicked() {
                let url = self.origin_url_input.trim().to_string();
                if !url.is_empty() {
                    let _ = crate::git::merge::run_git_seq(&path, &["remote", "set-url", "origin", &url]);
                }
            }
        });

        ui.add_space(14.0);

        // ── Git identity ─────────────────────────────────────────────────
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
            remote::fetch_all(&mut self.runner, &self.state);
        }
        if ctx.input(|i| i.modifiers.ctrl && i.modifiers.shift && i.key_pressed(Key::P)) {
            remote::pull_branch(&mut self.runner, &self.state);
        }
        if ctx.input(|i| i.modifiers.ctrl && i.modifiers.shift && i.key_pressed(Key::U)) {
            remote::push_branch(&mut self.runner, &self.state);
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

        // ── Drain runner progress events ──────────────────────────────────
        self.apply_runner_progress();

        // ── Persist graph column widths when they change ──────────────────
        if let Some((rw, gw, tw, hw)) = self.commit_graph_view.take_dirty_widths() {
            if let Some(ref path) = self.state.lock().unwrap().repo_path.clone() {
                self.config.set_repo_column_widths(path, rw, gw, tw, hw);
            }
        }

        // ── In-progress operation banner (rebase/merge/cherry-pick/revert) ─
        self.show_in_progress_banner(ctx);

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
    fn render_rename_branch_modal(&mut self, ctx: &Context, old_name: String) {
        if self.rename_branch_input.is_empty() {
            self.rename_branch_input = old_name.clone();
        }
        let mut close = false;
        let mut do_rename = false;
        modal_window(ctx, "Rename Branch", |ui| {
            ui.label(styled_label("New name"));
            ui.add_space(4.0);
            let r = ui.add(egui::TextEdit::singleline(&mut self.rename_branch_input)
                .desired_width(f32::INFINITY));
            r.request_focus();
            ui.add_space(12.0);
            let can = !self.rename_branch_input.trim().is_empty()
                && self.rename_branch_input.trim() != old_name.as_str();
            ui.horizontal(|ui| {
                let ok = ui.add_enabled(can, primary_btn("Rename")).clicked()
                    || (can && ui.input(|i| i.key_pressed(Key::Enter)));
                let cancel = ui.add(secondary_btn("Cancel")).clicked()
                    || ui.input(|i| i.key_pressed(Key::Escape));
                if ok     { do_rename = true; }
                if cancel { close = true; }
            });
        });
        if do_rename {
            branches::rename_branch(&old_name, self.rename_branch_input.trim(), self.state.clone());
            self.rename_branch_input.clear();
            self.state.lock().unwrap().modal = ModalKind::None;
        } else if close {
            self.rename_branch_input.clear();
            self.state.lock().unwrap().modal = ModalKind::None;
        }
    }

    fn render_detached_checkout_modal(&mut self, ctx: &Context, oid: String) {
        let mut close = false;
        let mut do_checkout = false;
        let short = &oid[..7.min(oid.len())];
        modal_window(ctx, "Checkout Commit", |ui| {
            ui.label(RichText::new(format!("Checkout commit {}?", short)).color(theme::TEXT_PRIMARY).size(13.0));
            ui.add_space(6.0);
            ui.label(styled_muted("This will put you in detached HEAD state."));
            ui.label(styled_muted("Any commits made here won't be on a named branch."));
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                if ui.add(danger_btn("Checkout")).clicked() { do_checkout = true; }
                if ui.add(secondary_btn("Cancel")).clicked()
                    || ui.input(|i| i.key_pressed(Key::Escape)) { close = true; }
            });
        });
        if do_checkout {
            branches::checkout_detached(&oid, self.state.clone());
            self.state.lock().unwrap().modal = ModalKind::None;
        } else if close {
            self.state.lock().unwrap().modal = ModalKind::None;
        }
    }

    fn render_stash_message_modal(&mut self, ctx: &Context) {
        let mut close = false;
        let mut do_stash = false;
        modal_window(ctx, "Stash Changes", |ui| {
            ui.label(styled_label("Stash message"));
            ui.add_space(4.0);
            let r = ui.add(egui::TextEdit::singleline(&mut self.stash_message_input)
                .desired_width(f32::INFINITY).hint_text("WIP"));
            r.request_focus();
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let ok = ui.add(primary_btn("Stash")).clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Enter));
                let cancel = ui.add(secondary_btn("Cancel")).clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Escape));
                if ok     { do_stash = true; }
                if cancel { close = true; }
            });
        });
        if do_stash {
            let msg = if self.stash_message_input.trim().is_empty() { "WIP".to_string() }
                      else { self.stash_message_input.trim().to_string() };
            branches::stash_save_with_message(&msg, self.state.clone());
            self.stash_message_input = "WIP".into();
            self.state.lock().unwrap().modal = ModalKind::None;
        } else if close {
            self.stash_message_input = "WIP".into();
            self.state.lock().unwrap().modal = ModalKind::None;
        }
    }

    fn render_stash_branch_modal(&mut self, ctx: &Context, stash_index: usize) {
        let mut close = false;
        let mut do_branch = false;
        modal_window(ctx, "Create Branch From Stash", |ui| {
            ui.label(styled_label("Branch name"));
            ui.add_space(4.0);
            let r = ui.add(egui::TextEdit::singleline(&mut self.stash_branch_input)
                .desired_width(f32::INFINITY).hint_text("e.g. feat/my-feature"));
            r.request_focus();
            ui.add_space(4.0);
            ui.label(styled_muted("Creates a branch from the stash's base commit, applies the stash, and drops it."));
            ui.add_space(12.0);
            let can = !self.stash_branch_input.trim().is_empty();
            ui.horizontal(|ui| {
                let ok = ui.add_enabled(can, primary_btn("Create Branch")).clicked()
                    || (can && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                let cancel = ui.add(secondary_btn("Cancel")).clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Escape));
                if ok     { do_branch = true; }
                if cancel { close = true; }
            });
        });
        if do_branch {
            branches::stash_branch(self.stash_branch_input.trim(), stash_index, self.state.clone());
            self.stash_branch_input.clear();
            self.state.lock().unwrap().modal = ModalKind::None;
        } else if close {
            self.stash_branch_input.clear();
            self.state.lock().unwrap().modal = ModalKind::None;
        }
    }

    fn render_clone_modal(&mut self, ctx: &Context) {
        let mut close = false;
        let mut do_clone = false;

        modal_window(ctx, "Clone Repository", |ui| {
            ui.label(styled_label("Repository URL"));
            ui.add_space(4.0);
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.clone_url)
                    .desired_width(f32::INFINITY)
                    .hint_text("https://github.com/owner/repo.git"),
            );
            r.request_focus();
            ui.add_space(8.0);
            ui.label(styled_label("Destination"));
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let dest_str = self.clone_dest.as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "Choose folder…".into());
                ui.label(RichText::new(dest_str).color(theme::TEXT_SECONDARY).size(12.0));
                if ui.add(secondary_btn("Browse")).clicked() {
                    if let Some(p) = rfd::FileDialog::new().pick_folder() {
                        self.clone_dest = Some(p);
                    }
                }
            });
            ui.add_space(16.0);
            let can_clone = !self.clone_url.trim().is_empty() && self.clone_dest.is_some();
            ui.horizontal(|ui| {
                let ok = ui.add_enabled(can_clone, primary_btn("Clone")).clicked()
                    || (can_clone && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                let cancel = ui.add(secondary_btn("Cancel")).clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Escape));
                if ok     { do_clone = true; }
                if cancel { close = true; }
            });
        });

        if do_clone {
            if let Some(dest) = self.clone_dest.clone() {
                let mut dest_path = dest;
                // Append the repo name from the URL as the subdirectory
                let repo_name = self.clone_url.trim_end_matches('/')
                    .trim_end_matches(".git")
                    .rsplit('/')
                    .next()
                    .unwrap_or("repo")
                    .to_string();
                dest_path.push(&repo_name);
                self.config.add_recent(dest_path.clone());
                lifecycle::clone_repo(self.clone_url.trim(), dest_path, self.state.clone());
            }
            self.clone_url.clear();
            self.clone_dest = None;
            self.state.lock().unwrap().modal = ModalKind::None;
        } else if close {
            self.clone_url.clear();
            self.clone_dest = None;
            self.state.lock().unwrap().modal = ModalKind::None;
        }
    }

    fn render_init_modal(&mut self, ctx: &Context) {
        let mut close = false;
        let mut do_init = false;

        modal_window(ctx, "Initialize Repository", |ui| {
            ui.label(styled_label("Choose an empty folder to initialise as a git repository:"));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let path_str = self.init_repo_path.as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "No folder selected".into());
                ui.label(RichText::new(path_str).color(theme::TEXT_SECONDARY).size(12.0));
                if ui.add(secondary_btn("Browse")).clicked() {
                    if let Some(p) = rfd::FileDialog::new().pick_folder() {
                        self.init_repo_path = Some(p);
                    }
                }
            });
            ui.add_space(16.0);
            let can_init = self.init_repo_path.is_some();
            ui.horizontal(|ui| {
                let ok = ui.add_enabled(can_init, primary_btn("Initialize")).clicked();
                let cancel = ui.add(secondary_btn("Cancel")).clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Escape));
                if ok     { do_init = true; }
                if cancel { close = true; }
            });
        });

        if do_init {
            if let Some(path) = self.init_repo_path.clone() {
                self.config.add_recent(path.clone());
                lifecycle::init_repo(path, self.state.clone());
            }
            self.init_repo_path = None;
            self.state.lock().unwrap().modal = ModalKind::None;
        } else if close {
            self.init_repo_path = None;
            self.state.lock().unwrap().modal = ModalKind::None;
        }
    }

    fn render_reflog_modal(&mut self, ctx: &Context) {
        use crate::git::reflog;

        let (repo_path, entries) = {
            let s = self.state.lock().unwrap();
            (s.repo_path.clone(), s.reflog_entries.clone())
        };

        // Load on first open
        if entries.is_empty() {
            if let Some(ref p) = repo_path {
                let loaded = reflog::load_reflog(p);
                self.state.lock().unwrap().reflog_entries = loaded;
            }
        }

        let mut close = false;
        let mut recover_request: Option<(String, String)> = None; // (oid, branch_name)

        egui::Window::new("Reflog")
            .collapsible(false)
            .resizable(true)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .default_size([620.0, 480.0])
            .show(ctx, |ui| {
                let entries = self.state.lock().unwrap().reflog_entries.clone();

                ui.horizontal(|ui| {
                    ui.heading(RichText::new("Reflog").size(14.0).color(theme::TEXT_PRIMARY));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(egui::Button::new("✕").fill(Color32::TRANSPARENT)).clicked() {
                            close = true;
                        }
                    });
                });
                ui.add_space(8.0);
                ui.separator();

                egui::ScrollArea::vertical().max_height(340.0).show(ui, |ui| {
                    for entry in &entries {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&entry.short_oid).monospace().color(theme::ACCENT_TEAL).size(11.5));
                            ui.add_space(6.0);
                            ui.label(RichText::new(&entry.refname).color(theme::TEXT_MUTED).size(11.0));
                            ui.add_space(6.0);
                            ui.label(RichText::new(&entry.message).color(theme::TEXT_SECONDARY).size(11.5));
                        }).response.context_menu(|ui| {
                            ui.set_min_width(180.0);
                            if ui.button("Recover as Branch…").clicked() {
                                self.reflog_recover_oid = Some(entry.oid.clone());
                                self.reflog_recover_input = format!("recover-{}", &entry.short_oid);
                                ui.close_menu();
                            }
                            if ui.button("Copy SHA").clicked() {
                                ui.output_mut(|o| o.copied_text = entry.oid.clone());
                                ui.close_menu();
                            }
                        });
                    }
                    if entries.is_empty() {
                        ui.label(RichText::new("No reflog entries.").color(theme::TEXT_MUTED).size(12.0));
                    }
                });

                if self.reflog_recover_oid.is_some() {
                    ui.separator();
                    ui.add_space(6.0);
                    ui.label(RichText::new("Branch name for recovery:").color(theme::TEXT_SECONDARY).size(12.0));
                    ui.add_space(4.0);
                    ui.add(egui::TextEdit::singleline(&mut self.reflog_recover_input)
                        .desired_width(f32::INFINITY));
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        let can = !self.reflog_recover_input.trim().is_empty();
                        if ui.add_enabled(can, primary_btn("Recover")).clicked() {
                            if let Some(ref oid) = self.reflog_recover_oid {
                                recover_request = Some((oid.clone(), self.reflog_recover_input.trim().to_string()));
                            }
                        }
                        if ui.add(secondary_btn("Cancel")).clicked() {
                            self.reflog_recover_oid = None;
                        }
                    });
                }

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(secondary_btn("Close")).clicked() { close = true; }
                    });
                });
            });

        if let Some((oid, branch)) = recover_request {
            if let Some(ref p) = repo_path {
                match reflog::recover(p, &oid, &branch) {
                    Ok(()) => crate::git::branches::reload_repo(self.state.clone()),
                    Err(e) => self.state.lock().unwrap().error = Some(e),
                }
            }
            self.reflog_recover_oid = None;
            self.state.lock().unwrap().modal = ModalKind::None;
        } else if close {
            self.state.lock().unwrap().reflog_entries.clear();
            self.state.lock().unwrap().modal = ModalKind::None;
        }
    }

    fn render_bisect_modal(&mut self, ctx: &Context) {
        use crate::git::bisect;

        let (repo_path, bisect_active, bisect_oid, selected_oid) = {
            let s = self.state.lock().unwrap();
            (s.repo_path.clone(), s.bisect_in_progress, s.bisect_current_oid.clone(), s.selected_commit.clone())
        };

        let mut close = false;
        let mut action: Option<String> = None; // "good" | "bad" | "reset"

        modal_window(ctx, "Bisect", |ui| {
            ui.set_min_width(380.0);

            if !bisect_active {
                ui.label(RichText::new("Start a bisect session to find which commit introduced a bug.").color(theme::TEXT_SECONDARY).size(12.0));
                ui.add_space(8.0);
                ui.label(RichText::new("Select the known-bad commit in the graph, then mark it bad. Select a known-good commit and mark it good.").color(theme::TEXT_MUTED).size(11.5));
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.add(primary_btn("Start Bisect")).clicked() {
                        if let Some(ref p) = repo_path {
                            match bisect::start(p) {
                                Ok(()) => { crate::git::branches::reload_repo(self.state.clone()); }
                                Err(e) => { self.state.lock().unwrap().error = Some(e); }
                            }
                        }
                    }
                    if ui.add(secondary_btn("Cancel")).clicked() { close = true; }
                });
            } else {
                ui.label(RichText::new("Bisect in progress").color(theme::ACCENT_TEAL).size(12.5).strong());
                ui.add_space(6.0);

                if let Some(ref oid) = bisect_oid {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Testing:").color(theme::TEXT_MUTED).size(12.0));
                        ui.label(RichText::new(&oid[..7.min(oid.len())]).monospace().color(theme::ACCENT_TEAL).size(12.0));
                    });
                    ui.add_space(4.0);
                }

                let cur = selected_oid.as_deref().unwrap_or("");
                let short = if cur.len() >= 7 { &cur[..7] } else { cur };
                if cur.is_empty() {
                    ui.label(RichText::new("Select a commit in the graph, then mark it good or bad.").color(theme::TEXT_MUTED).size(11.5));
                } else {
                    ui.label(RichText::new(format!("Selected: {}", short)).color(theme::TEXT_SECONDARY).size(12.0));
                }
                ui.add_space(10.0);

                ui.horizontal(|ui| {
                    let has_sel = !cur.is_empty();
                    if ui.add_enabled(has_sel, egui::Button::new(RichText::new("Mark Good").color(Color32::WHITE)).fill(theme::ACCENT_GREEN).min_size([110.0, 26.0].into())).clicked() {
                        action = Some("good".into());
                    }
                    ui.add_space(6.0);
                    if ui.add_enabled(has_sel, egui::Button::new(RichText::new("Mark Bad").color(Color32::WHITE)).fill(theme::ACCENT_RED).min_size([110.0, 26.0].into())).clicked() {
                        action = Some("bad".into());
                    }
                    ui.add_space(12.0);
                    if ui.add(secondary_btn("Reset")).clicked() { action = Some("reset".into()); }
                });
            }
        });

        if let Some(act) = action {
            if let Some(ref p) = repo_path {
                let sel = selected_oid.unwrap_or_default();
                let result = match act.as_str() {
                    "good"  => bisect::mark_good(p, &sel).map(|next| { if let Some(n) = next { self.state.lock().unwrap().selected_commit = Some(n); } }),
                    "bad"   => bisect::mark_bad(p, &sel).map(|next| { if let Some(n) = next { self.state.lock().unwrap().selected_commit = Some(n); } }),
                    "reset" => bisect::reset(p).map(|_| {}),
                    _ => Ok(()),
                };
                if let Err(e) = result { self.state.lock().unwrap().error = Some(e); }
                crate::git::branches::reload_repo(self.state.clone());
            }
            if act == "reset" { self.state.lock().unwrap().modal = ModalKind::None; }
        }
        if close { self.state.lock().unwrap().modal = ModalKind::None; }
    }

    fn render_compare_modal(&mut self, ctx: &Context, base_oid: String, compare_oid: String) {
        use crate::git::operations::{load_compare_files, load_compare_diff};

        let (repo_path, compare_files, compare_diff, compare_sel) = {
            let s = self.state.lock().unwrap();
            (s.repo_path.clone(), s.compare_files.clone(), s.compare_diff.clone(), s.compare_selected_file.clone())
        };

        // Load file list on first open
        if compare_files.is_empty() {
            load_compare_files(&base_oid, &compare_oid, self.state.clone());
        }

        let mut close = false;
        let mut select_file: Option<String> = None;

        egui::Window::new("Compare Commits")
            .collapsible(false)
            .resizable(true)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .default_size([800.0, 560.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Base:").color(theme::TEXT_MUTED).size(11.5));
                    ui.label(RichText::new(&base_oid[..7.min(base_oid.len())]).monospace().color(theme::ACCENT_TEAL).size(11.5));
                    ui.add_space(12.0);
                    ui.label(RichText::new("Compare:").color(theme::TEXT_MUTED).size(11.5));
                    ui.label(RichText::new(&compare_oid[..7.min(compare_oid.len())]).monospace().color(theme::ACCENT_TEAL).size(11.5));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(egui::Button::new("✕").fill(Color32::TRANSPARENT)).clicked() {
                            close = true;
                        }
                    });
                });
                ui.separator();

                ui.columns(2, |cols| {
                    // Left: file list
                    cols[0].label(RichText::new(format!("Changed files ({})", compare_files.len())).color(theme::TEXT_MUTED).size(11.0).strong());
                    cols[0].add_space(4.0);
                    egui::ScrollArea::vertical().id_source("cmp_files").max_height(460.0).show(&mut cols[0], |ui| {
                        for file in &compare_files {
                            let is_sel = compare_sel.as_deref() == Some(&file.path);
                            let color = if is_sel { theme::BG_APP } else { theme::TEXT_SECONDARY };
                            let resp = ui.selectable_label(is_sel,
                                RichText::new(file.path.rsplit('/').next().unwrap_or(&file.path)).color(color).size(12.0))
                                .on_hover_text(&file.path);
                            if resp.clicked() { select_file = Some(file.path.clone()); }
                        }
                        if compare_files.is_empty() {
                            ui.label(RichText::new("Loading…").color(theme::TEXT_MUTED).size(12.0));
                        }
                    });

                    // Right: diff view
                    if let Some(ref diff) = compare_diff {
                        cols[1].label(RichText::new(format!("+{}  −{}", diff.additions, diff.deletions)).color(theme::TEXT_MUTED).size(11.0));
                        cols[1].add_space(4.0);
                        egui::ScrollArea::vertical().id_source("cmp_diff").max_height(460.0).show(&mut cols[1], |ui| {
                            for hunk in &diff.hunks {
                                ui.label(RichText::new(&hunk.header).monospace().color(theme::ACCENT_PURPLE).size(11.0).background_color(theme::BG_DIFF_HUNK));
                                for line in &hunk.lines {
                                    let (color, bg) = match line.origin {
                                        '+' => (theme::ACCENT_GREEN, theme::BG_DIFF_ADD),
                                        '-' => (theme::ACCENT_RED,   theme::BG_DIFF_DEL),
                                        _   => (theme::TEXT_SECONDARY, Color32::TRANSPARENT),
                                    };
                                    let text = format!("{}{}", line.origin, line.content.trim_end_matches('\n'));
                                    let label = egui::RichText::new(&text).monospace().color(color).size(11.5);
                                    if bg != Color32::TRANSPARENT {
                                        ui.label(label.background_color(bg));
                                    } else {
                                        ui.label(label);
                                    }
                                }
                            }
                        });
                    } else if compare_sel.is_some() {
                        cols[1].label(RichText::new("Loading diff…").color(theme::TEXT_MUTED).size(12.0));
                    } else {
                        cols[1].label(RichText::new("Select a file to view its diff.").color(theme::TEXT_MUTED).size(12.0));
                    }
                });
            });

        if let Some(fp) = select_file {
            self.compare_file_selected = Some(fp.clone());
            self.state.lock().unwrap().compare_selected_file = Some(fp.clone());
            load_compare_diff(&base_oid, &compare_oid, &fp, self.state.clone());
        }

        if close {
            let mut s = self.state.lock().unwrap();
            s.compare_files.clear();
            s.compare_diff = None;
            s.compare_selected_file = None;
            s.compare_base_oid = None;
            s.modal = ModalKind::None;
        }
    }

    /// Generic branch-picker modal used for Merge and Rebase.
    fn render_branch_picker(&mut self, ctx: &Context, title: &str, hint: &str, on_confirm: impl Fn(&mut Self, String)) {
        let branches_list: Vec<String> = {
            let s = self.state.lock().unwrap();
            let head = s.head_branch.clone().unwrap_or_default();
            s.commit_graph.as_ref()
                .map(|g| {
                    let mut refs: Vec<&str> = g.nodes.iter()
                        .flat_map(|n| n.refs.iter().map(|r| r.as_str()))
                        .collect();
                    refs.sort_unstable(); refs.dedup();
                    refs.iter()
                        .filter(|r| !r.contains('/') && **r != head.as_str())
                        .map(|s| s.to_string()).collect()
                })
                .unwrap_or_default()
        };

        let mut close = false;
        let mut chosen: Option<String> = None;

        egui::Window::new(title)
            .collapsible(false).resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(egui::Frame::window(&ctx.style()).inner_margin(egui::Margin::same(16.0)))
            .show(ctx, |ui| {
                ui.set_min_width(340.0);
                ui.label(RichText::new(title).color(theme::TEXT_PRIMARY).size(15.0).strong());
                ui.add_space(6.0);
                ui.label(styled_muted(hint));
                ui.add_space(8.0);
                egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                    for name in &branches_list {
                        if ui.selectable_label(false, RichText::new(name).color(theme::TEXT_SECONDARY).size(12.5)).clicked() {
                            chosen = Some(name.clone());
                        }
                    }
                    if branches_list.is_empty() {
                        ui.label(styled_muted("No other local branches."));
                    }
                });
                ui.add_space(8.0);
                if ui.add(secondary_btn("Cancel")).clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                    close = true;
                }
            });

        if let Some(branch) = chosen {
            self.state.lock().unwrap().modal = ModalKind::None;
            on_confirm(self, branch);
        } else if close {
            self.state.lock().unwrap().modal = ModalKind::None;
        }
    }

    /// Persistent banner shown when a git sequencing operation is in progress.
    fn show_in_progress_banner(&mut self, ctx: &Context) {
        let caps = { self.state.lock().unwrap().capabilities.clone() };
        let Some(caps) = caps else { return };

        let (label, show_skip) = if caps.rebase_in_progress {
            ("Rebase in progress", true)
        } else if caps.merge_in_progress {
            ("Merge in progress", false)
        } else if caps.cherry_pick_in_progress {
            ("Cherry-pick in progress", false)
        } else if caps.revert_in_progress {
            ("Revert in progress", false)
        } else {
            return;
        };

        let mut do_continue = false;
        let mut do_skip = false;
        let mut do_abort = false;

        egui::TopBottomPanel::top("in_progress_banner")
            .exact_height(36.0)
            .frame(egui::Frame::none()
                .fill(Color32::from_rgb(0x4A, 0x32, 0x00))
                .inner_margin(egui::Margin::symmetric(16.0, 0.0)))
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.label(RichText::new("⟳").color(theme::ACCENT_YELLOW).size(14.0));
                    ui.add_space(6.0);
                    ui.label(RichText::new(label).color(theme::ACCENT_YELLOW).size(12.5).strong());
                    ui.add_space(16.0);
                    ui.label(styled_muted("Resolve conflicts, stage changes, then:"));
                    ui.add_space(10.0);
                    if ui.add(egui::Button::new(RichText::new("Continue").color(Color32::WHITE).size(12.0))
                        .fill(theme::ACCENT_TEAL).rounding(5.0)
                        .min_size(egui::Vec2::new(72.0, 24.0))).clicked() { do_continue = true; }
                    if show_skip {
                        ui.add_space(4.0);
                        if ui.add(egui::Button::new(RichText::new("Skip").color(theme::TEXT_SECONDARY).size(12.0))
                            .fill(Color32::TRANSPARENT)
                            .stroke(Stroke::new(1.0, theme::BORDER))
                            .rounding(5.0)
                            .min_size(egui::Vec2::new(52.0, 24.0))).clicked() { do_skip = true; }
                    }
                    ui.add_space(4.0);
                    if ui.add(egui::Button::new(RichText::new("Abort").color(theme::ACCENT_RED).size(12.0))
                        .fill(Color32::TRANSPARENT)
                        .stroke(Stroke::new(1.0, theme::ACCENT_RED.gamma_multiply(0.5)))
                        .rounding(5.0)
                        .min_size(egui::Vec2::new(52.0, 24.0))).clicked() { do_abort = true; }
                });
            });

        if do_continue {
            if caps.rebase_in_progress       { rebase::continue_rebase(self.state.clone()); }
            else if caps.merge_in_progress   { merge::continue_merge(self.state.clone()); }
            else if caps.cherry_pick_in_progress { branches::continue_cherry_pick(self.state.clone()); }
            else if caps.revert_in_progress  { branches::continue_revert(self.state.clone()); }
        }
        if do_skip && caps.rebase_in_progress { rebase::skip_rebase(self.state.clone()); }
        if do_abort {
            if caps.rebase_in_progress       { rebase::abort_rebase(self.state.clone()); }
            else if caps.merge_in_progress   { merge::abort_merge(self.state.clone()); }
            else if caps.cherry_pick_in_progress { branches::abort_cherry_pick(self.state.clone()); }
            else if caps.revert_in_progress  { branches::abort_revert(self.state.clone()); }
        }
    }

    fn apply_runner_progress(&mut self) {
        let events = self.runner.drain_progress();
        if events.is_empty() { return; }

        let mut state = self.state.lock().unwrap();
        for event in events {
            match event {
                JobProgress::Started { id, label, cancel } => {
                    state.console_log.push(format!("$ {} (job #{})", label.trim_end_matches('…'), id));
                    state.active_ops.push(ActiveOp {
                        id,
                        label,
                        progress_pct: None,
                        progress_msg: String::new(),
                        cancel,
                    });
                }
                JobProgress::Update { id, message, pct } => {
                    if let Some(op) = state.active_ops.iter_mut().find(|o| o.id == id) {
                        op.progress_msg = message;
                        op.progress_pct = pct;
                    }
                }
                JobProgress::Done { id } => {
                    let label = state.active_ops.iter().find(|o| o.id == id)
                        .map(|o| o.label.trim_end_matches('…').to_string())
                        .unwrap_or_else(|| format!("job #{}", id));
                    state.active_ops.retain(|o| o.id != id);
                    state.console_log.push(format!("{} done", label));
                    // Trigger a repo reload so the graph refreshes.
                    if state.repo_path.is_some() {
                        drop(state);
                        crate::git::branches::reload_repo(self.state.clone());
                        return;
                    }
                }
                JobProgress::Failed { id, error } => {
                    let label = state.active_ops.iter().find(|o| o.id == id)
                        .map(|o| o.label.trim_end_matches('…').to_string())
                        .unwrap_or_else(|| format!("job #{}", id));
                    state.active_ops.retain(|o| o.id != id);
                    let msg = error.user_message();
                    state.console_log.push(format!("error: {} — {}", label, msg));
                    state.error = Some(msg);
                }
            }
        }
    }

    fn show_operation_overlays(&mut self, ctx: &Context) {
        let (legacy_op, active_ops, error) = {
            let s = self.state.lock().unwrap();
            (
                s.remote_op.clone(),
                s.active_ops.iter().map(|o| {
                    let pct = o.progress_pct;
                    let msg = if o.progress_msg.is_empty() { o.label.clone() } else { o.progress_msg.clone() };
                    (o.id, msg, pct, o.cancel.clone())
                }).collect::<Vec<_>>(),
                s.error.clone(),
            )
        };

        // Runner-dispatched ops (one overlay per active job)
        for (i, (id, msg, pct, cancel)) in active_ops.iter().enumerate() {
            let y_offset = 104.0 + i as f32 * 52.0;
            ctx.request_repaint_after(std::time::Duration::from_millis(120));
            egui::Area::new(egui::Id::new(format!("runner_op_{}", id)))
                .anchor(egui::Align2::CENTER_TOP, [0.0, y_offset])
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
                                ui.label(RichText::new(msg.as_str()).color(theme::TEXT_PRIMARY).size(13.0).strong());
                                if let Some(p) = pct {
                                    ui.add_space(6.0);
                                    ui.label(RichText::new(format!("{}%", p)).color(theme::TEXT_MUTED).size(12.0));
                                }
                                ui.add_space(10.0);
                                if ui.add(secondary_btn("Cancel")).clicked() {
                                    cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                                }
                            });
                        });
                });
        }

        // Legacy overlay for branch/staging ops (remote_op path)
        let has_legacy = legacy_op.is_some();
        if let Some(op) = legacy_op {
            let y_offset = 104.0 + active_ops.len() as f32 * 52.0;
            ctx.request_repaint_after(std::time::Duration::from_millis(120));
            egui::Area::new(egui::Id::new("operation_overlay"))
                .anchor(egui::Align2::CENTER_TOP, [0.0, y_offset])
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

        // ── Discard confirmation ──────────────────────────────────────────
        let pending = { self.state.lock().unwrap().pending_discard.clone() };
        if let Some(ref pd) = pending {
            let y_off = 146.0 + active_ops.len() as f32 * 52.0 + if has_legacy { 52.0 } else { 0.0 };
            let desc = match pd {
                PendingDiscard::File { path } => format!("Discard all changes to \"{}\"?", path.rsplit('/').next().unwrap_or(path)),
                PendingDiscard::Hunk { hunk_index } => format!("Discard hunk #{}? This cannot be undone.", hunk_index + 1),
                PendingDiscard::ResetHard => "Hard reset: discard the last commit AND all staged/unstaged changes? This cannot be undone.".to_string(),
            };
            let mut confirm = false;
            let mut cancel  = false;
            egui::Area::new(egui::Id::new("discard_confirm_overlay"))
                .anchor(egui::Align2::CENTER_TOP, [0.0, y_off])
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    egui::Frame::none()
                        .fill(Color32::from_rgb(74, 24, 28))
                        .stroke(Stroke::new(1.5, theme::ACCENT_RED))
                        .rounding(8.0)
                        .inner_margin(egui::Margin::symmetric(16.0, 12.0))
                        .show(ui, |ui| {
                            ui.set_max_width(640.0);
                            ui.horizontal_centered(|ui| {
                                ui.label(RichText::new(&desc).color(Color32::WHITE).size(12.5));
                                ui.add_space(12.0);
                                if ui.add(danger_btn("Discard")).clicked() { confirm = true; }
                                if ui.add(secondary_btn("Cancel")).clicked() { cancel = true; }
                            });
                        });
                });
            if confirm {
                match pd {
                    PendingDiscard::File { path } => staging::discard_file(path, self.state.clone()),
                    PendingDiscard::Hunk { hunk_index } => staging::discard_hunk(*hunk_index, self.state.clone()),
                    PendingDiscard::ResetHard => staging::reset_hard(self.state.clone()),
                }
            } else if cancel {
                self.state.lock().unwrap().pending_discard = None;
            }
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
                        if ui.button("Open Repository…").clicked() {
                            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                                self.config.add_recent(path.clone());
                                load_repo(path, self.state.clone());
                            }
                            ui.close_menu();
                        }
                        if ui.button("Clone Repository…").clicked() {
                            self.state.lock().unwrap().modal = ModalKind::CloneRepo;
                            ui.close_menu();
                        }
                        if ui.button("Initialize Repository…").clicked() {
                            self.state.lock().unwrap().modal = ModalKind::InitRepo;
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
                        if ui.add_enabled(has_repo, egui::Button::new("Open Terminal")).clicked() {
                            if let Some(rp) = self.state.lock().unwrap().repo_path.clone() {
                                lifecycle::open_terminal(&rp);
                            }
                            ui.close_menu();
                        }
                        if ui.add_enabled(has_repo, egui::Button::new("Reveal in File Manager")).clicked() {
                            if let Some(rp) = self.state.lock().unwrap().repo_path.clone() {
                                lifecycle::reveal_in_file_manager(&rp);
                            }
                            ui.close_menu();
                        }
                        if ui.add_enabled(has_repo, egui::Button::new("Copy Path")).clicked() {
                            let path_str = self.state.lock().unwrap().repo_path
                                .as_ref().map(|p| p.display().to_string()).unwrap_or_default();
                            ctx.output_mut(|o| o.copied_text = path_str);
                            ui.close_menu();
                        }
                        if ui.add_enabled(has_repo, egui::Button::new("Pull Requests…")).clicked() {
                            self.pull_requests_open = true;
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.add_enabled(has_repo, egui::Button::new("Fetch")).clicked() {
                            remote::fetch_all(&mut self.runner, &self.state);
                            ui.close_menu();
                        }
                        if ui.add_enabled(has_repo, egui::Button::new("Pull")).clicked() {
                            remote::pull_branch(&mut self.runner, &self.state);
                            ui.close_menu();
                        }
                        if ui.add_enabled(has_repo, egui::Button::new("Push")).clicked() {
                            remote::push_branch(&mut self.runner, &self.state);
                            ui.close_menu();
                        }
                        if ui.add_enabled(has_repo, egui::Button::new("Refresh")).clicked() {
                            if let Some(path) = self.state.lock().unwrap().repo_path.clone() {
                                load_repo(path, self.state.clone());
                            }
                            ui.close_menu();
                        }
                        ui.separator();
                        let has_diff = self.state.lock().unwrap().current_diff.is_some();
                        if ui.add_enabled(has_diff, egui::Button::new("Export Patch…")).clicked() {
                            let diff = self.state.lock().unwrap().current_diff.clone();
                            if let Some(ref d) = diff {
                                let name = d.file_path.replace('/', "_");
                                if let Some(dest) = rfd::FileDialog::new()
                                    .add_filter("Patch", &["patch", "diff"])
                                    .set_file_name(&format!("{}.patch", name))
                                    .save_file()
                                {
                                    if let Err(e) = crate::git::patch::export_patch(d, &dest) {
                                        self.state.lock().unwrap().error = Some(e);
                                    }
                                }
                            }
                            ui.close_menu();
                        }
                        if ui.add_enabled(has_repo, egui::Button::new("Apply Patch File…")).clicked() {
                            let repo_path = self.state.lock().unwrap().repo_path.clone();
                            if let Some(ref rp) = repo_path {
                                if let Some(patch_path) = rfd::FileDialog::new()
                                    .add_filter("Patch", &["patch", "diff"])
                                    .pick_file()
                                {
                                    match crate::git::patch::import_patch_file(rp, &patch_path) {
                                        Ok(()) => load_repo(rp.clone(), self.state.clone()),
                                        Err(e) => self.state.lock().unwrap().error = Some(e.user_message()),
                                    }
                                }
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
                        ui.separator();
                        if ui.add_enabled(has_repo, egui::Button::new("Merge Branch…")).clicked() {
                            self.state.lock().unwrap().modal = ModalKind::MergeBranch;
                            ui.close_menu();
                        }
                        if ui.add_enabled(has_repo, egui::Button::new("Rebase Onto…")).clicked() {
                            self.state.lock().unwrap().modal = ModalKind::RebaseOnto;
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.add_enabled(has_repo, egui::Button::new("Stash Changes…")).clicked() {
                            self.stash_message_input = "WIP".into();
                            self.state.lock().unwrap().modal = ModalKind::StashWithMessage;
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

fn health_row(ui: &mut egui::Ui, label: &str, value: &str) {
    health_row_color(ui, label, value, theme::TEXT_SECONDARY);
}

fn health_row_color(ui: &mut egui::Ui, label: &str, value: &str, color: egui::Color32) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{:<14}", label)).color(theme::TEXT_MUTED).size(11.5).monospace());
        ui.label(RichText::new(value).color(color).size(11.5));
    });
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
