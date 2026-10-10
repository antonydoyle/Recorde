use crate::ui::button::Button;
use egui::{Color32, Context, Key, RichText, Stroke};

use crate::auth::{ProviderKind, StoredAccount};
use crate::config::{Config, PullStrategy};
use crate::git::{
    branches, lifecycle, merge,
    operations::{load_ci_runs, load_issues, load_prs, load_releases, load_repo},
    rebase, remote,
    runner::GitRunner,
    staging,
};
use crate::providers::{IssueFilter, PrFilter};
use crate::state::{new_shared_state, ModalKind, PendingDiscard, RebaseEntry, SharedState};
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
pub enum RightTab {
    Changes,
    Details,
}

#[derive(PartialEq, Clone, Copy, Default)]
enum ProviderTab {
    #[default]
    Issues,
    Ci,
    Releases,
}

#[derive(PartialEq, Clone, Copy)]
enum SettingsTab {
    App,
    Ui,
    Accounts,
    Shortcuts,
    Repository,
    About,
}

enum AccountChange {
    Migrated,
    Saved(StoredAccount),
    Removed(StoredAccount),
}

fn account_task(
    work: impl FnOnce() -> Result<AccountChange, String> + Send + 'static,
) -> std::sync::mpsc::Receiver<Result<AccountChange, String>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(work());
    });
    rx
}

struct RepoUiSession {
    sparse_paths: String,
    right_tab: RightTab,
    last_auto_detail_commit: Option<String>,
    create_branch_name: String,
    create_branch_checkout: bool,
    create_tag_name: String,
    push_tag_after_create: bool,
    repo_git_name_input: String,
    repo_git_email_input: String,
    commit_signing: crate::ui::commit_signing::CommitSigning,
    pull_requests_open: bool,
    review_auto_refresh: bool,
    last_review_refresh: std::time::Instant,
    rebase_modal: Option<RebaseModal>,
    origin_url_input: String,
    remote_name_input: String,
    remote_rename_from: Option<String>,
    remote_delete_pending: Option<String>,
    rename_branch_input: String,
    tag_message_input: String,
    stash_message_input: String,
    stash_branch_input: String,
    reflog_recover_input: String,
    reflog_recover_oid: Option<String>,
    compare_file_selected: Option<String>,
    provider_open: bool,
    provider_tab: ProviderTab,
    issues_fetched: bool,
    ci_fetched: bool,
    releases_fetched: bool,
    branch_from_issue_input: String,
    branch_from_issue_number: Option<u32>,
    worktree_branch_input: String,
    remove_worktree_pending: Option<String>,
    conflict_editor: Option<crate::ui::conflict_editor::ConflictEditor>,
    pending_recovery_id: Option<String>,
    provider_editor: Option<crate::ui::provider_editor::ProviderEditor>,
}

impl Default for RepoUiSession {
    fn default() -> Self {
        Self {
            sparse_paths: String::new(),
            right_tab: RightTab::Changes,
            last_auto_detail_commit: None,
            create_branch_name: String::new(),
            create_branch_checkout: true,
            create_tag_name: String::new(),
            push_tag_after_create: false,
            repo_git_name_input: String::new(),
            repo_git_email_input: String::new(),
            commit_signing: Default::default(),
            pull_requests_open: false,
            review_auto_refresh: false,
            last_review_refresh: std::time::Instant::now(),
            rebase_modal: None,
            origin_url_input: String::new(),
            remote_name_input: String::new(),
            remote_rename_from: None,
            remote_delete_pending: None,
            rename_branch_input: String::new(),
            tag_message_input: String::new(),
            stash_message_input: "WIP".into(),
            stash_branch_input: String::new(),
            reflog_recover_input: String::new(),
            reflog_recover_oid: None,
            compare_file_selected: None,
            provider_open: false,
            provider_tab: ProviderTab::Issues,
            issues_fetched: false,
            ci_fetched: false,
            releases_fetched: false,
            branch_from_issue_input: String::new(),
            branch_from_issue_number: None,
            worktree_branch_input: String::new(),
            remove_worktree_pending: None,
            conflict_editor: None,
            pending_recovery_id: None,
            provider_editor: None,
        }
    }
}

pub struct RecordeApp {
    workspace_name: String,
    repo_ui: RepoUiSession,
    repo_sessions: std::collections::HashMap<std::path::PathBuf, RepoUiSession>,
    state: SharedState,
    runner: GitRunner,
    toolbar: Toolbar,
    sidebar: Sidebar,
    commit_graph_view: CommitGraphView,
    diff_view: DiffView,
    last_navigation_save: std::time::Instant,
    frame_metrics: crate::ui::frame_metrics::FrameMetrics,
    detail_panel: DetailPanel,
    staging_panel: StagingPanel,
    status_bar: StatusBar,
    config: Config,
    /// Pre-loaded texture of the app icon for rendering in the title bar.
    app_icon_texture: egui::TextureHandle,
    /// Tracks the last-seen mtimes of key .git ref files so external changes
    /// (tags created in a terminal, commits from another tool, etc.) are picked up.
    repo_watch: Option<crate::git::watch::RepoWatch>,
    was_focused: bool,
    last_auto_fetch: std::time::Instant,
    auto_fetch_repo: Option<std::path::PathBuf>,
    observed_repo: Option<std::path::PathBuf>,
    observed_provider: Option<(String, String)>,
    dark_theme: bool,
    high_contrast: bool,
    // Modal form inputs
    // New modals
    command_palette_open: bool,
    command_palette_query: String,
    palette_selected: usize,
    palette_focus: bool,
    shortcut_capture: Option<crate::commands::Command>,
    shortcut_error: Option<String>,
    branch_switcher_open: bool,
    branch_switcher_query: String,
    settings_open: bool,
    settings_tab: SettingsTab,
    git_name_input: String,
    git_email_input: String,
    git_version: String,
    ssh_diagnostics: Option<crate::auth::ssh::SshDiagnostics>,
    diagnostics_pending:
        Option<std::sync::mpsc::Receiver<(String, crate::auth::ssh::SshDiagnostics)>>,
    default_editor_input: String,
    // Interactive rebase
    // Lifecycle modals
    clone_url: String,
    clone_depth: u32,
    clone_dest: Option<std::path::PathBuf>,
    init_repo_path: Option<std::path::PathBuf>,
    // Branch/tag management
    // Stash
    // Phase 7: History/Review
    // Phase 13: Provider integrations
    // Phase 10: Auth & Accounts
    browser_login: crate::ui::oauth_login::BrowserLogin,
    add_account_open: bool,
    add_account_provider: ProviderKind,
    add_account_host: String,
    add_account_username: String,
    add_account_token: String,
    add_account_error: Option<String>,
    account_task: Option<std::sync::mpsc::Receiver<Result<AccountChange, String>>>,
    // Phase 14: External tool inputs
    terminal_input: String,
    diff_tool_input: String,
    merge_tool_input: String,
}

impl RecordeApp {
    pub fn new(cc: &eframe::CreationContext<'_>, initial_repo: Option<std::path::PathBuf>) -> Self {
        let config = Config::load();
        let dark_theme = config.dark_theme;
        let high_contrast = config.high_contrast;
        let colorblind_graph = config.colorblind_graph;
        let default_editor = config.default_editor.clone();
        let terminal = config.terminal.clone();
        let diff_tool = config.diff_tool.clone();
        let merge_tool = config.merge_tool.clone();

        cc.egui_ctx.set_fonts(crate::icons::font_definitions());

        theme::apply(&cc.egui_ctx, dark_theme, high_contrast);
        theme::apply_ui_scale(&cc.egui_ctx, config.font_scale);

        // Pre-load the app icon as a texture so the title bar can render it.
        let app_icon_texture = {
            let icon_bytes = include_bytes!("../assets/icon.png");
            let icon_data =
                eframe::icon_data::from_png_bytes(icon_bytes).expect("failed to decode app icon");
            let color_image = egui::ColorImage::from_rgba_unmultiplied(
                [icon_data.width as usize, icon_data.height as usize],
                &icon_data.rgba,
            );
            cc.egui_ctx
                .load_texture("app_icon", color_image, egui::TextureOptions::LINEAR)
        };

        let state = new_shared_state();
        for (path, settings) in &config.per_repo {
            if let Some(navigation) = &settings.navigation {
                let path = std::path::PathBuf::from(path);
                let path = path.canonicalize().unwrap_or(path);
                state
                    .lock()
                    .unwrap()
                    .repo_views
                    .insert(path.clone(), navigation.restored_view(&path));
            }
        }
        let runner = GitRunner::new();
        let to_open = initial_repo
            .or_else(|| config.active_repo.clone())
            .or_else(|| config.recent_repos.first().cloned());
        let initial_repo_config = to_open.as_ref().map(|p| config.get_repo_config(p));
        if let Some(path) = to_open {
            load_repo(path, state.clone());
        }

        Self {
            workspace_name: String::new(),
            repo_ui: RepoUiSession::default(),
            repo_sessions: Default::default(),
            runner,
            toolbar: Toolbar::new(state.clone()),
            sidebar: Sidebar::new(state.clone()),
            commit_graph_view: {
                let mut cgv = CommitGraphView::new(state.clone());
                if let Some(rc) = initial_repo_config {
                    cgv.set_column_widths(rc.refs_w, rc.graph_w, rc.time_w, rc.hash_w);
                }
                cgv.set_colorblind(colorblind_graph);
                cgv
            },
            diff_view: DiffView::new(state.clone()),
            last_navigation_save: std::time::Instant::now(),
            frame_metrics: Default::default(),
            detail_panel: DetailPanel::new(state.clone()),
            staging_panel: StagingPanel::new(state.clone()),
            status_bar: StatusBar::new(state.clone()),
            app_icon_texture,
            repo_watch: None,
            was_focused: true,
            last_auto_fetch: std::time::Instant::now(),
            auto_fetch_repo: None,
            observed_repo: None,
            observed_provider: None,
            config,
            dark_theme,
            high_contrast,
            state,
            command_palette_open: false,
            command_palette_query: String::new(),
            palette_selected: 0,
            palette_focus: true,
            shortcut_capture: None,
            shortcut_error: None,
            branch_switcher_open: false,
            branch_switcher_query: String::new(),
            settings_open: false,
            settings_tab: SettingsTab::App,
            git_name_input: String::new(),
            git_email_input: String::new(),
            git_version: "Checking...".into(),
            ssh_diagnostics: None,
            diagnostics_pending: None,
            default_editor_input: default_editor,
            clone_url: String::new(),
            clone_depth: 0,
            clone_dest: None,
            init_repo_path: None,
            browser_login: Default::default(),
            add_account_open: false,
            add_account_provider: ProviderKind::GitHubCom,
            add_account_host: String::new(),
            add_account_username: String::new(),
            add_account_token: String::new(),
            add_account_error: None,
            account_task: Some(account_task(|| {
                crate::auth::store::migrate()?;
                Ok(AccountChange::Migrated)
            })),
            terminal_input: terminal,
            diff_tool_input: diff_tool,
            merge_tool_input: merge_tool,
        }
    }

    // ── Action handlers ───────────────────────────────────────────────────────

    fn handle_toolbar(&mut self, ctx: &Context, action: ToolbarAction) {
        if self.commit_graph_view.has_pending_action()
            || self.state.lock().unwrap().pending_discard.is_some()
        {
            return;
        }
        match action {
            ToolbarAction::OpenRepo(path) => {
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
                let path_str = self
                    .state
                    .lock()
                    .unwrap()
                    .repo_path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                ctx.output_mut(|o| o.copied_text = path_str);
            }
            ToolbarAction::Stash => {
                self.repo_ui.stash_message_input = "WIP".into();
                self.state.lock().unwrap().modal = ModalKind::StashWithMessage;
            }
            ToolbarAction::CreateBranch => {
                self.state.lock().unwrap().modal = ModalKind::CreateBranch { from_oid: None };
            }
            ToolbarAction::Fetch => remote::fetch_all(&mut self.runner, &self.state),
            ToolbarAction::Pull => remote::pull_branch(&mut self.runner, &self.state),
            ToolbarAction::PullWithRebase => {
                remote::pull_with_rebase(&mut self.runner, &self.state)
            }
            ToolbarAction::PullFfOnly => remote::pull_ff_only(&mut self.runner, &self.state),
            ToolbarAction::Push => remote::push_branch(&mut self.runner, &self.state),
            ToolbarAction::PushWithLease => remote::push_with_lease(&mut self.runner, &self.state),
            ToolbarAction::PushSetUpstream => {
                remote::push_set_upstream(&mut self.runner, &self.state)
            }
            ToolbarAction::MergeBranch => {
                self.state.lock().unwrap().modal = ModalKind::MergeBranch;
            }
            ToolbarAction::RebaseOnto => {
                self.state.lock().unwrap().modal = ModalKind::RebaseOnto;
            }
            ToolbarAction::ToggleTheme => {
                self.dark_theme = !self.dark_theme;
                self.config.dark_theme = self.dark_theme;
                self.save_config();
                theme::apply(ctx, self.dark_theme, self.high_contrast);
            }
            ToolbarAction::ToggleSearch => {
                let mut s = self.state.lock().unwrap();
                s.search_active = !s.search_active;
                if !s.search_active {
                    s.search_query.clear();
                }
            }
            ToolbarAction::OpenCommandPalette => {
                self.command_palette_open = !self.command_palette_open;
                self.palette_focus = true;
            }
            ToolbarAction::OpenBranchSwitcher => self.branch_switcher_open = true,
            ToolbarAction::OpenUserMenu => self.open_settings(SettingsTab::Accounts),
            ToolbarAction::ToggleAllBranches => {
                let mut s = self.state.lock().unwrap();
                s.show_all_branches = !s.show_all_branches;
                s.graph_filter = Default::default();
                drop(s);
                crate::git::operations::refresh_repo(self.state.clone());
            }
            ToolbarAction::OpenReflog => self.state.lock().unwrap().modal = ModalKind::Reflog,
            ToolbarAction::OpenBisect => self.state.lock().unwrap().modal = ModalKind::Bisect,
            ToolbarAction::OpenProvider => {
                self.repo_ui.provider_open = true;
                self.repo_ui.issues_fetched = false;
                self.repo_ui.ci_fetched = false;
                self.repo_ui.releases_fetched = false;
            }
        }
    }

    fn handle_sidebar(&mut self, action: SidebarAction) {
        if self.commit_graph_view.has_pending_action()
            || self.state.lock().unwrap().pending_discard.is_some()
        {
            return;
        }
        match action {
            SidebarAction::OpenRepository(path) => load_repo(path, self.state.clone()),
            SidebarAction::CheckoutBranch(n) => branches::checkout_branch(&n, self.state.clone()),
            SidebarAction::OpenCreateBranch { from_oid } => {
                self.state.lock().unwrap().modal = ModalKind::CreateBranch { from_oid }
            }
            SidebarAction::DeleteBranch(n) => {
                self.state.lock().unwrap().modal = ModalKind::ConfirmDeleteBranch { branch_name: n }
            }
            SidebarAction::CreateTag(oid) => {
                let from = if oid.is_empty() { None } else { Some(oid) };
                self.state.lock().unwrap().modal = ModalKind::CreateTag { from_oid: from };
            }
            SidebarAction::DeleteTag(n) => {
                self.state.lock().unwrap().modal = ModalKind::ConfirmDeleteTag { tag_name: n }
            }
            SidebarAction::StashApply(i) => branches::stash_apply(i, self.state.clone()),
            SidebarAction::StashPop(i) => branches::stash_pop(i, self.state.clone()),
            SidebarAction::StashDrop(i) => branches::stash_drop(i, self.state.clone()),
            SidebarAction::MergeBranch(n) => merge::merge_branch(&n, self.state.clone()),
            SidebarAction::RebaseOnto(n) => rebase::rebase_onto(&n, self.state.clone()),
            SidebarAction::RenameBranch(n) => {
                self.state.lock().unwrap().modal = ModalKind::RenameBranch { old_name: n };
            }
            SidebarAction::PublishBranch(n) => branches::publish_branch(&n, self.state.clone()),
            SidebarAction::SetUpstream(n, target) => {
                branches::set_upstream(&n, &target, self.state.clone())
            }
            SidebarAction::ForceDeleteBranch(n) => {
                branches::force_delete_branch(&n, self.state.clone())
            }
            SidebarAction::PushTag(n) => branches::push_tag(&n, self.state.clone()),
            SidebarAction::DeleteRemoteTag(n) => {
                branches::delete_remote_tag(&n, self.state.clone())
            }
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
                self.repo_ui.stash_branch_input.clear();
                self.state.lock().unwrap().modal =
                    ModalKind::CreateBranchFromStash { stash_index: idx };
            }
            SidebarAction::RestoreBranch(name, oid) => {
                branches::restore_deleted_branch(&name, &oid, self.state.clone())
            }
        }
    }

    fn open_settings(&mut self, tab: SettingsTab) {
        self.settings_tab = tab;
        self.settings_open = true;
        self.load_git_identity_inputs();
        self.load_app_settings_inputs();
        if self.diagnostics_pending.is_none() {
            let (tx, rx) = std::sync::mpsc::channel();
            self.diagnostics_pending = Some(rx);
            std::thread::spawn(move || {
                let mut command = crate::git::process::background_command("git");
                command.arg("--version");
                let version = crate::git::process::run(
                    &mut command,
                    &std::sync::atomic::AtomicBool::new(false),
                    std::time::Duration::from_secs(10),
                )
                .ok()
                .filter(|out| out.status.success())
                .and_then(|out| String::from_utf8(out.stdout).ok())
                .unwrap_or_else(|| "unavailable".into());
                let _ = tx.send((version, crate::auth::ssh::diagnose()));
            });
        }
    }

    fn load_app_settings_inputs(&mut self) {
        self.terminal_input = self.config.terminal.clone();
        self.diff_tool_input = self.config.diff_tool.clone();
        self.merge_tool_input = self.config.merge_tool.clone();
    }

    fn load_git_identity_inputs(&mut self) {
        let global = git_identity(None);
        self.git_name_input = global.0;
        self.git_email_input = global.1;

        let repo_path = self.state.lock().unwrap().repo_path.clone();
        let repo = git_identity(repo_path.as_deref());
        self.repo_ui.repo_git_name_input = repo.0;
        self.repo_ui.repo_git_email_input = repo.1;
        if let Some(path) = repo_path {
            self.repo_ui.commit_signing.reload(path);
        }
    }

    // ── Open rebase modal ─────────────────────────────────────────────────────

    fn open_rebase_modal(&mut self, upstream_oid: String) {
        let repo_path = self.state.lock().unwrap().repo_path.clone();
        let Some(path) = repo_path else { return };
        let upstream_summary = (|| -> Option<String> {
            let repo = git2::Repository::open(&path).ok()?;
            let oid = git2::Oid::from_str(&upstream_oid).ok()?;
            let commit = repo.find_commit(oid).ok()?;
            commit.summary().ok().flatten().map(str::to_owned)
        })()
        .unwrap_or_default();
        let entries = rebase::load_rebase_entries(&path, &upstream_oid);
        match entries {
            Some(e) if !e.is_empty() => {
                let preserve = git2::Repository::open(&path).is_ok_and(|repo| {
                    e.iter().any(|entry| {
                        git2::Oid::from_str(&entry.oid)
                            .ok()
                            .and_then(|oid| repo.find_commit(oid).ok())
                            .is_some_and(|c| c.parent_count() > 1)
                    })
                });
                let mut modal = RebaseModal::new(upstream_oid, upstream_summary, e);
                modal.preserve_merges = preserve;
                self.repo_ui.rebase_modal = Some(modal);
            }
            _ => {
                self.state.lock().unwrap().error =
                    Some("No commits to rebase between this commit and HEAD.".into())
            }
        }
    }

    // ── Modal rendering ───────────────────────────────────────────────────────

    fn render_modals(&mut self, ctx: &Context) {
        if self.commit_graph_view.has_pending_action() {
            return;
        }
        let modal_kind = self.state.lock().unwrap().modal.clone();
        match modal_kind {
            ModalKind::None => {}

            ModalKind::CreateBranch { from_oid } => modal_window(ctx, "Create Branch", |ui| {
                ui.label(styled_label("Branch name"));
                ui.add_space(4.0);
                let r = ui.add(
                    egui::TextEdit::singleline(&mut self.repo_ui.create_branch_name)
                        .desired_width(f32::INFINITY)
                        .margin(egui::Vec2::new(8.0, 5.0))
                        .hint_text("e.g. feat/my-feature"),
                );
                r.request_focus();
                if let Some(ref oid) = from_oid {
                    ui.add_space(4.0);
                    ui.label(styled_muted(&format!("From  {}", &oid[..7.min(oid.len())])));
                }
                ui.add_space(8.0);
                ui.checkbox(
                    &mut self.repo_ui.create_branch_checkout,
                    styled_label("Checkout immediately"),
                );
                ui.add_space(16.0);
                let can = !self.repo_ui.create_branch_name.trim().is_empty();
                ui.horizontal(|ui| {
                    let ok = ui.add_enabled(can, primary_btn("Create Branch")).clicked()
                        || (can && ui.input(|i| i.key_pressed(Key::Enter)));
                    let cancel = ui.add(secondary_btn("Cancel")).clicked()
                        || ui.input(|i| i.key_pressed(Key::Escape));
                    if ok {
                        branches::create_branch(
                            self.repo_ui.create_branch_name.trim(),
                            from_oid.as_deref(),
                            self.repo_ui.create_branch_checkout,
                            self.state.clone(),
                        );
                        self.state.lock().unwrap().modal = ModalKind::None;
                        self.repo_ui.create_branch_name.clear();
                    } else if cancel {
                        self.state.lock().unwrap().modal = ModalKind::None;
                        self.repo_ui.create_branch_name.clear();
                    }
                });
            }),

            ModalKind::ConfirmDeleteBranch { branch_name } => {
                modal_window(ctx, "Delete Branch", |ui| {
                    ui.label(
                        RichText::new(format!("Delete \"{}\"?", branch_name))
                            .color(theme::text_primary())
                            .size(14.0),
                    );
                    ui.label(styled_muted("Unmerged commits will be lost."));
                    ui.add_space(16.0);
                    ui.horizontal(|ui| {
                        let ok = ui.add(danger_btn("Delete")).clicked();
                        let cancel = ui.add(secondary_btn("Cancel")).clicked()
                            || ui.input(|i| i.key_pressed(Key::Escape));
                        if ok {
                            branches::delete_branch(&branch_name, self.state.clone());
                            self.state.lock().unwrap().modal = ModalKind::None;
                        }
                        if cancel {
                            self.state.lock().unwrap().modal = ModalKind::None;
                        }
                    });
                })
            }

            ModalKind::CreateTag { from_oid } => modal_window(ctx, "Create Tag", |ui| {
                ui.label(styled_label("Tag name"));
                ui.add_space(4.0);
                let r = ui.add(
                    egui::TextEdit::singleline(&mut self.repo_ui.create_tag_name)
                        .desired_width(f32::INFINITY)
                        .margin(egui::Vec2::new(8.0, 5.0))
                        .hint_text("e.g. v1.0.0"),
                );
                r.request_focus();
                if let Some(ref oid) = from_oid {
                    ui.add_space(4.0);
                    ui.label(styled_muted(&format!(
                        "At commit  {}",
                        &oid[..7.min(oid.len())]
                    )));
                }
                ui.add_space(8.0);
                ui.label(styled_label(
                    "Message (optional — leave blank for lightweight tag)",
                ));
                ui.add_space(4.0);
                ui.add(
                    egui::TextEdit::multiline(&mut self.repo_ui.tag_message_input)
                        .desired_width(f32::INFINITY)
                        .desired_rows(2)
                        .hint_text("Annotated tag message…"),
                );
                ui.add_space(8.0);
                ui.checkbox(
                    &mut self.repo_ui.push_tag_after_create,
                    RichText::new("Push to remote after creating")
                        .color(theme::text_secondary())
                        .size(13.0),
                );
                ui.add_space(8.0);
                let can = !self.repo_ui.create_tag_name.trim().is_empty();
                ui.horizontal(|ui| {
                    let ok = ui.add_enabled(can, primary_btn("Create Tag")).clicked()
                        || (can && ui.input(|i| i.key_pressed(Key::Enter)));
                    let cancel = ui.add(secondary_btn("Cancel")).clicked()
                        || ui.input(|i| i.key_pressed(Key::Escape));
                    if ok {
                        let tag_name = self.repo_ui.create_tag_name.trim().to_string();
                        let msg = if self.repo_ui.tag_message_input.trim().is_empty() {
                            None
                        } else {
                            Some(self.repo_ui.tag_message_input.trim())
                        };
                        branches::create_tag(
                            &tag_name,
                            from_oid.as_deref(),
                            msg,
                            self.state.clone(),
                        );
                        if self.repo_ui.push_tag_after_create {
                            branches::push_tag(&tag_name, self.state.clone());
                        }
                        self.state.lock().unwrap().modal = ModalKind::None;
                        self.repo_ui.create_tag_name.clear();
                        self.repo_ui.tag_message_input.clear();
                    }
                    if cancel {
                        self.state.lock().unwrap().modal = ModalKind::None;
                        self.repo_ui.create_tag_name.clear();
                        self.repo_ui.tag_message_input.clear();
                    }
                });
            }),

            ModalKind::ConfirmDeleteTag { tag_name } => modal_window(ctx, "Delete Tag", |ui| {
                ui.label(
                    RichText::new(format!("Delete tag \"{}\"?", tag_name))
                        .color(theme::text_primary())
                        .size(14.0),
                );
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    let ok = ui.add(danger_btn("Delete")).clicked();
                    let cancel = ui.add(secondary_btn("Cancel")).clicked()
                        || ui.input(|i| i.key_pressed(Key::Escape));
                    if ok {
                        branches::delete_tag(&tag_name, self.state.clone());
                        self.state.lock().unwrap().modal = ModalKind::None;
                    }
                    if cancel {
                        self.state.lock().unwrap().modal = ModalKind::None;
                    }
                });
            }),

            ModalKind::CloneRepo => self.render_clone_modal(ctx),
            ModalKind::InitRepo => self.render_init_modal(ctx),
            ModalKind::RenameBranch { old_name } => self.render_rename_branch_modal(ctx, old_name),
            ModalKind::ConfirmDetachedCheckout { oid } => {
                self.render_detached_checkout_modal(ctx, oid)
            }
            ModalKind::StashWithMessage => self.render_stash_message_modal(ctx),
            ModalKind::CreateBranchFromStash { stash_index } => {
                self.render_stash_branch_modal(ctx, stash_index)
            }

            ModalKind::MergeBranch => self.render_branch_picker(
                ctx,
                "Merge Branch",
                "Select a branch to merge into the current branch:",
                |app, branch| {
                    merge::merge_branch(&branch, app.state.clone());
                },
            ),

            ModalKind::RebaseOnto => self.render_branch_picker(
                ctx,
                "Rebase Onto",
                "Select a branch to rebase current onto:",
                |app, branch| {
                    rebase::rebase_onto(&branch, app.state.clone());
                },
            ),

            ModalKind::Reflog => self.render_reflog_modal(ctx),
            ModalKind::Bisect => self.render_bisect_modal(ctx),
            ModalKind::Compare {
                base_oid,
                compare_oid,
            } => self.render_compare_modal(ctx, base_oid, compare_oid),
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

        if self.repo_ui.pull_requests_open {
            self.render_pull_requests(ctx);
        }

        if self.repo_ui.provider_open {
            self.render_provider(ctx);
        }
    }

    fn dispatch_command(&mut self, ctx: &Context, command: crate::commands::Command) {
        use crate::commands::Command as C;
        if self.commit_graph_view.has_pending_action()
            || !command.available(&self.state.lock().unwrap())
        {
            return;
        }
        let toolbar = match command {
            C::PreviewCherryPick => {
                self.commit_graph_view
                    .preview_selected(crate::git::graph_action::Action::CherryPick);
                None
            }
            C::PreviewGraphRebase => {
                self.commit_graph_view
                    .preview_selected(crate::git::graph_action::Action::Rebase);
                None
            }
            C::Palette => Some(ToolbarAction::OpenCommandPalette),
            C::Open => {
                if let Some(path) = rfd::FileDialog::new().pick_folder() {
                    self.handle_toolbar(ctx, ToolbarAction::OpenRepo(path));
                }
                None
            }
            C::Clone => Some(ToolbarAction::CloneRepo),
            C::Init => Some(ToolbarAction::InitRepo),
            C::Settings => {
                self.open_settings(SettingsTab::App);
                None
            }
            C::Shortcuts => {
                self.open_settings(SettingsTab::Shortcuts);
                None
            }
            C::Accounts => Some(ToolbarAction::OpenUserMenu),
            C::Theme => Some(ToolbarAction::ToggleTheme),
            C::Fetch => Some(ToolbarAction::Fetch),
            C::Pull => Some(ToolbarAction::Pull),
            C::PullRebase => Some(ToolbarAction::PullWithRebase),
            C::PullFf => Some(ToolbarAction::PullFfOnly),
            C::Push => Some(ToolbarAction::Push),
            C::PushLease => Some(ToolbarAction::PushWithLease),
            C::PushUpstream => Some(ToolbarAction::PushSetUpstream),
            C::Branch => Some(ToolbarAction::CreateBranch),
            C::SwitchBranch => Some(ToolbarAction::OpenBranchSwitcher),
            C::Merge => Some(ToolbarAction::MergeBranch),
            C::Rebase => Some(ToolbarAction::RebaseOnto),
            C::Stash => Some(ToolbarAction::Stash),
            C::Search => Some(ToolbarAction::ToggleSearch),
            C::DiffSearch => {
                self.focus_pane(4);
                self.diff_view.open_search();
                None
            }
            C::AllBranches => Some(ToolbarAction::ToggleAllBranches),
            C::Reflog => Some(ToolbarAction::OpenReflog),
            C::Bisect => Some(ToolbarAction::OpenBisect),
            C::Provider => Some(ToolbarAction::OpenProvider),
            C::Terminal => Some(ToolbarAction::OpenTerminal),
            C::Reveal => Some(ToolbarAction::RevealInFolder),
            C::CopyPath => Some(ToolbarAction::CopyRepoPath),
            C::Refresh => {
                crate::git::operations::refresh_repo(self.state.clone());
                None
            }
            C::Sidebar => {
                self.focus_pane(1);
                None
            }
            C::Graph => {
                self.focus_pane(2);
                None
            }
            C::Changes => {
                self.repo_ui.right_tab = RightTab::Changes;
                self.focus_pane(3);
                None
            }
            C::Diff => {
                self.focus_pane(4);
                None
            }
            C::NewTag => {
                let mut s = self.state.lock().unwrap();
                s.modal = ModalKind::CreateTag {
                    from_oid: s.selected_commit.clone(),
                };
                None
            }
            C::CopyCommit => {
                if let Some(oid) = self.state.lock().unwrap().selected_commit.clone() {
                    ctx.output_mut(|o| o.copied_text = oid);
                }
                None
            }
            C::InteractiveRebase => {
                let mut s = self.state.lock().unwrap();
                s.pending_open_rebase = s.selected_commit.clone();
                None
            }
        };
        if let Some(action) = toolbar {
            self.handle_toolbar(ctx, action);
        }
    }

    fn render_command_palette(&mut self, ctx: &Context) {
        use crate::commands::Command;
        let mut chosen = None;
        let mut close = false;
        egui::Window::new("Command palette")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_TOP, [0.0, 60.0])
            .show(ctx, |ui| {
                ui.set_min_width(500.0);
                let input = ui.add(
                    egui::TextEdit::singleline(&mut self.command_palette_query)
                        .hint_text("Search commands (Up/Down, Enter, Escape)")
                        .desired_width(f32::INFINITY),
                );
                if self.palette_focus {
                    input.request_focus();
                    self.palette_focus = false;
                }
                if input.changed() {
                    self.palette_selected = 0;
                }
                let query = self.command_palette_query.to_lowercase();
                let actions: Vec<_> = Command::ALL
                    .iter()
                    .copied()
                    .filter(|c| *c != Command::Palette && c.label().to_lowercase().contains(&query))
                    .collect();
                self.palette_selected = self.palette_selected.min(actions.len().saturating_sub(1));
                let down = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::ArrowDown));
                let up = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::ArrowUp));
                if !actions.is_empty() {
                    if down {
                        self.palette_selected = (self.palette_selected + 1) % actions.len();
                    }
                    if up {
                        self.palette_selected =
                            (self.palette_selected + actions.len() - 1) % actions.len();
                    }
                    if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter)) {
                        chosen = Some(actions[self.palette_selected]);
                    }
                }
                egui::ScrollArea::vertical()
                    .max_height(400.0)
                    .show(ui, |ui| {
                        for (index, command) in actions.iter().enumerate() {
                            let reason = command.disabled_reason(&self.state.lock().unwrap());
                            let shortcut = self
                                .config
                                .shortcuts
                                .get(*command)
                                .map(|b| b.label())
                                .unwrap_or_default();
                            let label = format!("{}    {}", command.label(), shortcut);
                            let response = ui.add_enabled(
                                reason.is_none(),
                                egui::SelectableLabel::new(index == self.palette_selected, label),
                            );
                            if (up || down) && index == self.palette_selected {
                                response.scroll_to_me(Some(egui::Align::Center));
                            }
                            if response.clicked() {
                                chosen = Some(*command);
                            }
                            if let Some(reason) = reason {
                                response.on_hover_text(reason);
                                if index == self.palette_selected {
                                    ui.label(RichText::new(reason).color(theme::text_muted()));
                                }
                            }
                        }
                        if actions.is_empty() {
                            ui.label("No matching commands");
                        }
                    });
                close = ui.button("Close (Escape)").clicked()
                    || ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape));
            });
        if let Some(command) = chosen {
            if command.available(&self.state.lock().unwrap()) {
                close = true;
            } else {
                chosen = None;
            }
        }
        if close {
            self.command_palette_open = false;
            self.command_palette_query.clear();
            self.palette_focus = true;
        }
        if let Some(command) = chosen {
            self.dispatch_command(ctx, command);
        }
    }

    fn render_shortcut_settings(&mut self, ui: &mut egui::Ui) {
        ui.label("Choose Change, then press a key with Ctrl (Command on macOS). Escape cancels recording. Shortcuts pause while editing text or using a dialog.");
        if ui.button("Reset all shortcuts to defaults").clicked() {
            self.config.shortcuts = Default::default();
            self.shortcut_capture = None;
            self.shortcut_error = None;
            self.save_config();
        }
        if let Some(command) = self.shortcut_capture {
            ui.label(format!("Recording shortcut for {}", command.label()));
            let events = ui.input(|i| i.events.clone());
            for event in events {
                if let egui::Event::Key {
                    key,
                    pressed: true,
                    repeat: false,
                    modifiers,
                    ..
                } = event
                {
                    if key == Key::Escape {
                        self.shortcut_capture = None;
                    } else if let Some(binding) =
                        crate::commands::Shortcut::from_event(key, modifiers)
                    {
                        match self.config.shortcuts.set(command, Some(binding)) {
                            Ok(()) => {
                                self.shortcut_capture = None;
                                self.shortcut_error = None;
                                self.save_config();
                            }
                            Err(error) => self.shortcut_error = Some(error),
                        }
                    }
                }
            }
        }
        if let Some(error) = &self.shortcut_error {
            ui.colored_label(theme::accent_red(), error);
        }
        for &command in crate::commands::Command::ALL {
            ui.push_id(command.id(), |ui| {
                ui.horizontal(|ui| {
                    ui.label(command.label());
                    ui.label(
                        self.config
                            .shortcuts
                            .get(command)
                            .map(|b| b.label())
                            .unwrap_or_else(|| "Unassigned".into()),
                    );
                    if ui.button("Change").clicked() {
                        self.shortcut_capture = Some(command);
                        self.shortcut_error = None;
                    }
                    if ui.button("Clear").clicked() {
                        let _ = self.config.shortcuts.set(command, None);
                        self.shortcut_capture = None;
                        self.save_config();
                    }
                });
            });
        }
    }

    fn render_branch_switcher(&mut self, ctx: &Context) {
        let branches_list: Vec<String> = {
            let s = self.state.lock().unwrap();
            s.commit_graph
                .as_ref()
                .map(|g| {
                    let mut refs: Vec<&str> = g
                        .nodes
                        .iter()
                        .flat_map(|n| n.refs.iter().map(|r| r.as_str()))
                        .collect();
                    refs.sort_unstable();
                    refs.dedup();
                    refs.iter()
                        .filter(|r| !r.contains('/'))
                        .map(|s| s.to_string())
                        .collect()
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
                ui.label(
                    RichText::new("Switch Branch")
                        .color(theme::text_primary())
                        .size(15.0)
                        .strong(),
                );
                ui.add_space(10.0);
                let r = ui.add(
                    egui::TextEdit::singleline(&mut self.branch_switcher_query)
                        .desired_width(f32::INFINITY)
                        .margin(egui::Vec2::new(8.0, 5.0))
                        .hint_text("Filter branches…"),
                );
                r.request_focus();
                ui.add_space(6.0);

                let q = self.branch_switcher_query.to_lowercase();
                egui::ScrollArea::vertical()
                    .max_height(280.0)
                    .show(ui, |ui| {
                        for name in branches_list
                            .iter()
                            .filter(|n| q.is_empty() || n.to_lowercase().contains(&q))
                        {
                            let r = ui.selectable_label(
                                false,
                                RichText::new(name)
                                    .color(theme::text_secondary())
                                    .size(13.0),
                            );
                            if r.clicked() {
                                checkout = Some(name.clone());
                            }
                        }
                    });

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if let Some(ref name) = checkout {
                        let _ = name; // handled below
                    }
                    if ui.add(secondary_btn("Cancel")).clicked()
                        || ui.input(|i| i.key_pressed(Key::Escape))
                    {
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
        if let Some(pending) = &self.diagnostics_pending {
            match pending.try_recv() {
                Ok((version, ssh)) => {
                    self.git_version = version;
                    self.ssh_diagnostics = Some(ssh);
                    self.diagnostics_pending = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.diagnostics_pending = None;
                    self.git_version = "unavailable".into();
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100))
                }
            }
        }
        egui::Window::new("Settings")
            .collapsible(false)
            .resizable(true)
            .default_width(680.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(egui::Frame::window(&ctx.style()).inner_margin(egui::Margin::same(20.0)))
            .show(ctx, |ui| {
                ui.set_min_size(egui::vec2(520.0, 360.0));
                ui.label(
                    RichText::new("Settings")
                        .color(theme::text_primary())
                        .size(15.0)
                        .strong(),
                );
                ui.add_space(12.0);

                ui.horizontal(|ui| {
                    settings_tab_button(ui, &mut self.settings_tab, SettingsTab::App, "App");
                    settings_tab_button(ui, &mut self.settings_tab, SettingsTab::Ui, "Appearance");
                    settings_tab_button(
                        ui,
                        &mut self.settings_tab,
                        SettingsTab::Shortcuts,
                        "Keyboard",
                    );
                    settings_tab_button(
                        ui,
                        &mut self.settings_tab,
                        SettingsTab::Accounts,
                        "Accounts & SSH",
                    );
                    settings_tab_button(
                        ui,
                        &mut self.settings_tab,
                        SettingsTab::Repository,
                        "Repository",
                    );
                    settings_tab_button(ui, &mut self.settings_tab, SettingsTab::About, "About");
                });
                ui.add(egui::Separator::default().spacing(0.0));
                ui.add_space(8.0);

                egui::ScrollArea::vertical()
                    .id_source(("settings_content", self.settings_tab as u8))
                    .max_height((ctx.screen_rect().height() - 300.0).max(120.0))
                    .show(ui, |ui| match self.settings_tab {
                        SettingsTab::App => self.render_app_settings(ui),
                        SettingsTab::Ui => self.render_ui_settings(ctx, ui),
                        SettingsTab::Shortcuts => self.render_shortcut_settings(ui),
                        SettingsTab::Accounts => self.render_accounts_tab(ui),
                        SettingsTab::Repository => self.render_repo_settings(ui),
                        SettingsTab::About => self.render_about(ui),
                    });

                ui.add_space(16.0);
                if ui.add(secondary_btn("Close")).clicked()
                    || ui.input(|i| i.key_pressed(Key::Escape))
                {
                    self.settings_open = false;
                }
            });
    }

    fn render_ui_settings(&mut self, ctx: &Context, ui: &mut egui::Ui) {
        if ui
            .checkbox(
                &mut self.dark_theme,
                RichText::new("Dark theme").color(theme::text_secondary()),
            )
            .changed()
        {
            self.config.dark_theme = self.dark_theme;
            self.save_config();
            theme::apply(ctx, self.dark_theme, self.high_contrast);
        }
        if ui
            .checkbox(
                &mut self.high_contrast,
                RichText::new("High contrast").color(theme::text_secondary()),
            )
            .changed()
        {
            self.config.high_contrast = self.high_contrast;
            self.save_config();
            theme::apply(ctx, self.dark_theme, self.high_contrast);
        }

        // ── UI scale ─────────────────────────────────────────────────────────
        ui.add_space(12.0);
        ui.label(
            RichText::new("UI scale")
                .color(theme::text_secondary())
                .size(13.0)
                .strong(),
        );
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add(
                egui::Slider::new(&mut self.config.font_scale, 0.75..=2.0)
                    .step_by(0.25)
                    .text(""),
            );
            if ui.add(secondary_btn("Apply")).clicked() {
                theme::apply_ui_scale(ctx, self.config.font_scale);
                self.save_config();
            }
        });
        ui.label(styled_muted(
            "1.0 = your display’s native scale. Changes apply immediately.",
        ));

        // ── Accessibility ────────────────────────────────────────────────────
        ui.add_space(12.0);
        ui.label(
            RichText::new("Accessibility")
                .color(theme::text_secondary())
                .size(13.0)
                .strong(),
        );
        ui.add_space(4.0);
        if ui
            .checkbox(
                &mut self.config.colorblind_graph,
                RichText::new("Color-blind-safe graph colours").color(theme::text_secondary()),
            )
            .changed()
        {
            self.commit_graph_view
                .set_colorblind(self.config.colorblind_graph);
            self.save_config();
        }

        ui.add_space(8.0);
        ui.label(styled_muted(
            "Split diff view is controlled from the diff header.",
        ));
    }

    fn render_app_settings(&mut self, ui: &mut egui::Ui) {
        if let Some(error) = &self.config.load_error {
            ui.colored_label(theme::accent_red(), error);
            ui.separator();
        }
        ui.add_space(8.0);
        ui.label(
            RichText::new("Default editor")
                .color(theme::text_secondary())
                .size(13.0)
                .strong(),
        );
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_sized(
                [340.0, 28.0],
                egui::TextEdit::singleline(&mut self.default_editor_input)
                    .margin(egui::Vec2::new(8.0, 5.0))
                    .hint_text("code, codium, zed, vim, ..."),
            );
            if ui.add(secondary_btn("Save")).clicked() {
                self.config.default_editor = self.default_editor_input.trim().to_string();
                self.save_config();
            }
            if ui.add(secondary_btn("Open Repo")).clicked() {
                if let Err(e) = open_repo_in_editor(
                    self.state.lock().unwrap().repo_path.clone(),
                    &self.config.default_editor,
                ) {
                    self.state.lock().unwrap().error = Some(e);
                }
            }
        });
        ui.label(styled_muted("Used by Repository > Open in Editor. Arguments are supported, e.g. code --reuse-window."));

        // ── Pull strategy ────────────────────────────────────────────────────
        ui.add_space(12.0);
        ui.label(
            RichText::new("Default pull strategy")
                .color(theme::text_secondary())
                .size(13.0)
                .strong(),
        );
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui
                .radio_value(&mut self.config.pull_strategy, PullStrategy::Merge, "Merge")
                .changed()
            {
                self.save_config();
            }
            if ui
                .radio_value(
                    &mut self.config.pull_strategy,
                    PullStrategy::Rebase,
                    "Rebase",
                )
                .changed()
            {
                self.save_config();
            }
            if ui
                .radio_value(
                    &mut self.config.pull_strategy,
                    PullStrategy::FfOnly,
                    "Fast-forward only",
                )
                .changed()
            {
                self.save_config();
            }
        });
        ui.label(styled_muted("Used when pulling without an explicit flag."));

        // ── Fetch options ────────────────────────────────────────────────────
        ui.add_space(12.0);
        if ui
            .checkbox(
                &mut self.config.fetch_prune,
                RichText::new("Prune deleted remote branches on fetch")
                    .color(theme::text_secondary()),
            )
            .changed()
        {
            self.save_config();
        }
        if ui
            .checkbox(
                &mut self.config.auto_fetch,
                RichText::new("Auto-fetch on repo open").color(theme::text_secondary()),
            )
            .changed()
        {
            self.save_config();
        }
        ui.horizontal(|ui| {
            ui.add_enabled_ui(self.config.auto_fetch, |ui| {
                ui.label(RichText::new("Interval (seconds)").color(theme::text_secondary()));
                let mut interval_str = self.config.auto_fetch_interval_secs.to_string();
                if ui
                    .add_sized(
                        [80.0, 28.0],
                        egui::TextEdit::singleline(&mut interval_str)
                            .margin(egui::Vec2::new(8.0, 5.0)),
                    )
                    .changed()
                {
                    if let Ok(v) = interval_str.trim().parse::<u32>() {
                        self.config.auto_fetch_interval_secs = v;
                        self.save_config();
                    }
                }
            });
        });

        // ── External tools ───────────────────────────────────────────────────
        ui.add_space(12.0);
        ui.label(
            RichText::new("External tools")
                .color(theme::text_secondary())
                .size(13.0)
                .strong(),
        );
        ui.add_space(4.0);
        ui.label(RichText::new("Terminal").color(theme::text_secondary()));
        ui.horizontal(|ui| {
            ui.add_sized(
                [340.0, 28.0],
                egui::TextEdit::singleline(&mut self.terminal_input)
                    .margin(egui::Vec2::new(8.0, 5.0))
                    .hint_text("xterm, gnome-terminal, konsole, ..."),
            );
            if ui.add(secondary_btn("Save")).clicked() {
                self.config.terminal = self.terminal_input.trim().to_string();
                self.save_config();
            }
        });
        ui.add_space(4.0);
        ui.label(RichText::new("Diff tool").color(theme::text_secondary()));
        ui.horizontal(|ui| {
            ui.add_sized(
                [340.0, 28.0],
                egui::TextEdit::singleline(&mut self.diff_tool_input)
                    .margin(egui::Vec2::new(8.0, 5.0))
                    .hint_text("meld, kdiff3, vimdiff, ..."),
            );
            if ui.add(secondary_btn("Save")).clicked() {
                self.config.diff_tool = self.diff_tool_input.trim().to_string();
                self.save_config();
            }
        });
        ui.add_space(4.0);
        ui.label(RichText::new("Merge tool").color(theme::text_secondary()));
        ui.horizontal(|ui| {
            ui.add_sized(
                [340.0, 28.0],
                egui::TextEdit::singleline(&mut self.merge_tool_input)
                    .margin(egui::Vec2::new(8.0, 5.0))
                    .hint_text("meld, kdiff3, ..."),
            );
            if ui.add(secondary_btn("Save")).clicked() {
                self.config.merge_tool = self.merge_tool_input.trim().to_string();
                self.save_config();
            }
        });
    }

    fn render_accounts_tab(&mut self, ui: &mut egui::Ui) {
        if self.account_task.is_some() {
            ui.disable();
        }
        if let Some(error) = &self.add_account_error {
            ui.label(RichText::new(error).color(theme::accent_red()).size(12.0));
            if self.account_task.is_none() && ui.button("Retry credential migration").clicked() {
                self.account_task = Some(account_task(|| {
                    crate::auth::store::migrate()?;
                    Ok(AccountChange::Migrated)
                }));
            }
        }
        if self.account_task.is_some() {
            ui.label(styled_muted("Accessing credential store..."));
        }
        // ── Provider Accounts ─────────────────────────────────────────────
        ui.label(
            RichText::new("Provider Accounts")
                .color(theme::text_secondary())
                .size(13.0)
                .strong(),
        );
        ui.add_space(4.0);

        let accounts = self.config.accounts.clone();
        let repo_path = self.state.lock().unwrap().repo_path.clone();
        for acc in &accounts {
            ui.horizontal(|ui| {
                let dot_color = match acc.provider {
                    ProviderKind::GitHubCom | ProviderKind::GitHubEnterprise => {
                        theme::text_primary()
                    }
                    ProviderKind::GitLabCom | ProviderKind::GitLabSelf => theme::accent_red(),
                    ProviderKind::BitbucketCloud | ProviderKind::AzureDevOps => {
                        theme::accent_teal()
                    }
                };
                let (dot_rect, _) =
                    ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                ui.painter()
                    .circle_filled(dot_rect.center(), 3.5, dot_color);
                ui.add_space(4.0);
                ui.label(
                    RichText::new(acc.label())
                        .color(theme::text_secondary())
                        .size(13.0),
                );
                ui.label(styled_muted(&acc.host));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(
                            self.account_task.is_none(),
                            Button::new(
                                RichText::new("Remove")
                                    .color(theme::accent_red())
                                    .size(12.0),
                            )
                            .fill(Color32::TRANSPARENT),
                        )
                        .clicked()
                    {
                        let account = acc.clone();
                        self.account_task = Some(account_task(move || {
                            crate::auth::store::remove(&account.host, &account.username)?;
                            Ok(AccountChange::Removed(account))
                        }));
                    }
                });
            });
            if let (Some(path), Ok(host)) =
                (&repo_path, crate::auth::endpoint::normalize_host(&acc.host))
            {
                let mut selected = self
                    .config
                    .get_repo_config(path)
                    .provider_accounts
                    .get(&host)
                    == Some(&acc.username);
                if ui
                    .checkbox(
                        &mut selected,
                        format!("Use {} for {} in this repository", acc.username, host),
                    )
                    .changed()
                {
                    let previous = self.config.get_repo_config(path);
                    let repo = self
                        .config
                        .per_repo
                        .entry(path.display().to_string())
                        .or_default();
                    if selected {
                        repo.provider_accounts.insert(host, acc.username.clone());
                    } else {
                        repo.provider_accounts.remove(&host);
                    }
                    if let Err(error) = self.config.try_save() {
                        self.config
                            .per_repo
                            .insert(path.display().to_string(), previous);
                        self.add_account_error = Some(error);
                    } else {
                        self.state.lock().unwrap().invalidate_provider_data();
                    }
                }
            }
        }

        if accounts.is_empty() && !self.add_account_open {
            ui.label(styled_muted("No provider accounts connected."));
        }
        ui.add_space(6.0);

        if self.add_account_open {
            ui.separator();
            ui.add_space(6.0);
            ui.label(
                RichText::new("Add Account")
                    .color(theme::text_secondary())
                    .size(13.0)
                    .strong(),
            );
            ui.add_space(4.0);
            // Provider picker
            ui.horizontal_wrapped(|ui| {
                ui.label(styled_label("Provider"));
                ui.add_space(8.0);
                for p in [
                    ProviderKind::GitHubCom,
                    ProviderKind::GitHubEnterprise,
                    ProviderKind::GitLabCom,
                    ProviderKind::GitLabSelf,
                    ProviderKind::BitbucketCloud,
                    ProviderKind::AzureDevOps,
                ] {
                    let active = self.add_account_provider == p;
                    let color = if active {
                        theme::accent_teal()
                    } else {
                        theme::text_muted()
                    };
                    if ui
                        .add(
                            Button::new(RichText::new(p.label()).size(12.0).color(color))
                                .fill(Color32::TRANSPARENT),
                        )
                        .clicked()
                        && !active
                    {
                        self.add_account_host = p.default_host().to_string();
                        self.add_account_provider = p.clone();
                    }
                }
            });
            ui.add_space(4.0);
            if self.add_account_provider.needs_host() {
                settings_text_field(ui, "Host", &mut self.add_account_host);
            } else {
                ui.label(styled_muted(&format!(
                    "Host: {}",
                    self.add_account_provider.default_host()
                )));
            }
            match self.add_account_provider {
                ProviderKind::BitbucketCloud => {
                    ui.label(styled_muted("Use your Atlassian email and a scoped API token (repository, pull request and pipeline permissions as needed)."));
                }
                ProviderKind::AzureDevOps => {
                    ui.label(styled_muted("Use your account name and a PAT with Code, Build, Work Items and Release permissions as needed. Boards and releases show the whole project."));
                }
                _ => {}
            }
            let browser_host = crate::auth::endpoint::normalize_host(
                self.add_account_provider
                    .effective_host(&self.add_account_host),
            )
            .unwrap_or_default();
            let (configured, session) = self.browser_login.show(
                ui,
                &self.add_account_provider,
                &browser_host,
                self.config.oauth_clients.get(&browser_host),
                self.account_task.is_some(),
            );
            if let Some(client) = configured {
                self.config
                    .oauth_clients
                    .insert(client.host.clone(), client);
                self.save_config();
            }
            if let Some(session) = session {
                self.account_task = Some(account_task(move || {
                    crate::auth::store::save_oauth(&session)?;
                    Ok(AccountChange::Saved(StoredAccount {
                        provider: session.client.provider,
                        host: session.client.host,
                        username: session.username,
                    }))
                }));
            }
            ui.separator();
            ui.label("Or connect with a personal access token");
            settings_text_field(ui, "Username", &mut self.add_account_username);
            ui.horizontal(|ui| {
                ui.set_min_height(28.0);
                ui.label(
                    RichText::new(format!("{:<12}", "Token (PAT)"))
                        .monospace()
                        .color(theme::text_muted())
                        .size(12.0),
                );
                ui.add_sized(
                    [260.0, 28.0],
                    egui::TextEdit::singleline(&mut self.add_account_token)
                        .margin(egui::Vec2::new(8.0, 5.0))
                        .password(true)
                        .hint_text("ghp_… / glpat-…"),
                );
            });
            if let Some(ref err) = self.add_account_error.clone() {
                ui.label(RichText::new(err).color(theme::accent_red()).size(12.0));
            }
            ui.add_space(6.0);
            let eff_host = if self.add_account_provider.needs_host() {
                self.add_account_host.trim().to_string()
            } else {
                self.add_account_provider.default_host().to_string()
            };
            let can_save = !self.add_account_username.trim().is_empty()
                && !self.add_account_token.trim().is_empty()
                && !eff_host.is_empty();
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        can_save && self.account_task.is_none(),
                        primary_btn("Save Account"),
                    )
                    .clicked()
                {
                    let username = self.add_account_username.trim().to_string();
                    let token = self.add_account_token.trim().to_string();
                    let provider = self.add_account_provider.clone();
                    self.account_task = Some(account_task(move || {
                        let host = crate::auth::endpoint::normalize_host(&eff_host)?;
                        crate::auth::store::save(&host, &username, &token)?;
                        Ok(AccountChange::Saved(StoredAccount {
                            provider,
                            host,
                            username,
                        }))
                    }));
                }
                if ui.add(secondary_btn("Cancel")).clicked() {
                    self.add_account_open = false;
                    self.browser_login.cancel();
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
        ui.label(
            RichText::new("SSH Keys")
                .color(theme::text_secondary())
                .size(13.0)
                .strong(),
        );
        ui.add_space(4.0);
        if let Some(diag) = &self.ssh_diagnostics {
            let (agent_text, agent_color) = if diag.agent_running {
                (
                    if diag.agent_keys.is_empty() {
                        "running (no keys loaded)"
                    } else {
                        "running"
                    },
                    theme::accent_teal(),
                )
            } else {
                ("not running / SSH_AUTH_SOCK unset", theme::text_muted())
            };
            health_row_color(ui, "SSH agent", agent_text, agent_color);
            for key in &diag.agent_keys {
                ui.label(
                    RichText::new(format!("  {}", key))
                        .monospace()
                        .color(theme::text_muted())
                        .size(12.0),
                );
            }
            for (name, exists) in &diag.key_files {
                let (status, color) = if *exists {
                    ("found", theme::text_secondary())
                } else {
                    ("missing", theme::text_muted())
                };
                health_row_color(ui, &format!("~/.ssh/{}", name), status, color);
            }
        } else {
            ui.label("SSH diagnostics pending");
        }

        ui.add_space(14.0);

        // ── Git Identity ──────────────────────────────────────────────────
        ui.label(
            RichText::new("Global Git identity")
                .color(theme::text_secondary())
                .size(13.0)
                .strong(),
        );
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
        ui.label(styled_muted(&format!(
            "System credential helper: {}",
            git_config_value(None, "credential.helper").unwrap_or_else(|| "not configured".into())
        )));
        ui.label(styled_muted(
            "Tokens stored by Recorde take precedence over the system helper for HTTPS remotes.",
        ));
    }

    fn render_repo_settings(&mut self, ui: &mut egui::Ui) {
        let (
            repo_path,
            head_branch,
            ahead_behind,
            dirty_count,
            conflict_count,
            caps,
            submodules,
            worktrees,
        ) = {
            let s = self.state.lock().unwrap();
            (
                s.repo_path.clone(),
                s.head_branch.clone().unwrap_or_else(|| "—".into()),
                s.ahead_behind,
                s.dirty_count,
                s.conflicted_files.len(),
                s.capabilities.clone(),
                s.submodules.clone(),
                s.worktrees.clone(),
            )
        };
        let Some(path) = repo_path else {
            ui.label(styled_muted(
                "Open a repository to edit repository settings.",
            ));
            return;
        };

        ui.label(
            RichText::new("Repository")
                .color(theme::text_secondary())
                .size(13.0)
                .strong(),
        );
        ui.label(styled_muted(&path.display().to_string()));
        ui.add_space(10.0);
        egui::CollapsingHeader::new("Recovery snapshots").show(ui, |ui| {
            ui.label("Snapshots preserve the index and non-ignored files before commit, reset and discard, including before the first commit. Undo checks for later edits and saves a snapshot for redo. Ignored files are excluded.");
            let mut retention = git2::Repository::open(&path).ok().and_then(|r| r.config().ok()).and_then(|c| c.get_i64("recorde.recoveryRetentionDays").ok()).unwrap_or(0);
            let old_retention = retention;
            egui::ComboBox::from_label("Automatic cleanup").selected_text(if retention <= 0 { "Keep all snapshots".into() } else { format!("After {retention} days") }).show_ui(ui, |ui| {
                for (days, label) in [(0, "Keep all snapshots"), (30, "After 30 days"), (90, "After 90 days")] {
                    ui.selectable_value(&mut retention, days, label);
                }
            });
            ui.label(styled_muted("Cleanup always keeps the latest 20 snapshots, pinned snapshots and incomplete operations."));
            if retention != old_retention {
                crate::git::mutation::spawn_operation(path.clone(), self.state.clone(), move |repo| {
                    git2::Repository::open(repo).and_then(|r| r.config()).and_then(|mut c| c.set_i64("recorde.recoveryRetentionDays", retention)).map_err(|e| e.to_string())?;
                    crate::git::recovery::prune(repo)
                });
            }
            let saved = crate::git::recovery::list(&path);
            if saved.is_empty() { ui.label(styled_muted("No recovery snapshots yet.")); }
            egui::ScrollArea::vertical().id_source("recovery_snapshots").max_height(260.0).show(ui, |ui| {
            for checkpoint in saved {
                ui.horizontal(|ui| {
                    ui.label(format!("{} — {}", checkpoint.label, checkpoint.id));
                    let mut pinned = checkpoint.pinned;
                    if ui.checkbox(&mut pinned, "Keep").changed() {
                        let id = checkpoint.id.clone();
                        crate::git::mutation::spawn_operation(path.clone(), self.state.clone(), move |repo| crate::git::recovery::set_pinned(repo, &id, pinned));
                    }
                    if ui.button("Restore...").clicked() { self.repo_ui.pending_recovery_id = Some(checkpoint.id.clone()); }
                    if ui.button("Delete...").clicked() { self.repo_ui.pending_recovery_id = Some(format!("delete:{}", checkpoint.id)); }
                });
                if self.repo_ui.pending_recovery_id.as_ref() == Some(&checkpoint.id) {
                    ui.label("Restore the saved HEAD, index and file contents? Later changes must be preserved first; the saved snapshot remains available.");
                    ui.horizontal(|ui| {
                        if ui.button("Restore snapshot").clicked() {
                            let checkpoint = checkpoint.clone();
                            self.repo_ui.pending_recovery_id = None;
                            crate::git::mutation::spawn_operation(path.clone(), self.state.clone(), move |repo| {
                                crate::git::recovery::restore(repo, &checkpoint)
                            });
                        }
                        if ui.button("Cancel").clicked() { self.repo_ui.pending_recovery_id = None; }
                    });
                }
                if self.repo_ui.pending_recovery_id.as_deref() == Some(&format!("delete:{}", checkpoint.id)) {
                    ui.label("Delete this recovery snapshot and its protected ref? Its saved data may be removed by Git garbage collection.");
                    ui.horizontal(|ui| {
                        if ui.button("Delete snapshot").clicked() {
                            let checkpoint = checkpoint.clone();
                            self.repo_ui.pending_recovery_id = None;
                            crate::git::mutation::spawn_operation(path.clone(), self.state.clone(), move |repo| {
                                crate::git::recovery::remove(repo, &checkpoint)
                            });
                        }
                        if ui.button("Keep snapshot").clicked() { self.repo_ui.pending_recovery_id = None; }
                    });
                }
            }
            });
        });
        egui::CollapsingHeader::new("Checkout and history").show(ui, |ui| {
            let caps = self.state.lock().unwrap().capabilities.clone().unwrap_or_default();
            ui.label(if caps.is_shallow { "Shallow history" } else { "Full history" });
            if ui.add_enabled(!caps.is_shallow, Button::new("Optimize history loading")).clicked() {
                self.state.lock().unwrap().remote_op = Some("Optimize history loading".into());
                crate::git::mutation::spawn_operation(path.clone(), self.state.clone(), crate::git::lifecycle::optimize_history);
            }
            ui.label(styled_muted("Build Git's history index for faster graph loading. Large repositories may take a few minutes."));
            if caps.is_shallow && ui.button("Fetch full history").clicked() {
                crate::git::mutation::spawn_operation(path.clone(), self.state.clone(), |repo| crate::git::lifecycle::fetch_full_history(repo));
            }
            ui.label(if caps.is_sparse { "Sparse checkout is enabled" } else { "All directories are checked out" });
            ui.label("Directories to include, one per line (cone mode)");
            ui.add(egui::TextEdit::multiline(&mut self.repo_ui.sparse_paths).desired_rows(3));
            ui.label(styled_muted("Root files remain included. Applying or disabling sparse checkout requires a clean worktree and index."));
            ui.horizontal(|ui| {
                if ui.add_enabled(!self.repo_ui.sparse_paths.trim().is_empty(), Button::new("Apply sparse checkout")).clicked() {
                    let directories = self.repo_ui.sparse_paths.lines().map(str::to_string).collect();
                    crate::git::mutation::spawn_operation(path.clone(), self.state.clone(), move |repo| crate::git::lifecycle::set_sparse(repo, Some(directories)));
                }
                if caps.is_sparse && ui.button("Disable sparse checkout").clicked() {
                    crate::git::mutation::spawn_operation(path.clone(), self.state.clone(), |repo| crate::git::lifecycle::set_sparse(repo, None));
                }
            });
        });
        egui::CollapsingHeader::new("Worktrees").show(ui, |ui| {
            settings_text_field(ui, "New branch", &mut self.repo_ui.worktree_branch_input);
            if ui.add_enabled(!self.repo_ui.worktree_branch_input.trim().is_empty(), secondary_btn("Create worktree...")).clicked() {
                if let Some(destination) = rfd::FileDialog::new().set_title("Choose the new worktree directory").save_file() {
                    let branch = self.repo_ui.worktree_branch_input.trim().to_string();
                    crate::git::mutation::spawn_operation(path.clone(), self.state.clone(), move |repo| {
                        crate::git::worktrees::add(repo, &destination, &branch)
                    });
                }
            }
            for worktree in &worktrees {
                ui.horizontal_wrapped(|ui| {
                    ui.label(worktree.branch.as_deref().unwrap_or("Detached"));
                    if ui.small_button("Open").on_hover_text(&worktree.path).clicked() {
                        load_repo(worktree.path.clone().into(), self.state.clone());
                    }
                    if !worktree.is_main {
                        if worktree.prunable { ui.label("Missing worktree"); }
                        for (label, locked) in [(if worktree.locked { "Unlock" } else { "Lock" }, !worktree.locked)] {
                            if ui.small_button(label).clicked() {
                                let destination = std::path::PathBuf::from(&worktree.path);
                                crate::git::mutation::spawn_operation(path.clone(), self.state.clone(), move |repo| {
                                    crate::git::worktrees::set_locked(repo, &destination, locked)
                                });
                            }
                        }
                        if ui.small_button("Remove...").clicked() { self.repo_ui.remove_worktree_pending = Some(worktree.path.clone()); }
                    }
                });
                if self.repo_ui.remove_worktree_pending.as_ref() == Some(&worktree.path) {
                    ui.label(format!("Remove {}? The branch is retained. Git will refuse dirty or locked worktrees.", worktree.path));
                    ui.horizontal(|ui| {
                        if ui.button("Remove worktree").clicked() {
                            let destination = std::path::PathBuf::from(&worktree.path);
                            self.repo_ui.remove_worktree_pending = None;
                            crate::git::mutation::spawn_operation(path.clone(), self.state.clone(), move |repo| {
                                crate::git::worktrees::remove(repo, &destination)
                            });
                        }
                        if ui.button("Cancel").clicked() { self.repo_ui.remove_worktree_pending = None; }
                    });
                }
            }
        });
        egui::CollapsingHeader::new("Submodules").show(ui, |ui| {
            if submodules.is_empty() {
                ui.label(styled_muted("No submodules configured."));
            }
            for module in &submodules {
                ui.horizontal_wrapped(|ui| {
                    ui.label(&module.name);
                    if ui
                        .small_button(if module.initialized {
                            "Update"
                        } else {
                            "Initialize"
                        })
                        .clicked()
                    {
                        let module = module.path.clone();
                        crate::git::mutation::spawn_operation(
                            path.clone(),
                            self.state.clone(),
                            move |repo| crate::git::submodules::update(repo, &module),
                        );
                    }
                    if ui.small_button("Sync URL").clicked() {
                        let module = module.path.clone();
                        crate::git::mutation::spawn_operation(
                            path.clone(),
                            self.state.clone(),
                            move |repo| crate::git::submodules::sync(repo, &module),
                        );
                    }
                    if module.initialized && ui.small_button("Open").clicked() {
                        load_repo(path.join(&module.path), self.state.clone());
                    }
                });
            }
        });

        // ── Health ──────────────────────────────────────────────────────
        ui.label(
            RichText::new("Repository Health")
                .color(theme::text_secondary())
                .size(13.0)
                .strong(),
        );
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
            if dirty_count > 0 {
                parts.push(format!("{} changed", dirty_count));
            }
            if conflict_count > 0 {
                parts.push(format!("{} conflicted", conflict_count));
            }
            parts.join(", ")
        };
        health_row(ui, "Working tree", &dirty_str);
        if let Some(ref caps) = caps {
            let state_str = if caps.rebase_in_progress {
                "Rebasing"
            } else if caps.merge_in_progress {
                "Merging"
            } else if caps.cherry_pick_in_progress {
                "Cherry-picking"
            } else if caps.revert_in_progress {
                "Reverting"
            } else {
                "Normal"
            };
            health_row(ui, "State", state_str);
            // LFS row — three states
            if !caps.lfs_installed {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("{:<14}", "LFS"))
                            .color(theme::text_muted())
                            .size(12.0)
                            .monospace(),
                    );
                    ui.label(
                        RichText::new("not installed")
                            .color(theme::text_muted())
                            .size(12.0),
                    );
                });
            } else if !caps.lfs_active {
                health_row(ui, "LFS", "installed, not used");
            } else {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("{:<14}", "LFS"))
                            .color(theme::text_muted())
                            .size(12.0)
                            .monospace(),
                    );
                    ui.label(
                        RichText::new("active")
                            .color(theme::accent_teal())
                            .size(12.0),
                    );
                    if !caps.lfs_patterns.is_empty() {
                        ui.label(
                            RichText::new(format!("({})", caps.lfs_patterns.join(", ")))
                                .color(theme::text_secondary())
                                .size(12.0),
                        );
                    }
                });
            }
            // Submodules row — dim "none" or teal count + hover tooltip
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("{:<14}", "Submodules"))
                        .color(theme::text_muted())
                        .size(12.0)
                        .monospace(),
                );
                if submodules.is_empty() {
                    ui.label(RichText::new("none").color(theme::text_muted()).size(12.0));
                } else {
                    let count_label = ui.label(
                        RichText::new(format!("{}", submodules.len()))
                            .color(theme::accent_teal())
                            .size(12.0),
                    );
                    let tooltip: String = submodules
                        .iter()
                        .map(|s| {
                            let status = if s.initialized {
                                "initialized"
                            } else {
                                "not initialized"
                            };
                            format!("{} ({})", s.name, status)
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    count_label.on_hover_text(tooltip);
                }
            });
            // Worktrees row — dim "1 (main only)" or teal count + hover tooltip
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("{:<14}", "Worktrees"))
                        .color(theme::text_muted())
                        .size(12.0)
                        .monospace(),
                );
                if worktrees.len() <= 1 {
                    ui.label(
                        RichText::new("1 (main only)")
                            .color(theme::text_muted())
                            .size(12.0),
                    );
                } else {
                    let count_label = ui.label(
                        RichText::new(format!("{}", worktrees.len()))
                            .color(theme::accent_teal())
                            .size(12.0),
                    );
                    let tooltip: String = worktrees
                        .iter()
                        .map(|w| {
                            let branch = w.branch.as_deref().unwrap_or("(detached)");
                            let tag = if w.is_main { " [main]" } else { "" };
                            format!("{}{} — {}", branch, tag, &w.path)
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    count_label.on_hover_text(tooltip);
                }
            });
            health_row(ui, "Provider", &format!("{:?}", caps.provider));
        }
        health_row(ui, "Git", self.git_version.trim());

        ui.add_space(12.0);

        // ── Origin URL ───────────────────────────────────────────────────
        ui.label(
            RichText::new("Add or edit remote")
                .color(theme::text_secondary())
                .strong(),
        );
        settings_text_field(ui, "Name", &mut self.repo_ui.remote_name_input);
        settings_text_field(ui, "URL", &mut self.repo_ui.origin_url_input);
        ui.horizontal(|ui| {
            let label = if self.repo_ui.remote_rename_from.is_some() {
                "Save remote"
            } else {
                "Add / update remote"
            };
            if ui
                .add_enabled(
                    !self.repo_ui.remote_name_input.trim().is_empty()
                        && !self.repo_ui.origin_url_input.trim().is_empty(),
                    secondary_btn(label),
                )
                .clicked()
            {
                let name = self.repo_ui.remote_name_input.trim().to_string();
                let url = self.repo_ui.origin_url_input.trim().to_string();
                let old = self.repo_ui.remote_rename_from.take();
                let state = self.state.clone();
                crate::git::mutation::spawn_operation(
                    path.clone(),
                    self.state.clone(),
                    move |path| {
                        crate::git::remote::save_remote(path, &name, &url, old.as_deref())?;
                        let mut s = state.lock().unwrap();
                        if s.repo_path.as_deref() == Some(path) {
                            s.invalidate_provider_data();
                        }
                        Ok(())
                    },
                );
            }
            if ui.small_button("Clear").clicked() {
                self.repo_ui.remote_name_input.clear();
                self.repo_ui.origin_url_input.clear();
                self.repo_ui.remote_rename_from = None;
            }
        });
        if let Ok(repo) = git2::Repository::open(&path) {
            if let Ok(remotes) = repo.remotes() {
                for name in remotes.iter().flatten().flatten() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(name);
                        if ui.small_button("Edit / rename").clicked() {
                            self.repo_ui.remote_name_input = name.into();
                            self.repo_ui.remote_rename_from = Some(name.into());
                            self.repo_ui.origin_url_input = repo
                                .find_remote(name)
                                .ok()
                                .and_then(|r| r.url().ok().map(str::to_string))
                                .unwrap_or_default();
                        }
                        if ui.small_button("Remove...").clicked() {
                            self.repo_ui.remote_delete_pending = Some(name.into());
                        }
                    });
                }
            }
        }
        if let Some(name) = self.repo_ui.remote_delete_pending.clone() {
            ui.label(format!("Remove remote {name} and its local tracking refs?"));
            ui.horizontal(|ui| {
                if ui.button("Remove remote").clicked() {
                    self.repo_ui.remote_delete_pending = None;
                    let state = self.state.clone();
                    crate::git::mutation::spawn_operation(
                        path.clone(),
                        self.state.clone(),
                        move |path| {
                            crate::git::remote::remove_remote(path, &name)?;
                            let mut s = state.lock().unwrap();
                            if s.repo_path.as_deref() == Some(path) {
                                s.invalidate_provider_data();
                            }
                            Ok(())
                        },
                    );
                }
                if ui.button("Cancel").clicked() {
                    self.repo_ui.remote_delete_pending = None;
                }
            });
        }

        ui.add_space(14.0);

        // ── Git identity ─────────────────────────────────────────────────
        ui.label(
            RichText::new("Repository Git identity")
                .color(theme::text_secondary())
                .size(13.0)
                .strong(),
        );
        settings_text_field(ui, "Name", &mut self.repo_ui.repo_git_name_input);
        settings_text_field(ui, "Email", &mut self.repo_ui.repo_git_email_input);
        ui.add_space(6.0);
        if ui.add(primary_btn("Save Repository Identity")).clicked() {
            let name = self.repo_ui.repo_git_name_input.clone();
            let email = self.repo_ui.repo_git_email_input.clone();
            crate::git::mutation::spawn_operation(path.clone(), self.state.clone(), move |path| {
                save_git_identity(Some(path), &name, &email)
            });
        }
        ui.add_space(14.0);
        self.repo_ui
            .commit_signing
            .show(ui, path.clone(), self.state.clone());
        ui.add_space(14.0);
        ui.label(
            RichText::new("Remotes")
                .color(theme::text_secondary())
                .size(13.0)
                .strong(),
        );
        if let Ok(repo) = git2::Repository::open(&path) {
            let provider_remote = crate::git::remote::provider_remote(&repo).unwrap_or_default();
            let mut chosen = provider_remote.clone();
            egui::ComboBox::from_label("Provider remote")
                .selected_text(if chosen.is_empty() {
                    "Choose a remote"
                } else {
                    &chosen
                })
                .show_ui(ui, |ui| {
                    if let Ok(remotes) = repo.remotes() {
                        for remote in remotes.iter().flatten().flatten() {
                            ui.selectable_value(&mut chosen, remote.to_string(), remote);
                        }
                    }
                });
            if chosen != provider_remote {
                let state = self.state.clone();
                crate::git::mutation::spawn_operation(path.clone(), state.clone(), move |path| {
                    crate::git::merge::run_git_seq(
                        path,
                        &["config", "--local", "recorde.providerRemote", &chosen],
                    )
                    .map_err(|e| e.user_message())?;
                    let mut s = state.lock().unwrap();
                    if s.repo_path.as_deref() == Some(path) {
                        s.invalidate_provider_data();
                    }
                    Ok(())
                });
            }
            let current = repo
                .config()
                .ok()
                .and_then(|c| c.get_string("remote.pushDefault").ok())
                .unwrap_or_default();
            let mut selected = current.clone();
            egui::ComboBox::from_label("Default push remote")
                .selected_text(if current.is_empty() {
                    "Choose a remote"
                } else {
                    &current
                })
                .show_ui(ui, |ui| {
                    if let Ok(remotes) = repo.remotes() {
                        for remote in remotes.iter().flatten().flatten() {
                            ui.selectable_value(&mut selected, remote.to_string(), remote);
                        }
                    }
                });
            if selected != current {
                crate::git::mutation::spawn_operation(
                    path.clone(),
                    self.state.clone(),
                    move |path| {
                        crate::git::process::git(
                            path,
                            &["config", "--local", "remote.pushDefault", &selected],
                            &std::sync::atomic::AtomicBool::new(false),
                        )
                        .map(|_| ())
                        .map_err(|e| e.user_message())
                    },
                );
            }
            if let Ok(head) = repo.head() {
                if head.is_branch() {
                    if let Some(branch) = head.shorthand().ok() {
                        let key = format!("branch.{branch}.pushRemote");
                        let current = repo
                            .config()
                            .ok()
                            .and_then(|c| c.get_string(&key).ok())
                            .unwrap_or_default();
                        let mut selected = current.clone();
                        egui::ComboBox::from_label(format!("Push remote for {branch}"))
                            .selected_text(if current.is_empty() {
                                "Use repository default / upstream"
                            } else {
                                &current
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut selected,
                                    String::new(),
                                    "Use repository default / upstream",
                                );
                                if let Ok(remotes) = repo.remotes() {
                                    for remote in remotes.iter().flatten().flatten() {
                                        ui.selectable_value(
                                            &mut selected,
                                            remote.to_string(),
                                            remote,
                                        );
                                    }
                                }
                            });
                        if selected != current {
                            crate::git::mutation::spawn_operation(
                                path.clone(),
                                self.state.clone(),
                                move |path| {
                                    let args = if selected.is_empty() {
                                        vec!["config", "--local", "--unset", &key]
                                    } else {
                                        vec!["config", "--local", &key, &selected]
                                    };
                                    crate::git::merge::run_git_seq(path, &args)
                                        .map_err(|e| e.user_message())
                                },
                            );
                        }
                    }
                }
            }
        }
        for remote in repo_remotes(&path) {
            ui.label(styled_muted(&remote));
        }
    }

    fn render_about(&self, ui: &mut egui::Ui) {
        ui.label(
            RichText::new("Recorde")
                .color(theme::text_primary())
                .size(14.0)
                .strong(),
        );
        ui.label(styled_muted(
            "A cross-platform desktop Git client built with Rust by Antony Doyle",
        ));
        ui.hyperlink_to(
            "https://github.com/antonydoyle/recorde",
            "https://github.com/antonydoyle/recorde",
        );
        ui.hyperlink_to("https://antonydoyle.com/", "https://antonydoyle.com/");
        ui.add_space(8.0);
        ui.label(
            RichText::new("Robert Recorde (c. 1510 – 1558) was a Welsh physician and mathematician. He invented the equals sign (=) and also introduced the pre-existing plus (+) and minus (−) signs to English speakers in 1557.")
                .color(theme::text_muted())
                .size(13.0)
                .italics(),
        );
        ui.add_space(8.0);
        ui.label(styled_muted("Keyboard shortcuts are listed in the command palette and configurable in Settings > Keyboard. Ctrl on Windows/Linux is Command on macOS."));
        ui.add_space(16.0);
        ui.collapsing("Third-party font licenses", |ui| {
            for (name, license) in [
                (
                    "Inter 4.1",
                    include_str!("../assets/fonts/Inter-LICENSE.txt"),
                ),
                (
                    "Material Icons",
                    include_str!("../assets/fonts/MaterialIcons-LICENSE.txt"),
                ),
                (
                    "Noto Sans Symbols",
                    include_str!("../assets/fonts/NotoSansSymbols-OFL.txt"),
                ),
                (
                    "Noto Sans Symbols 2",
                    include_str!("../assets/fonts/NotoSansSymbols2-OFL.txt"),
                ),
                (
                    "Noto Emoji",
                    include_str!("../assets/fonts/NotoEmoji-OFL.txt"),
                ),
            ] {
                ui.collapsing(name, |ui| {
                    ui.small(license);
                });
            }
        });
    }

    fn render_pull_requests(&mut self, ctx: &Context) {
        use crate::providers::{CheckState, PrState};

        if self.repo_ui.review_auto_refresh
            && self.repo_ui.last_review_refresh.elapsed() >= std::time::Duration::from_secs(30)
            && ctx.input(|i| i.viewport().focused.unwrap_or(false))
        {
            let refresh = {
                let state = self.state.lock().unwrap();
                !state.pr_loading
                    && !state.pr_detail_loading
                    && state.pr_error.is_none()
                    && state
                        .pr_selected
                        .and_then(|index| state.pr_list.get(index))
                        .is_some_and(|item| matches!(item.state, PrState::Open | PrState::Draft))
            };
            if refresh {
                crate::git::operations::load_pr_details(self.state.clone());
            }
            self.repo_ui.last_review_refresh = std::time::Instant::now();
        }

        let (repo_path, branch, pr_list, pr_loaded, pr_loading, pr_error, pr_selected, pr_filter) = {
            let s = self.state.lock().unwrap();
            (
                s.repo_path.clone(),
                s.head_branch.clone().unwrap_or_else(|| "HEAD".into()),
                s.pr_list.clone(),
                s.pr_loaded,
                s.pr_loading,
                s.pr_error.clone(),
                s.pr_selected,
                s.pr_filter,
            )
        };

        let mut new_filter: Option<PrFilter> = None;
        let mut new_selected: Option<Option<usize>> = None;
        let mut checkout_review: Option<(u32, String)> = None;
        let mut open_url_req: Option<String> = None;

        egui::Window::new("Pull Requests / Merge Requests")
            .collapsible(false)
            .resizable(true)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .default_size([860.0, 560.0])
            .show(ctx, |ui| {
                // ── Header ─────────────────────────────────────────────────
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Pull Requests").color(theme::text_primary()).size(14.0).strong());
                    ui.checkbox(&mut self.repo_ui.review_auto_refresh, "Refresh selected review every 30s")
                        .on_hover_text("Refresh checks, reviews and merge outcome while focused. Pauses after API errors; use Refresh to retry.");
                    ui.add_space(8.0);
                    for f in [PrFilter::Open, PrFilter::Closed, PrFilter::All] {
                        let active = pr_filter == f;
                        let color = if active { theme::accent_teal() } else { theme::text_muted() };
                        let btn = ui.add(Button::new(RichText::new(f.label()).size(13.0).color(color))
                            .fill(Color32::TRANSPARENT));
                        if active { ui.painter().line_segment([btn.rect.left_bottom(), btn.rect.right_bottom()], egui::Stroke::new(2.0, theme::accent_teal())); }
                        if btn.clicked() && !active { new_filter = Some(f); }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(Button::new("✕").fill(Color32::TRANSPARENT)).clicked() {
                            self.repo_ui.pull_requests_open = false;
                        }
                        if ui.add(secondary_btn("Refresh")).clicked() {
                            load_prs(self.state.clone());
                        }
                        if let Some(path) = &repo_path {
                            if ui.add_enabled(self.repo_ui.provider_editor.is_none(), primary_btn("Create PR / MR")).clicked() {
                                self.repo_ui.provider_editor = Some(crate::ui::provider_editor::ProviderEditor::new(path.clone(), None,
                                    crate::providers::write::Action::Create { source_project: String::new(), target_project: String::new(), title: String::new(), body: String::new(), source: branch.clone(), target: String::new(), draft: false }));
                            }
                        }
                    });
                });
                ui.separator();

                // ── Load trigger ───────────────────────────────────────────
                if !pr_loaded && !pr_loading && pr_error.is_none() {
                    load_prs(self.state.clone());
                }

                if pr_loading {
                    ui.centered_and_justified(|ui| {
                        ui.label(RichText::new("Loading…").color(theme::text_muted()).size(14.0));
                    });
                    return;
                }

                if let Some(ref err) = pr_error {
                    ui.add_space(8.0);
                    ui.label(RichText::new(err).color(theme::accent_red()).size(13.0));
                    ui.add_space(4.0);
                    ui.label(styled_muted("Add a token in Settings > Accounts & SSH to enable in-app PR listing."));
                    return;
                }

                let visible: Vec<(usize, &crate::providers::PrItem)> = pr_list.iter().enumerate()
                    .filter(|(_, p)| pr_filter.matches(&p.state))
                    .collect();

                ui.columns(2, |cols| {
                    // ── Left: PR list ─────────────────────────────────────
                    egui::ScrollArea::vertical().id_source("pr_list").show(&mut cols[0], |ui| {
                        if visible.is_empty() {
                            ui.label(RichText::new("No pull requests.").color(theme::text_muted()).size(13.0));
                        }
                        for (i, pr) in &visible {
                            let is_sel = pr_selected == Some(*i);
                            let bg = if is_sel { theme::bg_selected() } else { Color32::TRANSPARENT };
                            let (row, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 56.0), egui::Sense::click());
                            ui.painter().rect_filled(row, 0.0, bg);
                            ui.allocate_ui_at_rect(row, |ui| {
                                ui.horizontal(|ui| {
                                    ui.add_space(8.0);
                                    // State dot
                                    let dot_color = match pr.state {
                                        PrState::Open   => theme::accent_green(),
                                        PrState::Draft  => theme::text_muted(),
                                        PrState::Merged => theme::accent_purple(),
                                        PrState::Closed => theme::accent_red(),
                                    };
                                    let (dr, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                                    ui.painter().circle_filled(dr.center(), 4.0, dot_color);
                                    ui.add_space(4.0);
                                    ui.vertical(|ui| {
                                        ui.label(RichText::new(format!("#{} {}", pr.number, &pr.title)).color(theme::text_primary()).size(13.0));
                                        ui.label(RichText::new(format!("{} → {}  by @{}", pr.head_ref, pr.base_ref, pr.author)).color(theme::text_muted()).size(12.0));
                                        // Check state badge
                                        if let Some(ref cs) = pr.check_state {
                                            let (cs_text, cs_color) = match cs {
                                                CheckState::Success => ("✓ checks passed", theme::accent_green()),
                                                CheckState::Failure => ("✗ checks failed", theme::accent_red()),
                                                CheckState::Pending => ("… checks pending", theme::accent_yellow()),
                                            };
                                            ui.label(RichText::new(cs_text).size(12.0).color(cs_color));
                                        }
                                    });
                                });
                            });
                            if ui.interact(row, egui::Id::new(("pr_row", i)), egui::Sense::click()).clicked() {
                                new_selected = Some(Some(*i));
                            }
                            ui.painter().line_segment([row.left_bottom(), row.right_bottom()],
                                egui::Stroke::new(1.0, theme::border()));
                        }
                    });

                    // ── Right: PR detail ──────────────────────────────────
                    if self.state.lock().unwrap().pr_detail_loading { cols[1].spinner(); }
                    if let Some(idx) = pr_selected {
                        if let Some(pr) = pr_list.get(idx) {
                            cols[1].add_space(4.0);
                            cols[1].horizontal(|ui| {
                                ui.label(RichText::new(format!("#{}", pr.number)).color(theme::text_muted()).size(13.0));
                                ui.add_space(6.0);
                                ui.label(RichText::new(&pr.title).color(theme::text_primary()).size(14.0).strong());
                            });
                            cols[1].add_space(4.0);
                            cols[1].horizontal(|ui| {
                                ui.label(RichText::new(format!("{} → {}", pr.head_ref, pr.base_ref)).color(theme::text_muted()).size(12.0));
                                ui.add_space(8.0);
                                ui.label(RichText::new(format!("by @{}", pr.author)).color(theme::text_secondary()).size(12.0));
                                ui.add_space(8.0);
                                ui.label(RichText::new(format!("updated {}", crate::theme::relative_time(pr.updated_ts))).color(theme::text_muted()).size(12.0));
                            });
                            if !pr.label_names.is_empty() {
                                cols[1].add_space(2.0);
                                cols[1].horizontal(|ui| {
                                    for label in &pr.label_names {
                                        ui.label(RichText::new(label).size(12.0).color(theme::accent_blue())
                                            .background_color(theme::accent_blue().gamma_multiply(0.15)));
                                        ui.add_space(4.0);
                                    }
                                });
                            }
                            if let Some(status) = &pr.queue_status { cols[1].label(status); }
                            let queue_supported = self.state.lock().unwrap().pr_source.as_ref().is_some_and(|(_, provider, _)| matches!(provider, crate::git::capabilities::ProviderType::GitHub | crate::git::capabilities::ProviderType::GitLab));
                            if cols[1].add_enabled(queue_supported && !self.state.lock().unwrap().pr_detail_loading, secondary_btn("Refresh queue / train status")).on_hover_text("Merge queue status is available for GitHub and GitLab.").clicked() {
                                if let Some(item) = self.state.lock().unwrap().pr_list.get_mut(idx) { item.queue_status = Some("Loading queue status...".into()); }
                                crate::git::operations::load_pr_details(self.state.clone());
                            }
                            cols[1].add_space(6.0);
                            cols[1].separator();
                            cols[1].add_space(4.0);
                            if pr.body.trim().is_empty() {
                                cols[1].label(RichText::new("No description.").color(theme::text_muted()).size(13.0));
                            } else {
                                egui::ScrollArea::vertical().id_source("pr_body").max_height(200.0).show(&mut cols[1], |ui| {
                                    ui.label(RichText::new(pr.body.trim()).color(theme::text_secondary()).size(13.0));
                                });
                            }
                            cols[1].add_space(8.0);
                            cols[1].separator();
                            cols[1].add_space(6.0);
                            cols[1].horizontal_wrapped(|ui| {
                                if ui.add(primary_btn("Open in Browser")).clicked() {
                                    open_url_req = Some(pr.url.clone());
                                }
                                if ui.add(secondary_btn("Checkout Review")).clicked() {
                                    checkout_review = Some((pr.number, pr.head_oid.clone()));
                                }
                                if let Some(path) = &repo_path {
                                    let remote = self.state.lock().unwrap().pr_source.as_ref().map(|(remote, _, url)| (remote.clone(), url.clone()));
                                    let enabled = self.repo_ui.provider_editor.is_none() && matches!(pr.state, PrState::Open | PrState::Draft) && !pr.head_oid.is_empty();
                                    if ui.add_enabled(enabled, secondary_btn("Review...")).clicked() {
                                        self.repo_ui.provider_editor = Some(crate::ui::provider_editor::ProviderEditor::new(path.clone(), remote.clone(),
                                            crate::providers::write::Action::Review { number: pr.number, head: pr.head_oid.clone(), target: pr.base_ref.clone(), body: String::new(), decision: crate::providers::write::Decision::Comment, inline: None }));
                                    }
                                    let can_merge = self.state.lock().unwrap().pr_source.as_ref().is_some_and(|(_, provider, _)| *provider != crate::git::capabilities::ProviderType::Bitbucket);
                                    if ui.add_enabled(enabled && can_merge && pr.state == PrState::Open, secondary_btn("Merge...")).on_hover_text("Bitbucket merges must be completed in the browser because its API cannot guard the displayed commit.").clicked() {
                                        self.repo_ui.provider_editor = Some(crate::ui::provider_editor::ProviderEditor::new(path.clone(), remote,
                                            crate::providers::write::Action::Merge { number: pr.number, head: pr.head_oid.clone(), target: pr.base_ref.clone(), method: crate::providers::write::MergeMethod::Merge, queue: false }));
                                    }
                                }
                            });
                        }
                    } else {
                        cols[1].centered_and_justified(|ui| {
                            ui.label(RichText::new("Select a pull request to view details.").color(theme::text_muted()).size(13.0));
                        });
                    }
                });
            });

        // Apply mutations after borrow ends
        if let Some(f) = new_filter {
            {
                self.state.lock().unwrap().pr_filter = f;
            }
            load_prs(self.state.clone());
        }
        if let Some(sel) = new_selected {
            self.state.lock().unwrap().pr_selected = sel;
            if sel.is_some() {
                crate::git::operations::load_pr_details(self.state.clone());
            }
        }
        if let Some(ref url) = open_url_req {
            if let Err(error) = open_url(url) {
                self.state.lock().unwrap().error = Some(error);
            }
        }
        if let Some((number, oid)) = checkout_review {
            let (path, source) = {
                let s = self.state.lock().unwrap();
                (s.repo_path.clone(), s.pr_source.clone())
            };
            if let (Some(path), Some((remote, provider, remote_url))) = (path, source) {
                crate::git::mutation::spawn_operation(path, self.state.clone(), move |path| {
                    crate::git::reviews::verify_remote(path, &remote, &remote_url)?;
                    crate::git::reviews::checkout(path, &remote, &provider, number, &oid)
                });
            }
        }
    }

    fn focus_pane(&mut self, pane: u8) {
        match pane {
            1 => self.config.show_sidebar = true,
            3 => self.config.show_right_panel = true,
            4 => self.config.show_diff_panel = true,
            _ => {}
        }
        self.save_config();
        self.state.lock().unwrap().focused_pane = pane;
    }

    fn reset_layout(&mut self) {
        self.config.show_sidebar = true;
        self.config.show_right_panel = true;
        self.config.show_diff_panel = true;
        self.config.sidebar_width = None;
        self.config.right_panel_width = None;
        self.config.diff_panel_height = None;
        if let Some(ref path) = self.state.lock().unwrap().repo_path.clone() {
            self.config.reset_repo_column_widths(path);
        }
        self.save_config();
        self.commit_graph_view.reset_column_widths();
    }
}

// ── eframe::App ───────────────────────────────────────────────────────────────

impl eframe::App for RecordeApp {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.persist_navigation();
        self.save_config();
    }

    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        let ready = self.state.lock().unwrap().commit_graph.is_some();
        self.frame_metrics
            .observe(ctx, _frame.info().cpu_usage, ready);
        self.sync_repo_session();
        if self.last_navigation_save.elapsed() >= std::time::Duration::from_secs(2) {
            self.persist_navigation();
            self.last_navigation_save = std::time::Instant::now();
        }
        let provider = self
            .state
            .lock()
            .unwrap()
            .capabilities
            .as_ref()
            .and_then(|c| c.provider_target.clone());
        if provider != self.observed_provider {
            self.repo_ui.issues_fetched = false;
            self.repo_ui.ci_fetched = false;
            self.repo_ui.releases_fetched = false;
            self.observed_provider = provider;
        }
        let request = {
            let mut state = self.state.lock().unwrap();
            state
                .pending_conflict_editor
                .take()
                .zip(state.repo_path.clone())
        };
        if let Some((file, repo)) = request {
            self.repo_ui.conflict_editor =
                Some(crate::ui::conflict_editor::ConflictEditor::new(repo, file));
        }
        if let Some(editor) = &mut self.repo_ui.conflict_editor {
            if editor.show(ctx, self.state.clone()) {
                self.repo_ui.conflict_editor = None;
            }
        }
        if let Some(editor) = &mut self.repo_ui.provider_editor {
            if editor.show(ctx, self.state.clone()) {
                self.repo_ui.provider_editor = None;
            }
        }
        if let Some(task) = &self.account_task {
            let completion = match task.try_recv() {
                Ok(result) => Some(result),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Err(
                    "Credential operation stopped; retry from Accounts & SSH.".into(),
                )),
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
            };
            if let Some(result) = completion {
                self.account_task = None;
                match result {
                    Ok(change) => {
                        self.add_account_error = None;
                        self.state.lock().unwrap().invalidate_provider_data();
                        match change {
                            AccountChange::Migrated => {}
                            AccountChange::Saved(account) => {
                                self.config.accounts.retain(|a| {
                                    !(a.username == account.username
                                        && crate::auth::endpoint::normalize_host(&a.host)
                                            .ok()
                                            .as_deref()
                                            == Some(&account.host))
                                });
                                self.config.accounts.push(account);
                                self.save_config();
                                self.add_account_open = false;
                                self.add_account_username.clear();
                                self.add_account_token.clear();
                            }
                            AccountChange::Removed(account) => {
                                self.config.accounts.retain(|a| {
                                    !(a.host == account.host && a.username == account.username)
                                });
                                self.save_config();
                            }
                        }
                    }
                    Err(error) => {
                        self.state.lock().unwrap().error = Some(error.clone());
                        self.add_account_error = Some(error);
                    }
                }
            }
        }
        if !self.settings_open
            || self.settings_tab != SettingsTab::Accounts
            || !self.add_account_open
        {
            self.browser_login.cancel();
        }
        ctx.style_mut(theme::kill_debug);
        ctx.options_mut(|o| o.warn_on_id_clash = false);

        let shortcuts = !ctx.wants_keyboard_input()
            && !self.settings_open
            && !self.command_palette_open
            && !self.branch_switcher_open
            && !self.add_account_open
            && !self.commit_graph_view.has_pending_action()
            && {
                let s = self.state.lock().unwrap();
                matches!(s.modal, ModalKind::None)
                    && s.pending_discard.is_none()
                    && s.repo_load_path.is_none()
            }
            && self.repo_ui.rebase_modal.is_none()
            && self.repo_ui.conflict_editor.is_none()
            && self.repo_ui.provider_editor.is_none();
        self.commit_graph_view.set_interactions_enabled(
            !self.settings_open
                && !self.command_palette_open
                && !self.branch_switcher_open
                && !self.add_account_open
                && !self.commit_graph_view.has_pending_action()
                && self.repo_ui.rebase_modal.is_none()
                && self.repo_ui.conflict_editor.is_none()
                && self.repo_ui.provider_editor.is_none()
                && {
                    let s = self.state.lock().unwrap();
                    matches!(s.modal, ModalKind::None)
                        && s.pending_discard.is_none()
                        && s.repo_load_path.is_none()
                },
        );
        if let Some(command) = self.config.shortcuts.take_command(ctx, !shortcuts) {
            self.dispatch_command(ctx, command);
        }

        // ── Detect external repo changes (tags, commits from other tools) ──
        // Request a wakeup in ~1 s so the poller fires even when the window is idle.
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
        self.poll_external_changes(ctx);

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
        if self.repo_ui.rebase_modal.is_some() {
            let mut close = false;
            let mut execute: Option<(String, Vec<RebaseEntry>)> = None;
            egui::Window::new("Interactive Rebase")
                .collapsible(false)
                .resizable(true)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .frame(egui::Frame::window(&ctx.style()).inner_margin(egui::Margin::same(16.0)))
                .show(ctx, |ui| {
                    if let Some(ref mut m) = self.repo_ui.rebase_modal {
                        match m.show(ui) {
                            Some(RebaseModalResult::Execute) => {
                                execute = Some((m.upstream_oid.clone(), m.entries.clone()));
                                close = true;
                            }
                            Some(RebaseModalResult::Cancel) => close = true,
                            None => {}
                        }
                    }
                });
            if close {
                self.repo_ui.rebase_modal = None;
            }
            if let Some((oid, entries)) = execute {
                rebase::start_rebase(&oid, entries, self.state.clone());
            }
        }

        // ── Panels ────────────────────────────────────────────────────────
        self.status_bar.show(ctx);

        // Title bar
        egui::TopBottomPanel::top("title_bar")
            .exact_height(40.0)
            .frame(
                egui::Frame::none()
                    .fill(theme::bg_titlebar())
                    .inner_margin(egui::Margin::symmetric(12.0, 0.0)),
            )
            .show(ctx, |ui| {
                let r = ui.max_rect();
                ui.painter().line_segment(
                    [r.left_bottom(), r.right_bottom()],
                    Stroke::new(1.0, theme::border()),
                );
                ui.horizontal_centered(|ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(480.0, 32.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| self.show_menu_contents(ctx, ui),
                    );

                    ui.add_space(14.0);
                    let mut layout_changed = false;
                    layout_changed |= panel_toggle(
                        ui,
                        &mut self.config.show_sidebar,
                        "Sidebar",
                        "Show or hide the branch/sidebar panel",
                    );
                    layout_changed |= panel_toggle(
                        ui,
                        &mut self.config.show_right_panel,
                        "Changes",
                        "Show or hide the Changes and Details panel",
                    );
                    layout_changed |= panel_toggle(
                        ui,
                        &mut self.config.show_diff_panel,
                        "Diff",
                        "Show or hide the bottom diff panel",
                    );
                    if layout_changed {
                        self.save_config();
                    }

                    // Icon is rendered inside menu::bar (see show_menu_contents),
                    // because menu::bar expands to fill all available width.
                });
            });

        self.render_repo_tabs(ctx);

        let has_workspace = {
            let state = self.state.lock().unwrap();
            state.repo_path.is_some() || state.repo_load_path.is_some()
        };
        if !has_workspace {
            self.show_welcome(ctx);
            self.commit_graph_view
                .show_action_confirmation(ctx, &self.config.protected_branches);
            self.show_operation_overlays(ctx);
            return;
        }

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
            !self.commit_graph_view.has_pending_action(),
        );
        if let Some(a) = toolbar_action {
            self.handle_toolbar(ctx, a);
        }

        if self.config.show_sidebar {
            let sidebar_response =
                egui::SidePanel::left(egui::Id::new(("sidebar", &self.observed_repo)))
                    .resizable(true)
                    .default_width(self.config.sidebar_width.unwrap_or(216.0))
                    .width_range(180.0..=360.0)
                    .frame(egui::Frame::none())
                    .show(ctx, |ui| {
                        if self.commit_graph_view.has_pending_action() {
                            ui.disable();
                        }
                        let a = self.sidebar.show(ui);
                        if let Some(sa) = a {
                            self.handle_sidebar(sa);
                        }
                        if self.state.lock().unwrap().focused_pane == 1 {
                            focus_ring(ui);
                        }
                    });
            let w = sidebar_response.response.rect.width();
            if self
                .config
                .sidebar_width
                .map_or(true, |old| (old - w).abs() > 1.0)
            {
                self.config.sidebar_width = Some(w);
                self.save_config();
            }
        }

        if self.config.show_right_panel {
            let right_response =
                egui::SidePanel::right(egui::Id::new(("right_panel_v2", &self.observed_repo)))
                    .resizable(true)
                    .default_width(self.config.right_panel_width.unwrap_or(344.0))
                    .width_range(300.0..=520.0)
                    .frame(egui::Frame::none())
                    .show(ctx, |ui| {
                        if self.commit_graph_view.has_pending_action() {
                            ui.disable();
                        }
                        let selected_commit = self.state.lock().unwrap().selected_commit.clone();
                        let has_commit = selected_commit.is_some();
                        let has_dirty = self.state.lock().unwrap().dirty_count > 0;
                        if let Some(ref oid) = selected_commit {
                            if self.repo_ui.last_auto_detail_commit.as_deref() != Some(oid) {
                                self.repo_ui.right_tab = RightTab::Details;
                                self.repo_ui.last_auto_detail_commit = Some(oid.clone());
                            }
                        } else {
                            self.repo_ui.last_auto_detail_commit = None;
                            if self.state.lock().unwrap().focused_pane == 3 {
                                self.repo_ui.right_tab = RightTab::Changes;
                            }
                        }

                        // Tab bar
                        let tab_h = 48.0;
                        let tab_rect = egui::Rect::from_min_size(
                            ui.min_rect().min,
                            egui::Vec2::new(ui.available_width(), tab_h),
                        );
                        ui.painter().rect_filled(tab_rect, 0.0, theme::bg_panel());
                        ui.painter().line_segment(
                            [tab_rect.left_bottom(), tab_rect.right_bottom()],
                            Stroke::new(1.0, theme::border()),
                        );

                        ui.allocate_ui_at_rect(tab_rect, |ui| {
                            ui.horizontal_centered(|ui| {
                                ui.add_space(12.0);
                                for (tab, lbl) in &[
                                    (RightTab::Changes, "Changes"),
                                    (RightTab::Details, "Details"),
                                ] {
                                    let sel = self.repo_ui.right_tab == *tab;
                                    let text_col = if sel {
                                        theme::text_primary()
                                    } else {
                                        theme::text_muted()
                                    };
                                    let btn = ui
                                        .add(
                                            Button::new(
                                                RichText::new(*lbl).size(14.0).color(text_col),
                                            )
                                            .fill(Color32::TRANSPARENT)
                                            .stroke(Stroke::NONE),
                                        )
                                        .on_hover_text(match *tab {
                                            RightTab::Changes => {
                                                "Show working tree changes and commit form"
                                            }
                                            RightTab::Details => "Show selected commit details",
                                        });
                                    // Teal underline for active tab (matches mockup)
                                    if sel {
                                        let br = btn.rect;
                                        ui.painter().line_segment(
                                            [br.left_bottom(), br.right_bottom()],
                                            Stroke::new(2.0, theme::accent_teal()),
                                        );
                                    }
                                    if btn.clicked() {
                                        if *tab == RightTab::Changes && has_commit && !has_dirty {
                                            let mut s = self.state.lock().unwrap();
                                            s.selected_commit = None;
                                            s.selected_commit_detail = None;
                                            s.selected_file = None;
                                            s.current_diff = None;
                                            self.repo_ui.last_auto_detail_commit = None;
                                        }
                                        self.repo_ui.right_tab = *tab;
                                    }
                                    ui.add_space(16.0);
                                }
                            });
                        });

                        let content_rect = egui::Rect::from_min_max(
                            egui::Pos2::new(ui.min_rect().min.x, ui.min_rect().min.y + tab_h),
                            ui.max_rect().max,
                        );
                        // Manual panel content keeps long labels from enlarging the resizable sidebar.
                        let mut content = ui.child_ui(
                            content_rect,
                            egui::Layout::top_down(egui::Align::Min),
                            None,
                        );
                        content.set_clip_rect(content_rect);
                        match self.repo_ui.right_tab {
                            RightTab::Changes => self.staging_panel.show(&mut content),
                            RightTab::Details => self.detail_panel.show(&mut content),
                        }
                        ui.set_min_size(ui.available_size());
                        if self.state.lock().unwrap().focused_pane == 3 {
                            focus_ring(ui);
                        }
                    });
            let w = right_response.response.rect.width();
            if self
                .config
                .right_panel_width
                .map_or(true, |old| (old - w).abs() > 1.0)
            {
                self.config.right_panel_width = Some(w);
                self.save_config();
            }
        }

        if self.config.show_diff_panel {
            let diff_response =
                egui::TopBottomPanel::bottom(egui::Id::new(("diff_panel", &self.observed_repo)))
                    .resizable(true)
                    .default_height(self.config.diff_panel_height.unwrap_or(300.0))
                    .height_range(160.0..=480.0)
                    .frame(egui::Frame::none().fill(theme::bg_app()))
                    .show(ctx, |ui| {
                        let r = ui.max_rect();
                        ui.painter().line_segment(
                            [r.left_top(), r.right_top()],
                            Stroke::new(1.0, theme::border()),
                        );
                        if self.commit_graph_view.has_pending_action() {
                            ui.disable();
                        }
                        self.diff_view.show(ui);
                        if self.state.lock().unwrap().focused_pane == 4 {
                            focus_ring(ui);
                        }
                    });
            let h = diff_response.response.rect.height();
            if self
                .config
                .diff_panel_height
                .map_or(true, |old| (old - h).abs() > 1.0)
            {
                self.config.diff_panel_height = Some(h);
                self.save_config();
            }
        }

        // Central panel — commit graph (always)
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(theme::bg_app()))
            .show(ctx, |ui| {
                self.commit_graph_view.show(ui);
                if self.state.lock().unwrap().focused_pane == 2 {
                    focus_ring(ui);
                }
            });

        self.commit_graph_view
            .show_action_confirmation(ctx, &self.config.protected_branches);
        self.show_operation_overlays(ctx);
    }
}

impl RecordeApp {
    fn render_rename_branch_modal(&mut self, ctx: &Context, old_name: String) {
        if self.repo_ui.rename_branch_input.is_empty() {
            self.repo_ui.rename_branch_input = old_name.clone();
        }
        let mut close = false;
        let mut do_rename = false;
        modal_window(ctx, "Rename Branch", |ui| {
            ui.label(styled_label("New name"));
            ui.add_space(4.0);
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.repo_ui.rename_branch_input)
                    .desired_width(f32::INFINITY)
                    .margin(egui::Vec2::new(8.0, 5.0)),
            );
            r.request_focus();
            ui.add_space(12.0);
            let can = !self.repo_ui.rename_branch_input.trim().is_empty()
                && self.repo_ui.rename_branch_input.trim() != old_name.as_str();
            ui.horizontal(|ui| {
                let ok = ui.add_enabled(can, primary_btn("Rename")).clicked()
                    || (can && ui.input(|i| i.key_pressed(Key::Enter)));
                let cancel = ui.add(secondary_btn("Cancel")).clicked()
                    || ui.input(|i| i.key_pressed(Key::Escape));
                if ok {
                    do_rename = true;
                }
                if cancel {
                    close = true;
                }
            });
        });
        if do_rename {
            branches::rename_branch(
                &old_name,
                self.repo_ui.rename_branch_input.trim(),
                self.state.clone(),
            );
            self.repo_ui.rename_branch_input.clear();
            self.state.lock().unwrap().modal = ModalKind::None;
        } else if close {
            self.repo_ui.rename_branch_input.clear();
            self.state.lock().unwrap().modal = ModalKind::None;
        }
    }

    fn render_detached_checkout_modal(&mut self, ctx: &Context, oid: String) {
        let mut close = false;
        let mut do_checkout = false;
        let short = &oid[..7.min(oid.len())];
        modal_window(ctx, "Checkout Commit", |ui| {
            ui.label(
                RichText::new(format!("Checkout commit {}?", short))
                    .color(theme::text_primary())
                    .size(14.0),
            );
            ui.add_space(6.0);
            ui.label(styled_muted("This will put you in detached HEAD state."));
            ui.label(styled_muted(
                "Any commits made here won't be on a named branch.",
            ));
            ui.label(styled_muted("Checkout will stop if it would overwrite local changes. Commit or stash them first."));
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                if ui.add(danger_btn("Checkout")).clicked() {
                    do_checkout = true;
                }
                if ui.add(secondary_btn("Cancel")).clicked()
                    || ui.input(|i| i.key_pressed(Key::Escape))
                {
                    close = true;
                }
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
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.repo_ui.stash_message_input)
                    .desired_width(f32::INFINITY)
                    .margin(egui::Vec2::new(8.0, 5.0))
                    .hint_text("WIP"),
            );
            r.request_focus();
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let ok = ui.add(primary_btn("Stash")).clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Enter));
                let cancel = ui.add(secondary_btn("Cancel")).clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Escape));
                if ok {
                    do_stash = true;
                }
                if cancel {
                    close = true;
                }
            });
        });
        if do_stash {
            let msg = if self.repo_ui.stash_message_input.trim().is_empty() {
                "WIP".to_string()
            } else {
                self.repo_ui.stash_message_input.trim().to_string()
            };
            branches::stash_save_with_message(&msg, self.state.clone());
            self.repo_ui.stash_message_input = "WIP".into();
            self.state.lock().unwrap().modal = ModalKind::None;
        } else if close {
            self.repo_ui.stash_message_input = "WIP".into();
            self.state.lock().unwrap().modal = ModalKind::None;
        }
    }

    fn render_stash_branch_modal(&mut self, ctx: &Context, stash_index: usize) {
        let mut close = false;
        let mut do_branch = false;
        modal_window(ctx, "Create Branch From Stash", |ui| {
            ui.label(styled_label("Branch name"));
            ui.add_space(4.0);
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.repo_ui.stash_branch_input)
                    .desired_width(f32::INFINITY)
                    .margin(egui::Vec2::new(8.0, 5.0))
                    .hint_text("e.g. feat/my-feature"),
            );
            r.request_focus();
            ui.add_space(4.0);
            ui.label(styled_muted(
                "Creates a branch from the stash's base commit, applies the stash, and drops it.",
            ));
            ui.add_space(12.0);
            let can = !self.repo_ui.stash_branch_input.trim().is_empty();
            ui.horizontal(|ui| {
                let ok = ui.add_enabled(can, primary_btn("Create Branch")).clicked()
                    || (can && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                let cancel = ui.add(secondary_btn("Cancel")).clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Escape));
                if ok {
                    do_branch = true;
                }
                if cancel {
                    close = true;
                }
            });
        });
        if do_branch {
            branches::stash_branch(
                self.repo_ui.stash_branch_input.trim(),
                stash_index,
                self.state.clone(),
            );
            self.repo_ui.stash_branch_input.clear();
            self.state.lock().unwrap().modal = ModalKind::None;
        } else if close {
            self.repo_ui.stash_branch_input.clear();
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
                    .margin(egui::Vec2::new(8.0, 5.0))
                    .hint_text("https://github.com/owner/repo.git"),
            );
            r.request_focus();
            ui.add_space(8.0);
            ui.label(styled_label("Destination"));
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let dest_str = self
                    .clone_dest
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "Choose folder…".into());
                ui.label(
                    RichText::new(dest_str)
                        .color(theme::text_secondary())
                        .size(13.0),
                );
                if ui.add(secondary_btn("Browse")).clicked() {
                    if let Some(p) = rfd::FileDialog::new().pick_folder() {
                        self.clone_dest = Some(p);
                    }
                }
            });
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                ui.label("History depth (0 = full history)");
                ui.add(egui::DragValue::new(&mut self.clone_depth).range(0..=1000000));
            });
            if self.clone_depth > 0 {
                ui.label(styled_muted("Shallow clones contain the default branch's recent history. Fetch full history later in repository settings."));
            }
            let can_clone = !self.clone_url.trim().is_empty() && self.clone_dest.is_some();
            ui.horizontal(|ui| {
                let ok = ui.add_enabled(can_clone, primary_btn("Clone")).clicked()
                    || (can_clone && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                let cancel = ui.add(secondary_btn("Cancel")).clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Escape));
                if ok {
                    do_clone = true;
                }
                if cancel {
                    close = true;
                }
            });
        });

        if do_clone {
            if let Some(dest) = self.clone_dest.clone() {
                let mut dest_path = dest;
                // Append the repo name from the URL as the subdirectory
                let repo_name = self
                    .clone_url
                    .trim_end_matches('/')
                    .trim_end_matches(".git")
                    .rsplit('/')
                    .next()
                    .unwrap_or("repo")
                    .to_string();
                if repo_name.is_empty()
                    || repo_name == "."
                    || repo_name == ".."
                    || repo_name.contains(['\\', ':'])
                {
                    self.state.lock().unwrap().error =
                        Some("The URL does not provide a valid folder name.".into());
                    return;
                }
                dest_path.push(&repo_name);
                self.config.add_recent(dest_path.clone());
                lifecycle::clone_repo(
                    self.clone_url.trim(),
                    dest_path,
                    self.clone_depth,
                    self.state.clone(),
                );
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
            ui.label(styled_label(
                "Choose an empty folder to initialise as a git repository:",
            ));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let path_str = self
                    .init_repo_path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "No folder selected".into());
                ui.label(
                    RichText::new(path_str)
                        .color(theme::text_secondary())
                        .size(13.0),
                );
                if ui.add(secondary_btn("Browse")).clicked() {
                    if let Some(p) = rfd::FileDialog::new().pick_folder() {
                        self.init_repo_path = Some(p);
                    }
                }
            });
            ui.add_space(16.0);
            let can_init = self.init_repo_path.is_some();
            ui.horizontal(|ui| {
                let ok = ui
                    .add_enabled(can_init, primary_btn("Initialize"))
                    .clicked();
                let cancel = ui.add(secondary_btn("Cancel")).clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Escape));
                if ok {
                    do_init = true;
                }
                if cancel {
                    close = true;
                }
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
                    ui.heading(
                        RichText::new("Reflog")
                            .size(14.0)
                            .color(theme::text_primary()),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(Button::new("✕").fill(Color32::TRANSPARENT))
                            .clicked()
                        {
                            close = true;
                        }
                    });
                });
                ui.add_space(8.0);
                ui.separator();

                egui::ScrollArea::vertical()
                    .max_height(340.0)
                    .show(ui, |ui| {
                        for entry in &entries {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(&entry.short_oid)
                                        .monospace()
                                        .color(theme::accent_teal())
                                        .size(12.0),
                                );
                                ui.add_space(6.0);
                                ui.label(
                                    RichText::new(&entry.refname)
                                        .color(theme::text_muted())
                                        .size(12.0),
                                );
                                ui.add_space(6.0);
                                ui.label(
                                    RichText::new(&entry.message)
                                        .color(theme::text_secondary())
                                        .size(12.0),
                                );
                            })
                            .response
                            .context_menu(|ui| {
                                ui.set_min_width(180.0);
                                if ui.button("Recover as Branch…").clicked() {
                                    self.repo_ui.reflog_recover_oid = Some(entry.oid.clone());
                                    self.repo_ui.reflog_recover_input =
                                        format!("recover-{}", &entry.short_oid);
                                    ui.close_menu();
                                }
                                if ui.button("Copy SHA").clicked() {
                                    ui.output_mut(|o| o.copied_text = entry.oid.clone());
                                    ui.close_menu();
                                }
                            });
                        }
                        if entries.is_empty() {
                            ui.label(
                                RichText::new("No reflog entries.")
                                    .color(theme::text_muted())
                                    .size(13.0),
                            );
                        }
                    });

                if self.repo_ui.reflog_recover_oid.is_some() {
                    ui.separator();
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("Branch name for recovery:")
                            .color(theme::text_secondary())
                            .size(13.0),
                    );
                    ui.add_space(4.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.repo_ui.reflog_recover_input)
                            .desired_width(f32::INFINITY)
                            .margin(egui::Vec2::new(8.0, 5.0)),
                    );
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        let can = !self.repo_ui.reflog_recover_input.trim().is_empty();
                        if ui.add_enabled(can, primary_btn("Recover")).clicked() {
                            if let Some(ref oid) = self.repo_ui.reflog_recover_oid {
                                recover_request = Some((
                                    oid.clone(),
                                    self.repo_ui.reflog_recover_input.trim().to_string(),
                                ));
                            }
                        }
                        if ui.add(secondary_btn("Cancel")).clicked() {
                            self.repo_ui.reflog_recover_oid = None;
                        }
                    });
                }

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(secondary_btn("Close")).clicked() {
                            close = true;
                        }
                    });
                });
            });

        if let Some((oid, branch)) = recover_request {
            if let Some(ref p) = repo_path {
                crate::git::mutation::spawn_operation(p.clone(), self.state.clone(), move |path| {
                    reflog::recover(path, &oid, &branch)
                });
            }
            self.repo_ui.reflog_recover_oid = None;
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
            (
                s.repo_path.clone(),
                s.bisect_in_progress,
                s.bisect_current_oid.clone(),
                s.selected_commit.clone(),
            )
        };

        let mut close = false;
        let mut action: Option<String> = None; // "good" | "bad" | "reset"

        modal_window(ctx, "Bisect", |ui| {
            ui.set_min_width(380.0);

            if !bisect_active {
                ui.label(
                    RichText::new("Start a bisect session to find which commit introduced a bug.")
                        .color(theme::text_secondary())
                        .size(13.0),
                );
                ui.add_space(8.0);
                ui.label(RichText::new("Select the known-bad commit in the graph, then mark it bad. Select a known-good commit and mark it good.").color(theme::text_muted()).size(12.0));
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.add(primary_btn("Start Bisect")).clicked() {
                        action = Some("start".into());
                    }
                    if ui.add(secondary_btn("Cancel")).clicked() {
                        close = true;
                    }
                });
            } else {
                ui.label(
                    RichText::new("Bisect in progress")
                        .color(theme::accent_teal())
                        .size(13.0)
                        .strong(),
                );
                ui.add_space(6.0);

                if let Some(ref oid) = bisect_oid {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Testing:")
                                .color(theme::text_muted())
                                .size(13.0),
                        );
                        ui.label(
                            RichText::new(&oid[..7.min(oid.len())])
                                .monospace()
                                .color(theme::accent_teal())
                                .size(13.0),
                        );
                    });
                    ui.add_space(4.0);
                }

                let cur = selected_oid.as_deref().unwrap_or("");
                let short = if cur.len() >= 7 { &cur[..7] } else { cur };
                if cur.is_empty() {
                    ui.label(
                        RichText::new("Select a commit in the graph, then mark it good or bad.")
                            .color(theme::text_muted())
                            .size(12.0),
                    );
                } else {
                    ui.label(
                        RichText::new(format!("Selected: {}", short))
                            .color(theme::text_secondary())
                            .size(13.0),
                    );
                }
                ui.add_space(10.0);

                ui.horizontal(|ui| {
                    let has_sel = !cur.is_empty();
                    if ui
                        .add_enabled(
                            has_sel,
                            Button::new(RichText::new("Mark Good").color(Color32::WHITE))
                                .fill(theme::accent_green())
                                .min_size([110.0, 26.0].into()),
                        )
                        .clicked()
                    {
                        action = Some("good".into());
                    }
                    ui.add_space(6.0);
                    if ui
                        .add_enabled(
                            has_sel,
                            Button::new(RichText::new("Mark Bad").color(Color32::WHITE))
                                .fill(theme::accent_red())
                                .min_size([110.0, 26.0].into()),
                        )
                        .clicked()
                    {
                        action = Some("bad".into());
                    }
                    ui.add_space(12.0);
                    if ui.add(secondary_btn("Reset")).clicked() {
                        action = Some("reset".into());
                    }
                });
            }
        });

        if let Some(act) = action {
            let reset = act == "reset";
            if let Some(ref p) = repo_path {
                let sel = selected_oid.unwrap_or_default();
                let state = self.state.clone();
                let epoch = state.lock().unwrap().repo_epoch;
                crate::git::mutation::spawn_operation(p.clone(), self.state.clone(), move |path| {
                    let next = match act.as_str() {
                        "start" => bisect::start(path).map(|_| None),
                        "good" => bisect::mark_good(path, &sel),
                        "bad" => bisect::mark_bad(path, &sel),
                        "reset" => bisect::reset(path).map(|_| None),
                        _ => Ok(None),
                    }?;
                    let mut s = state.lock().unwrap();
                    if s.repo_path.as_deref() == Some(path) && s.repo_epoch == epoch {
                        if let Some(next) = next {
                            s.selected_commit = Some(next);
                        }
                    }
                    Ok(())
                });
            }
            if reset {
                self.state.lock().unwrap().modal = ModalKind::None;
            }
        }
        if close {
            self.state.lock().unwrap().modal = ModalKind::None;
        }
    }

    fn render_compare_modal(&mut self, ctx: &Context, base_oid: String, compare_oid: String) {
        use crate::git::operations::{load_compare_diff, load_compare_files};

        let (compare_files, compare_diff, compare_sel) = {
            let s = self.state.lock().unwrap();
            (
                s.compare_files.clone(),
                s.compare_diff.clone(),
                s.compare_selected_file.clone(),
            )
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
                    ui.label(RichText::new("Base:").color(theme::text_muted()).size(12.0));
                    ui.label(
                        RichText::new(&base_oid[..7.min(base_oid.len())])
                            .monospace()
                            .color(theme::accent_teal())
                            .size(12.0),
                    );
                    ui.add_space(12.0);
                    ui.label(
                        RichText::new("Compare:")
                            .color(theme::text_muted())
                            .size(12.0),
                    );
                    ui.label(
                        RichText::new(&compare_oid[..7.min(compare_oid.len())])
                            .monospace()
                            .color(theme::accent_teal())
                            .size(12.0),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(Button::new("✕").fill(Color32::TRANSPARENT))
                            .clicked()
                        {
                            close = true;
                        }
                    });
                });
                ui.separator();

                ui.columns(2, |cols| {
                    // Left: file list
                    cols[0].label(
                        RichText::new(format!("Changed files ({})", compare_files.len()))
                            .color(theme::text_muted())
                            .size(12.0)
                            .strong(),
                    );
                    cols[0].add_space(4.0);
                    egui::ScrollArea::vertical()
                        .id_source("cmp_files")
                        .max_height(460.0)
                        .show(&mut cols[0], |ui| {
                            for file in &compare_files {
                                let is_sel = compare_sel.as_deref() == Some(&file.path);
                                let color = if is_sel {
                                    theme::text_primary()
                                } else {
                                    theme::text_secondary()
                                };
                                let resp = ui
                                    .selectable_label(
                                        is_sel,
                                        RichText::new(
                                            file.path.rsplit('/').next().unwrap_or(&file.path),
                                        )
                                        .color(color)
                                        .size(13.0),
                                    )
                                    .on_hover_text(&file.path);
                                if resp.clicked() {
                                    select_file = Some(file.path.clone());
                                }
                            }
                            if compare_files.is_empty() {
                                ui.label(
                                    RichText::new("Loading…")
                                        .color(theme::text_muted())
                                        .size(13.0),
                                );
                            }
                        });

                    // Right: diff view
                    if let Some(ref diff) = compare_diff {
                        cols[1].label(
                            RichText::new(format!("+{}  −{}", diff.additions, diff.deletions))
                                .color(theme::text_muted())
                                .size(12.0),
                        );
                        cols[1].add_space(4.0);
                        egui::ScrollArea::vertical()
                            .id_source("cmp_diff")
                            .max_height(460.0)
                            .show(&mut cols[1], |ui| {
                                for hunk in &diff.hunks {
                                    ui.label(
                                        RichText::new(&hunk.header)
                                            .monospace()
                                            .color(theme::accent_purple())
                                            .size(12.0)
                                            .background_color(theme::bg_diff_hunk()),
                                    );
                                    for line in &hunk.lines {
                                        let (color, bg) = match line.origin {
                                            '+' => (theme::accent_green(), theme::bg_diff_add()),
                                            '-' => (theme::accent_red(), theme::bg_diff_del()),
                                            _ => (theme::text_secondary(), Color32::TRANSPARENT),
                                        };
                                        let text = format!(
                                            "{}{}",
                                            line.origin,
                                            line.content.trim_end_matches('\n')
                                        );
                                        let label = egui::RichText::new(&text)
                                            .monospace()
                                            .color(color)
                                            .size(12.0);
                                        if bg != Color32::TRANSPARENT {
                                            ui.label(label.background_color(bg));
                                        } else {
                                            ui.label(label);
                                        }
                                    }
                                }
                            });
                    } else if compare_sel.is_some() {
                        cols[1].label(
                            RichText::new("Loading diff…")
                                .color(theme::text_muted())
                                .size(13.0),
                        );
                    } else {
                        cols[1].label(
                            RichText::new("Select a file to view its diff.")
                                .color(theme::text_muted())
                                .size(13.0),
                        );
                    }
                });
            });

        if let Some(fp) = select_file {
            self.repo_ui.compare_file_selected = Some(fp.clone());
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
    fn render_branch_picker(
        &mut self,
        ctx: &Context,
        title: &str,
        hint: &str,
        on_confirm: impl Fn(&mut Self, String),
    ) {
        let branches_list: Vec<String> = {
            let s = self.state.lock().unwrap();
            let head = s.head_branch.clone().unwrap_or_default();
            s.commit_graph
                .as_ref()
                .map(|g| {
                    let mut refs: Vec<&str> = g
                        .nodes
                        .iter()
                        .flat_map(|n| n.refs.iter().map(|r| r.as_str()))
                        .collect();
                    refs.sort_unstable();
                    refs.dedup();
                    refs.iter()
                        .filter(|r| !r.contains('/') && **r != head.as_str())
                        .map(|s| s.to_string())
                        .collect()
                })
                .unwrap_or_default()
        };

        let mut close = false;
        let mut chosen: Option<String> = None;

        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(egui::Frame::window(&ctx.style()).inner_margin(egui::Margin::same(16.0)))
            .show(ctx, |ui| {
                ui.set_min_width(340.0);
                ui.label(
                    RichText::new(title)
                        .color(theme::text_primary())
                        .size(15.0)
                        .strong(),
                );
                ui.add_space(6.0);
                ui.label(styled_muted(hint));
                ui.add_space(8.0);
                egui::ScrollArea::vertical()
                    .max_height(280.0)
                    .show(ui, |ui| {
                        for name in &branches_list {
                            if ui
                                .selectable_label(
                                    false,
                                    RichText::new(name)
                                        .color(theme::text_secondary())
                                        .size(13.0),
                                )
                                .clicked()
                            {
                                chosen = Some(name.clone());
                            }
                        }
                        if branches_list.is_empty() {
                            ui.label(styled_muted("No other local branches."));
                        }
                    });
                ui.add_space(8.0);
                if ui.add(secondary_btn("Cancel")).clicked()
                    || ui.input(|i| i.key_pressed(Key::Escape))
                {
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
        let mut do_split = false;

        egui::TopBottomPanel::top("in_progress_banner")
            .exact_height(36.0)
            .frame(
                egui::Frame::none()
                    .fill(Color32::from_rgb(0x4A, 0x32, 0x00))
                    .inner_margin(egui::Margin::symmetric(16.0, 0.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.label(RichText::new("⟳").color(theme::accent_yellow()).size(14.0));
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(label)
                            .color(theme::accent_yellow())
                            .size(13.0)
                            .strong(),
                    );
                    ui.add_space(16.0);
                    ui.label(styled_muted(if caps.rebase_split {
                        "Stage files or lines, commit each part, then continue."
                    } else if caps.rebase_edit {
                        "Amend this commit, or split it into smaller commits."
                    } else {
                        "Resolve conflicts, stage changes, then:"
                    }));
                    if caps.rebase_edit && !caps.rebase_split {
                        if ui.button("Split commit").on_hover_text("Uncommit this edit step, keep its files, then stage and commit each part in Changes.").clicked() {
                            do_split = true;
                        }
                    }
                    ui.add_space(10.0);
                    if ui
                        .add(
                            Button::new(RichText::new("Continue").color(Color32::WHITE).size(13.0))
                                .fill(theme::accent_teal())
                                .rounding(5.0)
                                .min_size(egui::Vec2::new(72.0, 24.0)),
                        )
                        .clicked()
                    {
                        do_continue = true;
                    }
                    if show_skip && !caps.rebase_split {
                        ui.add_space(4.0);
                        if ui
                            .add(
                                Button::new(
                                    RichText::new("Skip")
                                        .color(theme::text_secondary())
                                        .size(13.0),
                                )
                                .fill(Color32::TRANSPARENT)
                                .stroke(Stroke::new(1.0, theme::border()))
                                .rounding(5.0)
                                .min_size(egui::Vec2::new(52.0, 24.0)),
                            )
                            .clicked()
                        {
                            do_skip = true;
                        }
                    }
                    ui.add_space(4.0);
                    if ui
                        .add(
                            Button::new(
                                RichText::new("Abort").color(theme::accent_red()).size(13.0),
                            )
                            .fill(Color32::TRANSPARENT)
                            .stroke(Stroke::new(1.0, theme::accent_red().gamma_multiply(0.5)))
                            .rounding(5.0)
                            .min_size(egui::Vec2::new(52.0, 24.0)),
                        )
                        .clicked()
                    {
                        do_abort = true;
                    }
                });
            });

        if do_split {
            {
                let mut state = self.state.lock().unwrap();
                state.selected_commit = None;
                state.selected_commit_detail = None;
                state.selected_file = None;
                state.current_diff = None;
                state.focused_pane = 3;
            }
            rebase::split_paused_commit(self.state.clone());
        }
        if do_continue {
            if caps.rebase_in_progress {
                rebase::continue_rebase(self.state.clone());
            } else if caps.merge_in_progress {
                merge::continue_merge(self.state.clone());
            } else if caps.cherry_pick_in_progress {
                branches::continue_cherry_pick(self.state.clone());
            } else if caps.revert_in_progress {
                branches::continue_revert(self.state.clone());
            }
        }
        if do_skip && caps.rebase_in_progress {
            rebase::skip_rebase(self.state.clone());
        }
        if do_abort {
            if caps.rebase_in_progress {
                rebase::abort_rebase(self.state.clone());
            } else if caps.merge_in_progress {
                merge::abort_merge(self.state.clone());
            } else if caps.cherry_pick_in_progress {
                branches::abort_cherry_pick(self.state.clone());
            } else if caps.revert_in_progress {
                branches::abort_revert(self.state.clone());
            }
        }
    }

    fn poll_external_changes(&mut self, ctx: &Context) {
        let focused = ctx.input(|i| i.focused);
        let regained_focus = focused && !self.was_focused;
        self.was_focused = focused;
        let (path, busy, sequence) = {
            let s = self.state.lock().unwrap();
            (
                s.repo_path.clone(),
                s.repo_load_path.is_some()
                    || s.remote_op.is_some()
                    || !s.active_ops.is_empty()
                    || !s.local_jobs.is_empty(),
                s.capabilities.as_ref().is_some_and(|c| {
                    c.rebase_in_progress
                        || c.merge_in_progress
                        || c.cherry_pick_in_progress
                        || c.revert_in_progress
                }),
            )
        };
        let Some(path) = path else {
            self.repo_watch = None;
            return;
        };
        if self.repo_watch.as_ref().is_none_or(|w| w.path() != path) {
            self.repo_watch = Some(crate::git::watch::RepoWatch::new(path.clone()));
        }
        if let Some(refresh) = self
            .repo_watch
            .as_mut()
            .and_then(|w| w.poll(busy, regained_focus))
        {
            match refresh {
                crate::git::watch::Refresh::History => {
                    crate::git::operations::refresh_repo(self.state.clone())
                }
                crate::git::watch::Refresh::Worktree => {
                    crate::git::operations::refresh_worktree(self.state.clone())
                }
            }
        }
        let changed = self.auto_fetch_repo.as_ref() != Some(&path);
        let due = changed
            || self.last_auto_fetch.elapsed().as_secs()
                >= self.config.auto_fetch_interval_secs.max(30) as u64;
        if self.config.auto_fetch && due && !busy && !sequence && !self.runner.has_jobs(&path) {
            self.runner
                .submit(crate::git::job::JobKind::FetchAll, path.clone());
            self.last_auto_fetch = std::time::Instant::now();
            self.auto_fetch_repo = Some(path.clone());
        }
        if !self.config.auto_fetch {
            self.last_auto_fetch = std::time::Instant::now();
            self.auto_fetch_repo = Some(path);
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }

    fn apply_runner_progress(&mut self) {
        let events = self.runner.drain_progress();
        if events.is_empty() {
            return;
        }

        let refresh = crate::git::job::apply_progress(&mut self.state.lock().unwrap(), events);
        if refresh {
            crate::git::branches::reload_repo(self.state.clone());
        }
    }

    fn save_config(&mut self) {
        if let Some(path) = &self.observed_repo {
            self.config.remember_layout(path);
        }
        self.config.save();
    }

    fn persist_navigation(&mut self) {
        let navigation = {
            let state = self.state.lock().unwrap();
            state
                .repo_path
                .clone()
                .map(|path| (path, crate::config::RepoNavigation::capture(&state)))
        };
        if let Some((path, navigation)) = navigation {
            let preferences = self.diff_view.preferences();
            let settings = self
                .config
                .per_repo
                .entry(path.display().to_string())
                .or_default();
            if settings.navigation.as_ref() != Some(&navigation)
                || settings.diff_preferences != preferences
            {
                settings.navigation = Some(navigation);
                settings.diff_preferences = preferences;
                if let Err(error) = self.config.try_save() {
                    self.state.lock().unwrap().error = Some(error);
                }
            }
        }
    }

    fn sync_repo_session(&mut self) {
        let open_commit_settings = {
            let mut state = self.state.lock().unwrap();
            std::mem::take(&mut state.pending_commit_settings)
        };
        if open_commit_settings {
            self.open_settings(SettingsTab::Repository);
        }
        let active = self.state.lock().unwrap().repo_path.clone();
        if active != self.observed_repo {
            self.command_palette_open = false;
            self.branch_switcher_open = false;
            self.commit_graph_view.cancel_action();
            let previous = std::mem::take(&mut self.repo_ui);
            if let Some(path) = &self.observed_repo {
                self.repo_sessions.insert(path.clone(), previous);
                self.config.remember_layout(path);
                let state = self.state.lock().unwrap();
                let settings = self
                    .config
                    .per_repo
                    .entry(path.display().to_string())
                    .or_default();
                settings.navigation = state
                    .repo_views
                    .get(path)
                    .map(crate::config::RepoNavigation::from_view);
                settings.diff_preferences = self.diff_view.preferences();
            }
            if let Some(path) = &active {
                self.repo_ui = self.repo_sessions.remove(path).unwrap_or_default();
                self.config.restore_layout(path);
                if !self.config.open_repos.contains(path) {
                    self.config.open_repos.push(path.clone());
                }
                self.config.active_repo = Some(path.clone());
                let layout = self.config.get_repo_config(path);
                self.diff_view.restore_preferences(&layout.diff_preferences);
                self.commit_graph_view.reset_column_widths();
                self.commit_graph_view.set_column_widths(
                    layout.refs_w,
                    layout.graph_w,
                    layout.time_w,
                    layout.hash_w,
                );
                self.repo_ui.pending_recovery_id = None;
                self.repo_ui.remove_worktree_pending = None;
                self.repo_ui.issues_fetched = false;
                self.repo_ui.ci_fetched = false;
                self.repo_ui.releases_fetched = false;
                let identity = git_identity(Some(path));
                self.repo_ui.repo_git_name_input = identity.0;
                self.repo_ui.repo_git_email_input = identity.1;
                self.config.save();
            }
            self.observed_repo = active.clone();
        }
    }

    fn render_repo_tabs(&mut self, ctx: &Context) {
        let active = self.state.lock().unwrap().repo_path.clone();
        // A failed replacement load keeps the old repository usable and visible.
        if let Some(path) = &active {
            if self.state.lock().unwrap().repo_load_path.is_none()
                && !self.config.open_repos.contains(path)
            {
                self.config.open_repos.push(path.clone());
                self.save_config();
            }
        }
        let mut open = None;
        let mut close = None;
        let mut workspace = None;
        egui::TopBottomPanel::top("repository_tabs")
            .exact_height(44.0)
            .frame(
                egui::Frame::none()
                    .fill(theme::bg_titlebar())
                    .inner_margin(egui::Margin::symmetric(16.0, 6.0)),
            )
            .show(ctx, |ui| {
                if self.commit_graph_view.has_pending_action() {
                    ui.disable();
                }
                egui::ScrollArea::horizontal().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if let Some(path) = &active {
                            let favorite = self.config.favorites.contains(path);
                            if ui
                                .small_button(if favorite { "★" } else { "☆" })
                                .on_hover_text("Toggle repository favorite")
                                .clicked()
                            {
                                if favorite {
                                    self.config.favorites.retain(|p| p != path);
                                } else {
                                    self.config.favorites.push(path.clone());
                                }
                                self.save_config();
                            }
                        }
                        egui::ComboBox::from_id_source("repo_workspaces")
                            .selected_text("Workspaces")
                            .show_ui(ui, |ui| {
                                ui.strong("Favorites");
                                for path in &self.config.favorites {
                                    if ui.button(path.display().to_string()).clicked() {
                                        open = Some(path.clone());
                                    }
                                }
                                ui.separator();
                                ui.strong("Saved workspaces");
                                for (name, paths) in &self.config.workspaces {
                                    if ui.button(name).clicked() {
                                        workspace = Some(paths.clone());
                                    }
                                }
                                ui.separator();
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.workspace_name)
                                        .hint_text("Workspace name"),
                                );
                                if ui
                                    .add_enabled(
                                        !self.workspace_name.trim().is_empty()
                                            && !self.config.open_repos.is_empty(),
                                        Button::new("Save open tabs as workspace"),
                                    )
                                    .clicked()
                                {
                                    self.config.workspaces.insert(
                                        self.workspace_name.trim().to_string(),
                                        self.config.open_repos.clone(),
                                    );
                                    self.save_config();
                                }
                                if self
                                    .config
                                    .workspaces
                                    .contains_key(self.workspace_name.trim())
                                    && ui.button("Remove saved workspace").clicked()
                                {
                                    self.config.workspaces.remove(self.workspace_name.trim());
                                    self.save_config();
                                }
                            });
                        for path in &self.config.open_repos {
                            let label = path
                                .file_name()
                                .unwrap_or(path.as_os_str())
                                .to_string_lossy();
                            if ui
                                .selectable_label(active.as_ref() == Some(path), label.as_ref())
                                .on_hover_text(path.display().to_string())
                                .clicked()
                            {
                                open = Some(path.clone());
                            }
                            if self.config.open_repos.len() > 1
                                && ui
                                    .small_button("×")
                                    .on_hover_text("Close repository tab")
                                    .clicked()
                            {
                                close = Some(path.clone());
                            }
                            ui.separator();
                        }
                        if ui
                            .small_button("+")
                            .on_hover_text("Open repository")
                            .clicked()
                        {
                            open = rfd::FileDialog::new().pick_folder();
                        }
                    });
                });
            });
        if let Some(paths) = workspace {
            if let Some(first) = paths.first() {
                open = Some(first.clone());
                self.config.open_repos = paths;
                self.save_config();
            }
        }
        if let Some(path) = close {
            self.config.open_repos.retain(|p| p != &path);
            if active.as_ref() == Some(&path) {
                open = self.config.open_repos.last().cloned();
            }
            self.save_config();
        }
        if let Some(path) = open {
            self.config.add_recent(path.clone());
            load_repo(path, self.state.clone());
        }
    }

    fn show_operation_overlays(&mut self, ctx: &Context) {
        let (legacy_op, active_ops, error) = {
            let s = self.state.lock().unwrap();
            (
                s.remote_op.clone(),
                s.active_ops
                    .iter()
                    .map(|o| {
                        let pct = o.progress_pct;
                        let msg = if o.progress_msg.is_empty() {
                            o.label.clone()
                        } else {
                            o.progress_msg.clone()
                        };
                        (format!("remote_{}", o.id), msg, pct, o.cancel.clone())
                    })
                    .chain(s.local_jobs.iter().map(|job| {
                        let status = if job.running { "" } else { "Queued: " };
                        (
                            format!("local_{}", job.id),
                            format!(
                                "{status}{} ({})",
                                job.label,
                                job.path.file_name().unwrap_or_default().to_string_lossy()
                            ),
                            None,
                            job.cancel.clone(),
                        )
                    }))
                    .collect::<Vec<_>>(),
                s.error.as_deref().map(crate::auth::redact::text),
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
                        .fill(theme::bg_elevated())
                        .stroke(Stroke::new(1.0, theme::accent_teal()))
                        .rounding(8.0)
                        .inner_margin(egui::Margin::symmetric(14.0, 10.0))
                        .show(ui, |ui| {
                            ui.horizontal_centered(|ui| {
                                ui.spinner();
                                ui.add_space(8.0);
                                ui.label(RichText::new(msg.as_str()).color(theme::text_primary()).size(14.0).strong());
                                if let Some(p) = pct {
                                    ui.add_space(6.0);
                                    ui.label(RichText::new(format!("{}%", p)).color(theme::text_muted()).size(13.0));
                                }
                                ui.add_space(10.0);
                                if ui.add(secondary_btn("Cancel")).on_hover_text("Stops queued work or running Git commands. Completed changes remain; short file operations finish before cancellation takes effect.").clicked() {
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
                        .fill(theme::bg_elevated())
                        .stroke(Stroke::new(1.0, theme::accent_teal()))
                        .rounding(8.0)
                        .inner_margin(egui::Margin::symmetric(14.0, 10.0))
                        .show(ui, |ui| {
                            ui.horizontal_centered(|ui| {
                                ui.spinner();
                                ui.add_space(8.0);
                                ui.label(
                                    RichText::new(op)
                                        .color(theme::text_primary())
                                        .size(14.0)
                                        .strong(),
                                );
                            });
                        });
                });
        }

        // ── Discard confirmation ──────────────────────────────────────────
        let pending = { self.state.lock().unwrap().pending_discard.clone() };
        if let Some(ref pd) = pending {
            let y_off =
                146.0 + active_ops.len() as f32 * 52.0 + if has_legacy { 52.0 } else { 0.0 };
            let desc = match pd {
                PendingDiscard::File { path } => format!("Discard all changes to \"{}\"?", path.rsplit('/').next().unwrap_or(path)),
                PendingDiscard::Selection { diff, lines, .. } => format!("Discard {} selected changes in {}? A recovery snapshot will be saved.", lines.len(), diff.file_path),
                PendingDiscard::ResetHard => "Hard reset: discard the last commit AND all staged/unstaged changes? This cannot be undone.".to_string(),
            };
            let mut confirm = false;
            let mut cancel = false;
            egui::Area::new(egui::Id::new("discard_confirm_overlay"))
                .anchor(egui::Align2::CENTER_TOP, [0.0, y_off])
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    egui::Frame::none()
                        .fill(Color32::from_rgb(74, 24, 28))
                        .stroke(Stroke::new(1.5, theme::accent_red()))
                        .rounding(8.0)
                        .inner_margin(egui::Margin::symmetric(16.0, 12.0))
                        .show(ui, |ui| {
                            ui.set_max_width(640.0);
                            ui.horizontal_centered(|ui| {
                                ui.label(RichText::new(&desc).color(Color32::WHITE).size(13.0));
                                ui.add_space(12.0);
                                if ui.add(danger_btn("Discard")).clicked() {
                                    confirm = true;
                                }
                                if ui.add(secondary_btn("Cancel")).clicked() {
                                    cancel = true;
                                }
                            });
                        });
                });
            if confirm {
                match pd {
                    PendingDiscard::File { path } => {
                        staging::discard_file(path, self.state.clone())
                    }
                    PendingDiscard::Selection { repo, diff, lines } => staging::discard_selection(
                        repo.clone(),
                        diff.clone(),
                        lines.clone(),
                        self.state.clone(),
                    ),
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
                        .stroke(Stroke::new(1.5, theme::accent_red()))
                        .rounding(8.0)
                        .inner_margin(egui::Margin::symmetric(16.0, 12.0))
                        .show(ui, |ui| {
                            ui.set_max_width(720.0);
                            ui.horizontal_centered(|ui| {
                                ui.label(
                                    RichText::new("Operation blocked")
                                        .color(Color32::WHITE)
                                        .size(13.5)
                                        .strong(),
                                );
                                ui.add_space(10.0);
                                ui.label(RichText::new(error).color(Color32::WHITE).size(13.0));
                                ui.add_space(10.0);
                                if ui.add(secondary_btn("Dismiss")).clicked() {
                                    self.state.lock().unwrap().error = None;
                                }
                            });
                        });
                });
        }
    }

    fn show_welcome(&mut self, ctx: &Context) {
        let mut action = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(theme::bg_app()))
            .show(ctx, |ui| {
                let width = 620.0_f32.min(ui.available_width() - 48.0);
                let left = ui.max_rect().center().x - width / 2.0;
                let top = ui.max_rect().top() + (ui.available_height() * 0.12).max(24.0);
                let rect = egui::Rect::from_min_size(
                    egui::pos2(left, top),
                    egui::vec2(width, ui.available_height() - 40.0),
                );
                ui.allocate_ui_at_rect(rect, |ui| {
                    ui.label(
                        crate::icons::rich(crate::icons::SOURCE, 40.0).color(theme::accent_teal()),
                    );
                    ui.add_space(16.0);
                    ui.label(
                        RichText::new("Your next commit starts here.")
                            .size(30.0)
                            .strong()
                            .color(theme::text_primary()),
                    );
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(
                            "Explore your history. Review your changes. Ship with confidence.",
                        )
                        .size(14.0)
                        .color(theme::text_secondary()),
                    );
                    ui.add_space(24.0);
                    ui.horizontal_wrapped(|ui| {
                        if ui.add(primary_btn("Open repository…")).clicked() {
                            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                                action = Some(ToolbarAction::OpenRepo(path));
                            }
                        }
                        if ui.add(secondary_btn("Clone repository…")).clicked() {
                            action = Some(ToolbarAction::CloneRepo);
                        }
                        if ui.add(secondary_btn("Create repository…")).clicked() {
                            action = Some(ToolbarAction::InitRepo);
                        }
                    });
                    ui.add_space(32.0);
                    if !self.config.recent_repos.is_empty() {
                        ui.label(
                            RichText::new("Pick up where you left off")
                                .size(15.0)
                                .strong(),
                        );
                        ui.add_space(12.0);
                        egui::ScrollArea::vertical()
                            .max_height(240.0)
                            .show(ui, |ui| {
                                for path in self.config.recent_repos.iter().take(8) {
                                    let name =
                                        path.file_name().unwrap_or_default().to_string_lossy();
                                    if ui
                                        .add_sized(
                                            [width, 38.0],
                                            Button::new(format!("{name}    {}", path.display()))
                                                .truncate()
                                                .fill(theme::bg_panel())
                                                .stroke(Stroke::new(1.0, theme::border())),
                                        )
                                        .on_hover_text(path.display().to_string())
                                        .clicked()
                                    {
                                        action = Some(ToolbarAction::OpenRepo(path.clone()));
                                    }
                                }
                            });
                    }
                    ui.add_space(24.0);
                    ui.label(
                        RichText::new("Ctrl/Cmd+K  Command palette     Ctrl/Cmd+B  Switch branch")
                            .size(12.0)
                            .color(theme::text_muted()),
                    );
                });
            });
        if let Some(action) = action {
            self.handle_toolbar(ctx, action);
        }
    }

    fn show_menu_contents(&mut self, ctx: &Context, ui: &mut egui::Ui) {
        if self.commit_graph_view.has_pending_action() {
            ui.disable();
        }
        let icon_texture = self.app_icon_texture.clone();
        egui::menu::bar(ui, |ui| {
            // App icon on the far left of the menu bar
            ui.add(egui::Image::new(egui::load::SizedTexture::new(
                icon_texture.id(),
                [20.0, 20.0],
            )));
            ui.add_space(6.0);
            ui.label(
                RichText::new("Recorde")
                    .size(16.0)
                    .strong()
                    .color(theme::text_primary()),
            );
            ui.add_space(20.0);
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
                let current_repo = self.state.lock().unwrap().repo_path.clone();
                let selected = ui
                    .menu_button("Recent Repositories", |ui| {
                        crate::ui::toolbar::recent_repositories_menu(
                            ui,
                            &self.config.recent_repos,
                            current_repo.as_deref(),
                        )
                    })
                    .inner
                    .flatten();
                if let Some(action) = selected {
                    self.handle_toolbar(ctx, action);
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
                if ui
                    .button(if show_all {
                        "Show Current Branch Only"
                    } else {
                        "Show All Branches"
                    })
                    .clicked()
                {
                    self.handle_toolbar(ctx, ToolbarAction::ToggleAllBranches);
                    ui.close_menu();
                }
                if ui.button("Toggle Theme").clicked() {
                    self.handle_toolbar(ctx, ToolbarAction::ToggleTheme);
                    ui.close_menu();
                }
                ui.separator();
                if ui
                    .checkbox(&mut self.config.show_sidebar, "Show Sidebar")
                    .changed()
                {
                    self.save_config();
                }
                if ui
                    .checkbox(
                        &mut self.config.show_right_panel,
                        "Show Changes / Details Panel",
                    )
                    .changed()
                {
                    self.save_config();
                }
                if ui
                    .checkbox(&mut self.config.show_diff_panel, "Show Diff Panel")
                    .changed()
                {
                    self.save_config();
                }
                if ui.button("Reset Layout").clicked() {
                    self.reset_layout();
                    ui.close_menu();
                }
                ui.separator();
                for (label, pane) in [
                    ("Focus Sidebar", 1),
                    ("Focus Commit Graph", 2),
                    ("Focus Changes", 3),
                    ("Focus Diff", 4),
                ] {
                    if ui.button(label).clicked() {
                        self.focus_pane(pane);
                        ui.close_menu();
                    }
                }
            });

            ui.menu_button("Repository", |ui| {
                let has_repo = self.state.lock().unwrap().repo_path.is_some();
                if ui
                    .add_enabled(has_repo, Button::new("Open in Editor"))
                    .clicked()
                {
                    if let Err(e) = open_repo_in_editor(
                        self.state.lock().unwrap().repo_path.clone(),
                        &self.config.default_editor,
                    ) {
                        self.state.lock().unwrap().error = Some(e);
                    }
                    ui.close_menu();
                }
                if ui
                    .add_enabled(has_repo, Button::new("Open Terminal"))
                    .clicked()
                {
                    if let Some(rp) = self.state.lock().unwrap().repo_path.clone() {
                        lifecycle::open_terminal(&rp);
                    }
                    ui.close_menu();
                }
                if ui
                    .add_enabled(has_repo, Button::new("Reveal in File Manager"))
                    .clicked()
                {
                    if let Some(rp) = self.state.lock().unwrap().repo_path.clone() {
                        lifecycle::reveal_in_file_manager(&rp);
                    }
                    ui.close_menu();
                }
                if ui.add_enabled(has_repo, Button::new("Copy Path")).clicked() {
                    let path_str = self
                        .state
                        .lock()
                        .unwrap()
                        .repo_path
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default();
                    ctx.output_mut(|o| o.copied_text = path_str);
                    ui.close_menu();
                }
                if ui
                    .add_enabled(has_repo, Button::new("Pull Requests…"))
                    .clicked()
                {
                    self.repo_ui.pull_requests_open = true;
                    ui.close_menu();
                }
                ui.separator();
                if ui.add_enabled(has_repo, Button::new("Fetch")).clicked() {
                    remote::fetch_all(&mut self.runner, &self.state);
                    ui.close_menu();
                }
                if ui.add_enabled(has_repo, Button::new("Pull")).clicked() {
                    remote::pull_branch(&mut self.runner, &self.state);
                    ui.close_menu();
                }
                if ui.add_enabled(has_repo, Button::new("Push")).clicked() {
                    remote::push_branch(&mut self.runner, &self.state);
                    ui.close_menu();
                }
                if ui.add_enabled(has_repo, Button::new("Refresh")).clicked() {
                    if let Some(path) = self.state.lock().unwrap().repo_path.clone() {
                        load_repo(path, self.state.clone());
                    }
                    ui.close_menu();
                }
                ui.separator();
                let has_diff = self.state.lock().unwrap().current_diff.is_some();
                if ui
                    .add_enabled(has_diff, Button::new("Export Patch…"))
                    .clicked()
                {
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
                if ui
                    .add_enabled(has_repo, Button::new("Apply Patch File…"))
                    .clicked()
                {
                    let repo_path = self.state.lock().unwrap().repo_path.clone();
                    if let Some(ref rp) = repo_path {
                        if let Some(patch_path) = rfd::FileDialog::new()
                            .add_filter("Patch", &["patch", "diff"])
                            .pick_file()
                        {
                            crate::git::mutation::spawn_operation(
                                rp.clone(),
                                self.state.clone(),
                                move |path| {
                                    crate::git::patch::import_patch_file(path, &patch_path)
                                        .map_err(|e| e.user_message())
                                },
                            );
                        }
                    }
                    ui.close_menu();
                }
                ui.separator();
                if ui
                    .add_enabled(has_repo, Button::new("Repository Settings"))
                    .clicked()
                {
                    self.open_settings(SettingsTab::Repository);
                    ui.close_menu();
                }
            });

            ui.menu_button("Branch", |ui| {
                let has_repo = self.state.lock().unwrap().repo_path.is_some();
                if ui
                    .add_enabled(has_repo, Button::new("Switch Branch..."))
                    .clicked()
                {
                    self.branch_switcher_open = true;
                    ui.close_menu();
                }
                if ui
                    .add_enabled(has_repo, Button::new("New Branch..."))
                    .clicked()
                {
                    self.state.lock().unwrap().modal = ModalKind::CreateBranch { from_oid: None };
                    ui.close_menu();
                }
                ui.separator();
                if ui
                    .add_enabled(has_repo, Button::new("Merge Branch…"))
                    .clicked()
                {
                    self.state.lock().unwrap().modal = ModalKind::MergeBranch;
                    ui.close_menu();
                }
                if ui
                    .add_enabled(has_repo, Button::new("Rebase Onto…"))
                    .clicked()
                {
                    self.state.lock().unwrap().modal = ModalKind::RebaseOnto;
                    ui.close_menu();
                }
                ui.separator();
                if ui
                    .add_enabled(has_repo, Button::new("Stash Changes…"))
                    .clicked()
                {
                    self.repo_ui.stash_message_input = "WIP".into();
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
        egui::Stroke::new(1.5, theme::accent_teal().gamma_multiply(0.5)),
    );
}

fn panel_toggle(ui: &mut egui::Ui, visible: &mut bool, label: &str, tip: &str) -> bool {
    let color = if *visible {
        theme::accent_teal()
    } else {
        theme::text_muted()
    };
    let resp = ui
        .add(
            Button::new(RichText::new(label).color(color).size(12.0))
                .fill(if *visible {
                    theme::bg_panel_alt()
                } else {
                    Color32::TRANSPARENT
                })
                .stroke(Stroke::new(
                    1.0,
                    if *visible {
                        theme::accent_teal().gamma_multiply(0.45)
                    } else {
                        theme::border()
                    },
                ))
                .rounding(5.0)
                .min_size(egui::Vec2::new(0.0, 24.0)),
        )
        .on_hover_text(tip);
    if resp.clicked() {
        *visible = !*visible;
        true
    } else {
        false
    }
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

impl RecordeApp {
    fn render_provider(&mut self, ctx: &Context) {
        use crate::providers::CiStatus as CS;

        let (
            issue_list,
            issue_loading,
            issue_error,
            issue_sel,
            issue_filter,
            ci_runs,
            ci_loading,
            ci_error,
            releases,
            rel_loading,
            rel_error,
        ) = {
            let s = self.state.lock().unwrap();
            (
                s.issue_list.clone(),
                s.issue_loading,
                s.issue_error.clone(),
                s.issue_selected,
                s.issue_filter,
                s.ci_runs.clone(),
                s.ci_loading,
                s.ci_error.clone(),
                s.releases.clone(),
                s.releases_loading,
                s.releases_error.clone(),
            )
        };

        let mut new_issue_filter: Option<IssueFilter> = None;
        let mut new_issue_sel: Option<Option<usize>> = None;
        let mut open_url_req: Option<String> = None;
        let mut create_branch_for: Option<(u32, String)> = None;

        egui::Window::new("Provider Integrations")
            .collapsible(false)
            .resizable(true)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .default_size([820.0, 560.0])
            .frame(egui::Frame::window(&ctx.style()).inner_margin(egui::Margin::same(14.0)))
            .show(ctx, |ui| {
                // ── Tab bar ─────────────────────────────────────────────
                ui.horizontal(|ui| {
                    for (tab, label) in [
                        (ProviderTab::Issues, "Issues"),
                        (ProviderTab::Ci, "CI / Checks"),
                        (ProviderTab::Releases, "Releases"),
                    ] {
                        let active = self.repo_ui.provider_tab == tab;
                        let color = if active {
                            theme::text_primary()
                        } else {
                            theme::text_muted()
                        };
                        let btn = ui
                            .add(
                                Button::new(RichText::new(label).size(13.0).color(color))
                                    .fill(Color32::TRANSPARENT),
                            )
                            .on_hover_text(match tab {
                                ProviderTab::Issues => "View provider issues for this repository",
                                ProviderTab::Ci => "View CI/check runs for this repository",
                                ProviderTab::Releases => {
                                    "View provider releases for this repository"
                                }
                            });
                        if active {
                            ui.painter().line_segment(
                                [btn.rect.left_bottom(), btn.rect.right_bottom()],
                                egui::Stroke::new(2.0, theme::accent_teal()),
                            );
                        }
                        if btn.clicked() {
                            self.repo_ui.provider_tab = tab;
                        }
                        ui.add_space(8.0);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(Button::new("✕").fill(Color32::TRANSPARENT))
                            .on_hover_text("Close provider integrations")
                            .clicked()
                        {
                            self.repo_ui.provider_open = false;
                        }
                    });
                });
                ui.separator();

                if self.state.lock().unwrap().capabilities.as_ref().is_some_and(|c| c.provider == crate::git::capabilities::ProviderType::AzureDevOps) {
                    ui.label(styled_muted("Azure Boards work items and classic releases cover the whole project. CI is filtered to this repository and branch."));
                }
                match self.repo_ui.provider_tab {
                    ProviderTab::Issues => {
                        // Filter bar
                        ui.horizontal(|ui| {
                            for f in [IssueFilter::Open, IssueFilter::Closed, IssueFilter::All] {
                                let active = issue_filter == f;
                                let color = if active {
                                    theme::accent_teal()
                                } else {
                                    theme::text_muted()
                                };
                                let btn = ui.add(
                                    Button::new(RichText::new(f.label()).size(12.0).color(color))
                                        .fill(Color32::TRANSPARENT),
                                );
                                if active {
                                    ui.painter().line_segment(
                                        [btn.rect.left_bottom(), btn.rect.right_bottom()],
                                        egui::Stroke::new(2.0, theme::accent_teal()),
                                    );
                                }
                                if btn.clicked() && !active {
                                    new_issue_filter = Some(f);
                                }
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.add(secondary_btn("Refresh")).clicked() {
                                        load_issues(self.state.clone());
                                    }
                                },
                            );
                        });
                        ui.add_space(4.0);

                        if !self.repo_ui.issues_fetched {
                            self.repo_ui.issues_fetched = true;
                            load_issues(self.state.clone());
                        }
                        if issue_loading {
                            ui.centered_and_justified(|ui| {
                                ui.label(RichText::new("Loading…").color(theme::text_muted()));
                            });
                            return;
                        }
                        if let Some(ref err) = issue_error {
                            ui.label(RichText::new(err).color(theme::accent_red()).size(13.0));
                            return;
                        }

                        let visible: Vec<(usize, &crate::providers::IssueItem)> = issue_list
                            .iter()
                            .enumerate()
                            .filter(|(_, i)| issue_filter.matches(&i.state))
                            .collect();

                        ui.columns(2, |cols| {
                            egui::ScrollArea::vertical().id_source("iss_list").show(
                                &mut cols[0],
                                |ui| {
                                    if visible.is_empty() {
                                        ui.label(
                                            RichText::new("No issues.")
                                                .color(theme::text_muted())
                                                .size(13.0),
                                        );
                                    }
                                    for (i, issue) in &visible {
                                        let is_sel = issue_sel == Some(*i);
                                        let bg = if is_sel {
                                            theme::bg_selected()
                                        } else {
                                            Color32::TRANSPARENT
                                        };
                                        let (row, _) = ui.allocate_exact_size(
                                            egui::vec2(ui.available_width(), 44.0),
                                            egui::Sense::click(),
                                        );
                                        ui.painter().rect_filled(row, 0.0, bg);
                                        ui.allocate_ui_at_rect(row, |ui| {
                                            ui.horizontal(|ui| {
                                                ui.add_space(8.0);
                                                let dot_color = if issue.state
                                                    == crate::providers::IssueState::Open
                                                {
                                                    theme::accent_green()
                                                } else {
                                                    theme::accent_red()
                                                };
                                                let (dr, _) = ui.allocate_exact_size(
                                                    egui::vec2(8.0, 8.0),
                                                    egui::Sense::hover(),
                                                );
                                                ui.painter().circle_filled(
                                                    dr.center(),
                                                    4.0,
                                                    dot_color,
                                                );
                                                ui.add_space(4.0);
                                                ui.vertical(|ui| {
                                                    ui.label(
                                                        RichText::new(format!(
                                                            "#{} {}",
                                                            issue.number, &issue.title
                                                        ))
                                                        .color(theme::text_primary())
                                                        .size(13.0),
                                                    );
                                                    ui.label(
                                                        RichText::new(format!(
                                                            "@{}  •  {}",
                                                            issue.author,
                                                            crate::theme::relative_time(
                                                                issue.updated_ts
                                                            )
                                                        ))
                                                        .color(theme::text_muted())
                                                        .size(12.0),
                                                    );
                                                });
                                            });
                                        });
                                        if ui
                                            .interact(
                                                row,
                                                egui::Id::new(("iss_row", i)),
                                                egui::Sense::click(),
                                            )
                                            .clicked()
                                        {
                                            new_issue_sel = Some(Some(*i));
                                        }
                                        ui.painter().line_segment(
                                            [row.left_bottom(), row.right_bottom()],
                                            egui::Stroke::new(1.0, theme::border()),
                                        );
                                    }
                                },
                            );

                            if let Some(idx) = issue_sel {
                                if let Some(issue) = issue_list.get(idx) {
                                    cols[1].add_space(4.0);
                                    cols[1].label(
                                        RichText::new(format!(
                                            "#{} {}",
                                            issue.number, &issue.title
                                        ))
                                        .color(theme::text_primary())
                                        .size(14.0)
                                        .strong(),
                                    );
                                    cols[1].add_space(4.0);
                                    cols[1].horizontal(|ui| {
                                        ui.label(
                                            RichText::new(format!(
                                                "@{}  updated {}",
                                                issue.author,
                                                crate::theme::relative_time(issue.updated_ts)
                                            ))
                                            .color(theme::text_muted())
                                            .size(12.0),
                                        );
                                    });
                                    if !issue.label_names.is_empty() {
                                        cols[1].horizontal(|ui| {
                                            for l in &issue.label_names {
                                                ui.label(
                                                    RichText::new(l)
                                                        .size(12.0)
                                                        .color(theme::accent_blue())
                                                        .background_color(
                                                            theme::accent_blue()
                                                                .gamma_multiply(0.15),
                                                        ),
                                                );
                                                ui.add_space(4.0);
                                            }
                                        });
                                    }
                                    cols[1].add_space(4.0);
                                    cols[1].separator();
                                    egui::ScrollArea::vertical()
                                        .id_source("iss_body")
                                        .max_height(200.0)
                                        .show(&mut cols[1], |ui| {
                                            let body = if issue.body.trim().is_empty() {
                                                "No description."
                                            } else {
                                                issue.body.trim()
                                            };
                                            ui.label(
                                                RichText::new(body)
                                                    .color(theme::text_secondary())
                                                    .size(13.0),
                                            );
                                        });
                                    cols[1].add_space(8.0);
                                    cols[1].separator();
                                    cols[1].add_space(4.0);
                                    cols[1].horizontal(|ui| {
                                        if ui.add(primary_btn("Open in Browser")).clicked() {
                                            open_url_req = Some(issue.url.clone());
                                        }
                                        if ui.add(secondary_btn("Create Branch…")).clicked() {
                                            create_branch_for =
                                                Some((issue.number, issue.title.clone()));
                                        }
                                    });
                                }
                            } else {
                                cols[1].centered_and_justified(|ui| {
                                    ui.label(
                                        RichText::new("Select an issue to view details.")
                                            .color(theme::text_muted())
                                            .size(13.0),
                                    );
                                });
                            }
                        });
                    }

                    ProviderTab::Ci => {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("CI runs for current branch")
                                    .color(theme::text_muted())
                                    .size(12.0),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.add(secondary_btn("Refresh")).clicked() {
                                        load_ci_runs(self.state.clone());
                                    }
                                },
                            );
                        });
                        ui.add_space(4.0);

                        if !self.repo_ui.ci_fetched {
                            self.repo_ui.ci_fetched = true;
                            load_ci_runs(self.state.clone());
                        }
                        if ci_loading {
                            ui.centered_and_justified(|ui| {
                                ui.label(RichText::new("Loading…").color(theme::text_muted()));
                            });
                            return;
                        }
                        if let Some(ref err) = ci_error {
                            ui.label(RichText::new(err).color(theme::accent_red()).size(13.0));
                            return;
                        }

                        egui::ScrollArea::vertical().show(ui, |ui| {
                            if ci_runs.is_empty() {
                                ui.label(
                                    RichText::new("No CI runs found for this branch.")
                                        .color(theme::text_muted())
                                        .size(13.0),
                                );
                            }
                            for run in &ci_runs {
                                let (status_color, status_icon) = match run.status {
                                    CS::Success => (theme::accent_green(), "✓"),
                                    CS::Failure => (theme::accent_red(), "✗"),
                                    CS::Running => (theme::accent_yellow(), "…"),
                                    CS::Pending => (theme::text_muted(), "·"),
                                    CS::Cancelled => (theme::text_muted(), "—"),
                                };
                                let row = ui
                                    .horizontal(|ui| {
                                        ui.add_space(8.0);
                                        ui.label(
                                            RichText::new(status_icon)
                                                .size(14.0)
                                                .color(status_color),
                                        );
                                        ui.add_space(6.0);
                                        ui.vertical(|ui| {
                                            ui.label(
                                                RichText::new(&run.name)
                                                    .color(theme::text_primary())
                                                    .size(13.0),
                                            );
                                            ui.horizontal(|ui| {
                                                ui.label(
                                                    RichText::new(run.status.label())
                                                        .color(status_color)
                                                        .size(12.0),
                                                );
                                                ui.label(
                                                    RichText::new(format!(
                                                        "  •  {}",
                                                        crate::theme::relative_time(run.updated_ts)
                                                    ))
                                                    .color(theme::text_muted())
                                                    .size(12.0),
                                                );
                                            });
                                        });
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if ui
                                                    .add(
                                                        Button::new(
                                                            RichText::new("Open")
                                                                .size(12.0)
                                                                .color(theme::accent_teal()),
                                                        )
                                                        .fill(Color32::TRANSPARENT),
                                                    )
                                                    .clicked()
                                                {
                                                    open_url_req = Some(run.url.clone());
                                                }
                                            },
                                        );
                                    })
                                    .response;
                                ui.painter().line_segment(
                                    [row.rect.left_bottom(), row.rect.right_bottom()],
                                    egui::Stroke::new(1.0, theme::border()),
                                );
                                ui.add_space(2.0);
                            }
                        });
                    }

                    ProviderTab::Releases => {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("Releases")
                                    .color(theme::text_muted())
                                    .size(12.0),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.add(secondary_btn("Refresh")).clicked() {
                                        load_releases(self.state.clone());
                                    }
                                },
                            );
                        });
                        ui.add_space(4.0);

                        if !self.repo_ui.releases_fetched {
                            self.repo_ui.releases_fetched = true;
                            load_releases(self.state.clone());
                        }
                        if rel_loading {
                            ui.centered_and_justified(|ui| {
                                ui.label(RichText::new("Loading…").color(theme::text_muted()));
                            });
                            return;
                        }
                        if let Some(ref err) = rel_error {
                            ui.label(RichText::new(err).color(theme::accent_red()).size(13.0));
                            return;
                        }

                        egui::ScrollArea::vertical().show(ui, |ui| {
                            if releases.is_empty() {
                                ui.label(
                                    RichText::new("No releases found.")
                                        .color(theme::text_muted())
                                        .size(13.0),
                                );
                            }
                            for rel in &releases {
                                let row = ui
                                    .horizontal(|ui| {
                                        ui.add_space(8.0);
                                        let tag_color = if rel.prerelease {
                                            theme::accent_yellow()
                                        } else {
                                            theme::accent_teal()
                                        };
                                        ui.label(
                                            RichText::new(&rel.tag_name)
                                                .color(tag_color)
                                                .size(13.0)
                                                .strong(),
                                        );
                                        ui.add_space(6.0);
                                        ui.label(
                                            RichText::new(&rel.name)
                                                .color(theme::text_secondary())
                                                .size(13.0),
                                        );
                                        if rel.prerelease {
                                            ui.label(
                                                RichText::new("pre-release")
                                                    .size(12.0)
                                                    .color(theme::accent_yellow())
                                                    .background_color(
                                                        theme::accent_yellow().gamma_multiply(0.15),
                                                    ),
                                            );
                                        }
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                ui.label(
                                                    RichText::new(crate::theme::relative_time(
                                                        rel.published_ts,
                                                    ))
                                                    .color(theme::text_muted())
                                                    .size(12.0),
                                                );
                                                ui.add_space(8.0);
                                                if ui
                                                    .add(
                                                        Button::new(
                                                            RichText::new("Open")
                                                                .size(12.0)
                                                                .color(theme::accent_teal()),
                                                        )
                                                        .fill(Color32::TRANSPARENT),
                                                    )
                                                    .clicked()
                                                {
                                                    open_url_req = Some(rel.url.clone());
                                                }
                                            },
                                        );
                                    })
                                    .response;
                                ui.painter().line_segment(
                                    [row.rect.left_bottom(), row.rect.right_bottom()],
                                    egui::Stroke::new(1.0, theme::border()),
                                );
                                ui.add_space(2.0);
                            }
                        });
                    }
                }
            });

        // ── Branch-from-issue form (inline modal) ─────────────────────────
        if let Some((number, title)) = create_branch_for {
            if self.repo_ui.branch_from_issue_number != Some(number) {
                self.repo_ui.branch_from_issue_number = Some(number);
                self.repo_ui.branch_from_issue_input = format!(
                    "{}-{}",
                    number,
                    slugify(&title).chars().take(40).collect::<String>()
                );
            }
        }
        if self.repo_ui.branch_from_issue_number.is_some() {
            let mut close = false;
            let mut do_create = false;
            modal_window(ctx, "Create Branch from Issue", |ui| {
                ui.label(styled_label("Branch name"));
                ui.add_space(4.0);
                let r = ui.add(
                    egui::TextEdit::singleline(&mut self.repo_ui.branch_from_issue_input)
                        .desired_width(f32::INFINITY)
                        .margin(egui::Vec2::new(8.0, 5.0)),
                );
                r.request_focus();
                ui.add_space(4.0);
                let can = !self.repo_ui.branch_from_issue_input.trim().is_empty();
                ui.horizontal(|ui| {
                    if ui.add_enabled(can, primary_btn("Create Branch")).clicked()
                        || (can && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                    {
                        do_create = true;
                    }
                    if ui.add(secondary_btn("Cancel")).clicked() {
                        close = true;
                    }
                });
            });
            if do_create {
                branches::create_branch(
                    &self.repo_ui.branch_from_issue_input.trim(),
                    None,
                    true,
                    self.state.clone(),
                );
                self.repo_ui.branch_from_issue_number = None;
                self.repo_ui.branch_from_issue_input.clear();
            } else if close {
                self.repo_ui.branch_from_issue_number = None;
                self.repo_ui.branch_from_issue_input.clear();
            }
        }

        // Apply deferred mutations
        if let Some(f) = new_issue_filter {
            self.state.lock().unwrap().issue_filter = f;
            load_issues(self.state.clone());
        }
        if let Some(sel) = new_issue_sel {
            self.state.lock().unwrap().issue_selected = sel;
        }
        if let Some(ref url) = open_url_req {
            if let Err(error) = open_url(url) {
                self.state.lock().unwrap().error = Some(error);
            }
        }
    }
}

/// Returns a combined modification-time fingerprint for the ref files that
/// change when tags, branches, or commits are added/removed externally.
/// Returns 0 if the repo path can't be opened.
fn slugify(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn primary_btn(label: &str) -> Button<'_> {
    Button::new(RichText::new(label).color(Color32::WHITE).size(14.0))
        .fill(theme::primary_fill())
        .rounding(7.0)
        .min_size(egui::Vec2::new(120.0, 28.0))
}

fn danger_btn(label: &str) -> Button<'_> {
    Button::new(RichText::new(label).color(Color32::WHITE))
        .fill(theme::accent_red())
        .rounding(6.0)
        .min_size(egui::Vec2::new(0.0, 28.0))
}

fn secondary_btn(label: &str) -> Button<'_> {
    Button::new(RichText::new(label).color(theme::text_secondary()))
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::new(1.0, theme::border()))
        .rounding(6.0)
        .min_size(egui::Vec2::new(0.0, 28.0))
}

fn health_row(ui: &mut egui::Ui, label: &str, value: &str) {
    health_row_color(ui, label, value, theme::text_secondary());
}

fn health_row_color(ui: &mut egui::Ui, label: &str, value: &str, color: egui::Color32) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("{:<14}", label))
                .color(theme::text_muted())
                .size(12.0)
                .monospace(),
        );
        ui.label(RichText::new(value).color(color).size(12.0));
    });
}

fn styled_label(text: &str) -> RichText {
    RichText::new(text)
        .color(theme::text_secondary())
        .size(13.0)
}

fn styled_muted(text: &str) -> RichText {
    RichText::new(text).color(theme::text_muted()).size(12.0)
}

fn settings_tab_button(ui: &mut egui::Ui, tab: &mut SettingsTab, value: SettingsTab, label: &str) {
    let selected = *tab == value;
    let color = if selected {
        theme::text_primary()
    } else {
        theme::text_muted()
    };
    if ui
        .add(
            Button::new(RichText::new(label).color(color).size(13.0))
                .fill(if selected {
                    theme::bg_selected()
                } else {
                    Color32::TRANSPARENT
                })
                .stroke(Stroke::new(
                    1.0,
                    if selected {
                        theme::border_strong()
                    } else {
                        theme::border()
                    },
                ))
                .rounding(6.0)
                .min_size(egui::Vec2::new(0.0, 28.0)),
        )
        .clicked()
    {
        *tab = value;
    }
}

fn settings_text_field(ui: &mut egui::Ui, label: &str, value: &mut String) {
    ui.horizontal(|ui| {
        ui.set_width(460.0);
        ui.label(
            RichText::new(label)
                .color(theme::text_muted())
                .font(egui::FontId::monospace(12.0)),
        );
        ui.add_space(8.0);
        ui.add_sized(
            [340.0, 28.0],
            egui::TextEdit::singleline(value).margin(egui::Vec2::new(8.0, 5.0)),
        );
    });
}

fn git_identity(repo_path: Option<&std::path::Path>) -> (String, String) {
    let config = match repo_path {
        Some(path) => git2::Repository::open(path)
            .ok()
            .and_then(|r| r.config().ok()),
        None => git2::Config::open_default().ok(),
    };
    let Some(config) = config else {
        return (String::new(), String::new());
    };
    (
        config.get_string("user.name").unwrap_or_default(),
        config.get_string("user.email").unwrap_or_default(),
    )
}

fn save_git_identity(
    repo_path: Option<&std::path::Path>,
    name: &str,
    email: &str,
) -> Result<(), String> {
    crate::git::commit_settings::validate_identity(name, email)?;
    let mut config = match repo_path {
        Some(path) => git2::Repository::open(path)
            .map_err(|e| e.message().to_string())?
            .config()
            .map_err(|e| e.message().to_string())?,
        None => git2::Config::open_default()
            .and_then(|mut c| c.open_global())
            .map_err(|e| e.message().to_string())?,
    };
    if !name.trim().is_empty() {
        config
            .set_str("user.name", name.trim())
            .map_err(|e| e.message().to_string())?;
    }
    if !email.trim().is_empty() {
        config
            .set_str("user.email", email.trim())
            .map_err(|e| e.message().to_string())?;
    }
    Ok(())
}

fn git_config_value(repo_path: Option<&std::path::Path>, key: &str) -> Option<String> {
    match repo_path {
        Some(path) => git2::Repository::open(path)
            .ok()?
            .config()
            .ok()?
            .get_string(key)
            .ok(),
        None => git2::Config::open_default().ok()?.get_string(key).ok(),
    }
}

fn repo_remotes(path: &std::path::Path) -> Vec<String> {
    let Ok(repo) = git2::Repository::open(path) else {
        return vec!["Could not open repository.".into()];
    };
    let Ok(names) = repo.remotes() else {
        return vec!["No remotes configured.".into()];
    };
    let mut remotes = Vec::new();
    for name in names.iter().flatten().flatten() {
        let url = repo
            .find_remote(name)
            .ok()
            .and_then(|r| r.url().ok().map(str::to_owned))
            .unwrap_or_default();
        remotes.push(format!("{name}: {url}"));
    }
    if remotes.is_empty() {
        remotes.push("No remotes configured.".into());
    }
    remotes
}

fn open_repo_in_editor(
    repo_path: Option<std::path::PathBuf>,
    configured_editor: &str,
) -> Result<(), String> {
    let path =
        repo_path.ok_or_else(|| "Open a repository before launching an editor.".to_string())?;
    let editor = if !configured_editor.trim().is_empty() {
        configured_editor.trim().to_string()
    } else {
        std::env::var("VISUAL")
            .or_else(|_| std::env::var("EDITOR"))
            .map_err(|_| "Set a default editor in App Settings first.".to_string())?
    };

    crate::external::open_editor(&path, &editor)
}

fn open_url(url: &str) -> Result<(), String> {
    crate::external::open_url(url)
}
