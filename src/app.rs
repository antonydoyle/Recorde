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

mod accounts;
mod history_plan;
mod lifecycle_guards;
mod menus;
mod settings;
mod switch;
mod workflows;

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
    /// Shown as "General".
    App,
    Ui,
    /// Pull strategy, fetching, commit defaults and protected branches.
    Git,
    /// Editor, terminal, diff and merge tools.
    Tools,
    Accounts,
    Shortcuts,
    Repository,
    About,
    Updates,
}

enum AccountChange {
    Migrated,
    Saved(StoredAccount),
    Removed(StoredAccount),
}

#[derive(Clone)]
struct RepoSettingsSnapshot {
    retention_days: i64,
    remotes: Vec<(String, String)>,
    provider_remote: String,
    push_default: String,
    branch: Option<String>,
    branch_push_remote: String,
    recovery: Vec<crate::git::recovery::Checkpoint>,
}

impl RepoSettingsSnapshot {
    fn load(path: &std::path::Path) -> Result<Self, String> {
        let repo = git2::Repository::open(path).map_err(|error| error.to_string())?;
        Self::from_repo(&repo)
    }

    fn from_repo(repo: &git2::Repository) -> Result<Self, String> {
        let config = repo.config().map_err(|error| error.to_string())?;
        let names = repo.remotes().map_err(|error| error.to_string())?;
        let mut remotes = Vec::new();
        for name in names.iter().flatten().flatten() {
            let url = repo
                .find_remote(name)
                .ok()
                .and_then(|remote| remote.url().ok().map(str::to_owned))
                .unwrap_or_default();
            remotes.push((name.to_owned(), url));
        }
        let branch = repo
            .head()
            .ok()
            .filter(|head| head.is_branch())
            .and_then(|head| head.shorthand().ok().map(str::to_owned));
        let branch_push_remote = branch
            .as_ref()
            .and_then(|branch| {
                config
                    .get_string(&format!("branch.{branch}.pushRemote"))
                    .ok()
            })
            .unwrap_or_default();
        Ok(Self {
            retention_days: config.get_i64("recorde.recoveryRetentionDays").unwrap_or(0),
            remotes,
            provider_remote: remote::provider_remote(repo).unwrap_or_default(),
            push_default: config.get_string("remote.pushDefault").unwrap_or_default(),
            branch,
            branch_push_remote,
            recovery: crate::git::recovery::list(repo.workdir().unwrap_or(repo.path())),
        })
    }
}

type RepoSettingsResult = Result<RepoSettingsSnapshot, String>;

#[derive(Default)]
struct RepoSettingsCache {
    scope: Option<(std::path::PathBuf, u64, String)>,
    visible: bool,
    generation: std::sync::Arc<std::sync::atomic::AtomicU64>,
    loaded_generation: u64,
    request_id: u64,
    pending: Option<(u64, std::sync::mpsc::Receiver<RepoSettingsResult>)>,
    snapshot: Option<RepoSettingsResult>,
}

impl RepoSettingsCache {
    fn poll(&mut self, path: &std::path::Path, epoch: u64, branch: &str, ctx: &Context) {
        use std::sync::atomic::Ordering;
        let scope = (path.to_path_buf(), epoch, branch.to_owned());
        let generation = self.generation.load(Ordering::Acquire);
        if !self.visible
            || self.scope.as_ref() != Some(&scope)
            || self.loaded_generation != generation
        {
            self.visible = true;
            self.scope = Some(scope);
            self.loaded_generation = generation;
            self.request_id += 1;
            self.snapshot = None;
        }
        if let Some((id, rx)) = &self.pending {
            let result = match rx.try_recv() {
                Ok(result) => Some(result),
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    Some(Err("Could not load repository settings.".into()))
                }
            };
            if let Some(result) = result {
                if *id == self.request_id {
                    self.snapshot = Some(result);
                }
                self.pending = None;
            }
        }
        // Keep at most one reader running, even when tabs or repositories change quickly.
        if self.pending.is_none() && self.snapshot.is_none() {
            let (tx, rx) = std::sync::mpsc::channel();
            self.pending = Some((self.request_id, rx));
            let path = path.to_path_buf();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(RepoSettingsSnapshot::load(&path));
                ctx.request_repaint();
            });
        }
        if self.pending.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
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
    stash_include_untracked: bool,
    stash_branch_input: String,
    reset_mode: crate::git::branches::ResetMode,
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
    file_editor: Option<crate::ui::file_editor::FileEditor>,
    merge_mode: crate::git::merge::MergeMode,
    merge_message: String,
    merge_choice: Option<String>,
    flow_config: crate::git::gitflow::FlowConfig,
    flow_kind: crate::git::gitflow::Kind,
    flow_name: String,
    flow_finish: Option<workflows::FlowFinish>,
    reset_preview: Option<crate::git::branches::ResetPreview>,
    rebase_target: Option<crate::git::rebase::PlanTarget>,
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
            stash_include_untracked: false,
            stash_branch_input: String::new(),
            reset_mode: Default::default(),
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
            file_editor: None,
            merge_mode: Default::default(),
            merge_message: String::new(),
            merge_choice: None,
            flow_config: Default::default(),
            flow_kind: crate::git::gitflow::Kind::Feature,
            flow_name: String::new(),
            flow_finish: None,
            reset_preview: None,
            rebase_target: None,
        }
    }
}

fn unsaved_file_edit<'a>(
    active: &'a RepoUiSession,
    inactive: impl Iterator<Item = &'a RepoUiSession>,
) -> Option<&'a str> {
    std::iter::once(active)
        .chain(inactive)
        .filter_map(|session| session.file_editor.as_ref())
        .find(|editor| editor.is_dirty())
        .map(|editor| editor.file.as_str())
}

pub struct RecordeApp {
    quit_waiting: bool,
    pending_navigation: Option<lifecycle_guards::Navigation>,
    history_plan: Option<history_plan::Task>,
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
    help: crate::ui::help::HelpBrowser,
    support: crate::diagnostics::SupportWindow,
    repository_browser: crate::ui::repository_browser::RepositoryBrowser,
    /// Pre-loaded texture of the app icon for rendering in the title bar.
    app_icon_texture: egui::TextureHandle,
    /// Tracks the last-seen mtimes of key .git ref files so external changes
    /// (tags created in a terminal, commits from another tool, etc.) are picked up.
    repo_watch: Option<crate::git::watch::RepoWatch>,
    was_focused: bool,
    last_auto_fetch: std::time::Instant,
    auto_fetch_repo: Option<std::path::PathBuf>,
    auto_fetch_failures: u32,
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
    repo_settings: RepoSettingsCache,
    updater: crate::update::Controller,
    instance_guard: Option<crate::update::InstanceGuard>,
    git_name_input: String,
    git_email_input: String,
    git_readiness: crate::git::readiness::Monitor,
    git_executable_input: String,
    ssh_diagnostics: Option<crate::auth::ssh::SshDiagnostics>,
    diagnostics_pending: Option<std::sync::mpsc::Receiver<crate::auth::ssh::SshDiagnostics>>,
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
    terminal: crate::ui::terminal_panel::TerminalPanel,
    init_options: crate::git::templates::InitOptions,
    profile_input: crate::profiles::Profile,
    profile_message: Option<String>,
    ssh_key_path: String,
    ssh_key_comment: String,
    ssh_key_passphrase: String,
    ssh_public_key: Option<(std::path::PathBuf, String)>,
    ssh_task: Option<std::sync::mpsc::Receiver<Result<workflows::SshDone, String>>>,
    ssh_message: Option<String>,
}

impl RecordeApp {
    pub fn with_instance_guard(mut self, guard: Option<crate::update::InstanceGuard>) -> Self {
        self.instance_guard = guard;
        self
    }

    pub fn new(cc: &eframe::CreationContext<'_>, initial_repo: Option<std::path::PathBuf>) -> Self {
        let config = Config::load();
        crate::git::readiness::configure(&config.git_executable);
        let git_executable_input = config.git_executable.clone();
        let dark_theme = config.dark_theme;
        let high_contrast = config.high_contrast;
        let colorblind_graph = config.colorblind_graph;
        let default_editor = config.default_editor.clone();
        let terminal = config.terminal.clone();
        let diff_tool = config.diff_tool.clone();
        let merge_tool = config.merge_tool.clone();

        cc.egui_ctx.set_fonts(crate::icons::font_definitions());
        crate::ui::avatars::set_enabled(config.gravatar);
        crate::ui::motion::apply(&cc.egui_ctx, config.motion);

        theme::apply_palette(
            &cc.egui_ctx,
            dark_theme,
            high_contrast,
            config.theme_palette,
        );
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
            quit_waiting: false,
            pending_navigation: None,
            history_plan: None,
            workspace_name: String::new(),
            repo_ui: RepoUiSession::default(),
            repo_sessions: Default::default(),
            runner,
            toolbar: Toolbar::new(state.clone()),
            sidebar: {
                let mut sidebar = Sidebar::new(state.clone());
                sidebar.set_colorblind(colorblind_graph);
                sidebar
            },
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
            auto_fetch_failures: 0,
            observed_repo: None,
            observed_provider: None,
            config,
            help: Default::default(),
            support: Default::default(),
            repository_browser: Default::default(),
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
            repo_settings: RepoSettingsCache::default(),
            updater: Default::default(),
            instance_guard: None,
            git_name_input: String::new(),
            git_email_input: String::new(),
            git_readiness: Default::default(),
            git_executable_input,
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
            terminal: Default::default(),
            init_options: Default::default(),
            profile_input: Default::default(),
            profile_message: None,
            ssh_key_path: String::new(),
            ssh_key_comment: String::new(),
            ssh_key_passphrase: String::new(),
            ssh_public_key: None,
            ssh_task: None,
            ssh_message: None,
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
                // Release the state lock before reporting a launch error.
                let repo_path = self.state.lock().unwrap().repo_path.clone();
                if let Some(rp) = repo_path {
                    if let Err(error) = lifecycle::try_open_terminal(&rp) {
                        self.state.lock().unwrap().error = Some(error);
                    }
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
                    .map(|p| crate::ui::kit::display_path(p))
                    .unwrap_or_default();
                ctx.copy_text(path_str);
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
                self.state.lock().unwrap().modal = ModalKind::MergeBranch { branch: None };
            }
            ToolbarAction::Undo => self.open_undo(false),
            ToolbarAction::Redo => self.open_undo(true),
            ToolbarAction::RebaseOnto => {
                self.state.lock().unwrap().modal = ModalKind::RebaseOnto;
            }
            ToolbarAction::ToggleTheme => {
                self.dark_theme = !self.dark_theme;
                self.config.dark_theme = self.dark_theme;
                self.save_config();
                theme::apply_palette(
                    ctx,
                    self.dark_theme,
                    self.high_contrast,
                    self.config.theme_palette,
                );
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
            SidebarAction::StashApply(i, oid) => {
                if let Some(target) = self.stash_target(i, &oid) {
                    branches::stash_apply(target, self.state.clone());
                }
            }
            SidebarAction::StashPop(i, oid) => {
                if let Some(target) = self.stash_target(i, &oid) {
                    branches::stash_pop(target, self.state.clone());
                }
            }
            SidebarAction::StashDrop(i, oid) => {
                if let Some(target) = self.stash_target(i, &oid) {
                    self.state.lock().unwrap().modal = ModalKind::ConfirmDropStash { target };
                }
            }
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
                self.state.lock().unwrap().modal =
                    ModalKind::ConfirmForceDeleteBranch { branch_name: n }
            }
            SidebarAction::PushTag(n) => branches::push_tag(&n, self.state.clone()),
            SidebarAction::DeleteRemoteTag(n) => {
                self.state.lock().unwrap().modal = ModalKind::ConfirmDeleteRemoteTag { tag_name: n }
            }
            SidebarAction::CheckoutRemoteBranch(n) => {
                branches::checkout_remote_branch(&n, self.state.clone())
            }
            SidebarAction::DeleteRemoteBranch(n) => {
                self.state.lock().unwrap().modal = ModalKind::ConfirmDeleteRemoteBranch { name: n }
            }
            SidebarAction::RevealCommit(oid) => self.commit_graph_view.reveal_commit(&oid),
            SidebarAction::MergeWithOptions(n) => {
                self.state.lock().unwrap().modal = ModalKind::MergeBranch { branch: Some(n) }
            }
            SidebarAction::FastForward(n) => merge::fast_forward_branch(&n, self.state.clone()),
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
            SidebarAction::StashBranch(idx, oid) => {
                self.repo_ui.stash_branch_input.clear();
                if let Some(target) = self.stash_target(idx, &oid) {
                    self.state.lock().unwrap().modal = ModalKind::CreateBranchFromStash { target };
                }
            }
            SidebarAction::RestoreBranch(name, oid) => {
                branches::restore_deleted_branch(&name, &oid, self.state.clone())
            }
        }
    }

    fn stash_target(&self, index: usize, oid: &str) -> Option<branches::StashTarget> {
        let (path, epoch) = {
            let state = self.state.lock().unwrap();
            (state.repo_path.clone()?, state.repo_epoch)
        };
        match branches::StashTarget::capture(&path, epoch, index, oid) {
            Ok(target) => Some(target),
            Err(error) => {
                self.state.lock().unwrap().error = Some(error);
                None
            }
        }
    }

    fn open_settings(&mut self, tab: SettingsTab) {
        self.settings_tab = tab;
        self.settings_open = true;
        self.repo_settings.visible = false;
        self.load_git_identity_inputs();
        self.load_app_settings_inputs();
        if self.diagnostics_pending.is_none() {
            let (tx, rx) = std::sync::mpsc::channel();
            self.diagnostics_pending = Some(rx);
            std::thread::spawn(move || {
                let _ = tx.send(crate::auth::ssh::diagnose());
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
        self.start_history_plan(history_plan::Request::Rebase(upstream_oid));
    }
    // ── Modal rendering ───────────────────────────────────────────────────────

    fn render_modals(&mut self, ctx: &Context) {
        if self.commit_graph_view.has_pending_action() {
            return;
        }
        let modal_kind = self.state.lock().unwrap().modal.clone();
        if !matches!(modal_kind, ModalKind::ResetToCommit { .. }) {
            self.repo_ui.reset_preview = None;
        }
        match modal_kind {
            ModalKind::None => {}

            ModalKind::CreateBranch { from_oid } => modal_window(ctx, "Create Branch", |ui| {
                ui.label(styled_label("Branch name"));
                ui.add_space(4.0);
                let r = ui.add(
                    crate::ui::kit::text_field(&mut self.repo_ui.create_branch_name)
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
                    ui.label(styled_muted(
                        "Git refuses to delete a branch that is not fully merged. \
                         Use Force Delete for those.",
                    ));
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
                    crate::ui::kit::text_field(&mut self.repo_ui.create_tag_name)
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

            ModalKind::ConfirmForceDeleteBranch { branch_name } => self.render_confirm_modal(
                ctx,
                "Force Delete Branch",
                &format!("Force delete \"{branch_name}\"?"),
                "Commits that are not on another branch will no longer be reachable. \
                     The branch can be restored from Recently deleted.",
                "Force Delete",
                move |app| branches::force_delete_branch(&branch_name, app.state.clone()),
            ),

            ModalKind::ConfirmDeleteRemoteBranch { name } => self.render_confirm_modal(
                ctx,
                "Delete Remote Branch",
                &format!("Delete \"{name}\" from the remote?"),
                "This changes the shared repository for everyone. Its last known tip is kept \
                 under Recently deleted so it can be restored locally.",
                "Delete from Remote",
                move |app| branches::delete_remote_branch(&name, app.state.clone()),
            ),

            ModalKind::ConfirmDeleteRemoteTag { tag_name } => self.render_confirm_modal(
                ctx,
                "Delete Remote Tag",
                &format!("Delete tag \"{tag_name}\" from the remote?"),
                "This changes the shared repository for everyone. The local tag is kept.",
                "Delete from Remote",
                move |app| branches::delete_remote_tag(&tag_name, app.state.clone()),
            ),

            ModalKind::ResetToCommit { oid } => self.render_reset_modal(ctx, oid),

            ModalKind::ConfirmCommitSequence { preview } => {
                let label = preview.label();
                let branch = preview
                    .head_ref
                    .strip_prefix("refs/heads/")
                    .unwrap_or(&preview.head_ref);
                let question = format!("{label} {} commits on {branch}?", preview.oids.len());
                let commits = preview
                    .oids
                    .iter()
                    .take(8)
                    .map(|oid| &oid[..7.min(oid.len())])
                    .collect::<Vec<_>>()
                    .join(", ");
                let detail = format!("Repository: {}\nDestination HEAD: {}\nSelected commits (oldest first): {commits}{}. {} A recovery snapshot is saved first. The operation stops if the destination branch or HEAD changes before it starts.", crate::ui::kit::display_path(&preview.path), preview.head_oid, if preview.oids.len() > 8 { ", …" } else { "" }, if preview.revert { "Reverts run newest first." } else { "Cherry-picks run oldest first." });
                self.render_confirm_modal(ctx, label, &question, &detail, label, move |app| {
                    preview.submit(app.state.clone())
                });
            }

            ModalKind::ConfirmUndo { id, label, redo } => {
                let question = format!("{label}?");
                let button = label.clone();
                self.render_confirm_modal(
                    ctx,
                    if redo { "Redo" } else { "Undo" },
                    &question,
                    "Restores HEAD, the index and working files saved around that operation. \
                     Pushes and other remote changes are not undone. Recorde stops if files \
                     changed since then, and saves a new snapshot so this can be reversed.",
                    &button,
                    move |app| {
                        let path = app.state.lock().unwrap().repo_path.clone();
                        if let Some(path) = path {
                            app.state.lock().unwrap().remote_op = Some(label);
                            crate::git::mutation::spawn_operation(
                                path,
                                app.state.clone(),
                                move |repo| crate::git::recovery::restore_step(repo, &id, redo),
                            );
                        }
                    },
                )
            }

            ModalKind::CloneRepo => self.render_clone_modal(ctx),
            ModalKind::InitRepo => self.render_init_modal(ctx),
            ModalKind::RenameBranch { old_name } => self.render_rename_branch_modal(ctx, old_name),
            ModalKind::ConfirmDetachedCheckout { oid } => {
                self.render_detached_checkout_modal(ctx, oid)
            }
            ModalKind::StashWithMessage => self.render_stash_message_modal(ctx),
            ModalKind::CreateBranchFromStash { target } => {
                self.render_stash_branch_modal(ctx, target)
            }
            ModalKind::ConfirmDropStash { target } => {
                let question = format!("Drop stash@{{{}}} ({})?", target.index, &target.oid[..12]);
                let detail = format!("Repository: {}\nA recovery copy is kept at {}. Restore it with git stash apply; this action is not in Undo history.", crate::ui::kit::display_path(&target.path), target.recovery_ref());
                self.render_confirm_modal(
                    ctx,
                    "Drop Stash",
                    &question,
                    &detail,
                    "Drop Stash",
                    move |app| {
                        branches::stash_drop(target, app.state.clone());
                    },
                );
            }

            ModalKind::MergeBranch { branch } => self.render_merge_modal(ctx, branch),
            ModalKind::GitFlow => self.render_git_flow_modal(ctx),

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
        if self.updater.applying || self.updater.ready_to_close {
            return;
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
            C::Commit => {
                self.repo_ui.right_tab = RightTab::Changes;
                self.staging_panel.request_commit();
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
                    ctx.copy_text(oid);
                }
                None
            }
            C::InteractiveRebase => {
                let mut s = self.state.lock().unwrap();
                s.pending_open_rebase = s.selected_commit.clone();
                None
            }
            C::Undo => {
                self.open_undo(false);
                None
            }
            C::Redo => {
                self.open_undo(true);
                None
            }
            C::IntegratedTerminal => {
                self.toggle_terminal();
                None
            }
            C::GitFlow => {
                self.open_git_flow();
                None
            }
            C::ResetToCommit => {
                let mut s = self.state.lock().unwrap();
                if let Some(oid) = s.selected_commit.clone() {
                    s.modal = ModalKind::ResetToCommit { oid };
                }
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
                    crate::ui::kit::text_field(&mut self.command_palette_query)
                        .hint_text("Search commands")
                        .desired_width(f32::INFINITY),
                );
                input.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Search commands")
                });
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
                            let response = ui.add_enabled(
                                reason.is_none(),
                                egui::Button::selectable(
                                    index == self.palette_selected,
                                    command.label(),
                                ),
                            );
                            crate::ui::motion::hover(ui, &response);
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(command.description()).color(theme::text_muted()),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            RichText::new(shortcut).color(theme::text_muted()),
                                        );
                                    },
                                );
                            });
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
            .frame(egui::Frame::window(&ctx.global_style()).inner_margin(egui::Margin::same(16)))
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
                    crate::ui::kit::text_field(&mut self.branch_switcher_query)
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
                    egui::ScrollArea::vertical().id_salt("pr_list").show(&mut cols[0], |ui| {
                        if visible.is_empty() {
                            ui.label(RichText::new("No pull requests.").color(theme::text_muted()).size(13.0));
                        }
                        for (i, pr) in &visible {
                            let is_sel = pr_selected == Some(*i);
                            let bg = if is_sel { theme::bg_selected() } else { Color32::TRANSPARENT };
                            let (row, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 56.0), egui::Sense::click());
                            ui.painter().rect_filled(row, 0.0, bg);
                            ui.scope_builder(egui::UiBuilder::new().max_rect(row), |ui| {
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
                                egui::ScrollArea::vertical().id_salt("pr_body").max_height(200.0).show(&mut cols[1], |ui| {
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
                                if ui.add_enabled(pr.head_oid.len() == 40, secondary_btn("Checkout Review")).on_hover_text("Load review details to resolve the full source commit.").clicked() {
                                    checkout_review = Some((pr.number, pr.head_oid.clone()));
                                }
                                if let Some(path) = &repo_path {
                                    let remote = self.state.lock().unwrap().pr_source.as_ref().map(|(remote, _, url)| (remote.clone(), url.clone()));
                                    let enabled = self.repo_ui.provider_editor.is_none() && matches!(pr.state, PrState::Open | PrState::Draft) && !pr.head_oid.is_empty();
                                    if ui.add_enabled(enabled && pr.head_oid.len() == 40, secondary_btn("Review...")).clicked() {
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
            let (path, source, epoch) = {
                let s = self.state.lock().unwrap();
                (s.repo_path.clone(), s.pr_source.clone(), s.repo_epoch)
            };
            if let (Some(path), Some((remote, provider, remote_url))) = (path, source) {
                let switch_state = self.state.clone();
                crate::git::mutation::spawn_operation(path, self.state.clone(), move |path| {
                    {
                        let s = switch_state.lock().unwrap();
                        if s.repo_path.as_deref() != Some(path) || s.repo_epoch != epoch {
                            return Ok(());
                        }
                    }
                    crate::git::reviews::verify_remote(path, &remote, &remote_url)?;
                    let target =
                        crate::git::reviews::prepare(path, &remote, &provider, number, &oid)?;
                    crate::git::switch::request_for(path, epoch, target, switch_state.clone());
                    Ok(())
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
            self.config
                .per_repo
                .entry(path.display().to_string())
                .or_default()
                .graph_preferences = Default::default();
        }
        self.save_config();
        self.commit_graph_view.reset_column_widths();
        self.commit_graph_view.set_preferences(Default::default());
    }
}

// ── eframe::App ───────────────────────────────────────────────────────────────

impl eframe::App for RecordeApp {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.persist_navigation();
        self.save_config();
    }

    fn ui(&mut self, root_ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let context = root_ui.ctx().clone();
        let ctx = &context;
        self.git_readiness.poll(ctx);
        self.updater.poll(ctx);
        if self.updater.ready_to_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if self.updater.applying {
            egui::CentralPanel::default().show(root_ui, |ui| {
                ui.spinner();
                ui.label(
                    "Preparing the verified update. Recorde will close when its helper is ready.",
                );
            });
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
            return;
        }

        if ctx.input(|input| input.viewport().close_requested()) {
            self.request_quit(ctx);
        }

        let ready = self.state.lock().unwrap().commit_graph.is_some();
        self.frame_metrics
            .observe(ctx, _frame.info().cpu_usage, ready);
        self.sync_repo_session();
        if !self.settings_open || self.settings_tab != SettingsTab::Repository {
            self.repo_settings.visible = false;
        }
        self.poll_history_plan(ctx);
        self.show_lifecycle_guards(ctx);
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
        ctx.global_style_mut(theme::kill_debug);
        ctx.options_mut(|o| o.warn_on_id_clash = false);

        let shortcuts = !crate::commands::text_input_has_focus(ctx)
            && !self.help.open
            && !self.support.open
            && !self.repository_browser.open
            && !self.settings_open
            && !self.quit_waiting
            && self.pending_navigation.is_none()
            && self.history_plan.is_none()
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
            && self.repo_ui.provider_editor.is_none()
            && self.repo_ui.file_editor.is_none();
        self.commit_graph_view.set_interactions_enabled(
            !self.settings_open
                && !self.quit_waiting
                && self.pending_navigation.is_none()
                && self.history_plan.is_none()
                && !self.help.open
                && !self.support.open
                && !self.repository_browser.open
                && !self.command_palette_open
                && !self.branch_switcher_open
                && !self.add_account_open
                && !self.commit_graph_view.has_pending_action()
                && self.repo_ui.rebase_modal.is_none()
                && self.repo_ui.conflict_editor.is_none()
                && self.repo_ui.provider_editor.is_none()
                && self.repo_ui.file_editor.is_none()
                && {
                    let s = self.state.lock().unwrap();
                    matches!(s.modal, ModalKind::None)
                        && s.pending_discard.is_none()
                        && s.repo_load_path.is_none()
                },
        );
        if let Some(command) = self.config.shortcuts.take_command(
            ctx,
            !shortcuts || self.staging_panel.has_file_confirmation(),
        ) {
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
        if let Some(preferences) = self.commit_graph_view.take_dirty_preferences() {
            if let Some(path) = self.state.lock().unwrap().repo_path.clone() {
                self.config
                    .per_repo
                    .entry(path.display().to_string())
                    .or_default()
                    .graph_preferences = preferences;
                self.config.save();
            }
        }
        if let Some(action) = self.commit_graph_view.take_sidebar_action() {
            self.handle_sidebar(action);
        }

        // ── In-progress operation banner (rebase/merge/cherry-pick/revert) ─
        self.show_in_progress_banner(root_ui);

        // ── Pending actions from graph context menu ───────────────────────
        let pending_rebase = self.state.lock().unwrap().pending_open_rebase.take();
        if let Some(oid) = pending_rebase {
            self.open_rebase_modal(oid);
        }
        let pending_squash = self.state.lock().unwrap().pending_squash.take();
        if let Some(oids) = pending_squash {
            self.open_squash(oids);
        }
        let pending_drop = self.state.lock().unwrap().pending_drop.take();
        if let Some(oids) = pending_drop {
            self.open_drop(oids);
        }
        self.show_file_editor(ctx);
        self.help.show(ctx);
        self.support.show(ctx, self.updater.diagnostics_state());
        if let Some(url) = self.repository_browser.show(ctx, &self.config.accounts) {
            self.clone_url = url;
            self.state.lock().unwrap().modal = ModalKind::CloneRepo;
        }

        // ── Modals ────────────────────────────────────────────────────────
        self.render_modals(ctx);
        self.render_switch_work(ctx);
        if self.updater.applying || self.updater.ready_to_close {
            return;
        }

        // Interactive rebase window
        if self.repo_ui.rebase_modal.is_some() {
            let mut close = false;
            let mut execute: Option<(String, Vec<RebaseEntry>)> = None;
            egui::Window::new("Interactive Rebase")
                .collapsible(false)
                .resizable(true)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .frame(
                    egui::Frame::window(&ctx.global_style()).inner_margin(egui::Margin::same(16)),
                )
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
                if let Some(target) = self.repo_ui.rebase_target.take() {
                    rebase::start_rebase_checked(target, &oid, entries, self.state.clone());
                } else {
                    self.state.lock().unwrap().error =
                        Some("Reopen the rebase plan to review its branch and HEAD.".into());
                }
            } else if close {
                self.repo_ui.rebase_target = None;
            }
        }

        // ── Panels ────────────────────────────────────────────────────────
        self.status_bar.show(root_ui);

        // Title bar
        egui::Panel::top("title_bar")
            .exact_size(40.0)
            .frame(
                egui::Frame::NONE
                    .fill(theme::bg_titlebar())
                    .inner_margin(egui::Margin::symmetric(12, 0)),
            )
            .show(root_ui, |ui| {
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

                    // Repository tabs share the title bar instead of taking a band of
                    // their own; the panel toggles keep the right edge. Their width is
                    // measured on the previous frame so the tabs can scroll before them.
                    let toggles_id = egui::Id::new("panel_toggles_width");
                    let toggles_width = ui
                        .data(|data| data.get_temp::<f32>(toggles_id))
                        .unwrap_or(260.0);
                    ui.add_space(crate::ui::kit::space::SM);
                    let tabs_width =
                        (ui.available_width() - toggles_width - crate::ui::kit::space::MD).max(0.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(tabs_width, 32.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.set_width(tabs_width);
                            self.show_repo_tabs(ui);
                        },
                    );
                    ui.add_space(crate::ui::kit::space::MD);
                    let toggles_start = ui.cursor().left();
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
                    if panel_toggle(
                        ui,
                        &mut self.config.show_terminal,
                        "Terminal",
                        "Show or hide the integrated terminal",
                    ) {
                        layout_changed = true;
                        if self.config.show_terminal {
                            self.terminal.focus();
                        }
                    }
                    if layout_changed {
                        self.save_config();
                    }
                    let measured = ui.min_rect().right() - toggles_start;
                    if (measured - toggles_width).abs() > 0.5 {
                        ui.data_mut(|data| data.insert_temp(toggles_id, measured));
                        ui.ctx().request_repaint();
                    }

                    // Icon is rendered inside menu::bar (see show_menu_contents),
                    // because menu::bar expands to fill all available width.
                });
            });

        let has_workspace = {
            let state = self.state.lock().unwrap();
            state.repo_path.is_some() || state.repo_load_path.is_some()
        };
        if !has_workspace {
            self.show_welcome(root_ui);
            self.commit_graph_view
                .show_action_confirmation(ctx, &self.config.protected_branches);
            self.show_operation_overlays(ctx);
            return;
        }

        // Toolbar
        let (search_active, show_all_branches, pull_strategy) = {
            let s = self.state.lock().unwrap();
            let pull_strategy = match self.config.pull_strategy {
                PullStrategy::GitConfig => s
                    .capabilities
                    .as_ref()
                    .map(|caps| caps.pull_strategy)
                    .unwrap_or_default(),
                PullStrategy::Merge => remote::EffectivePull::Merge,
                PullStrategy::Rebase => remote::EffectivePull::Rebase,
                PullStrategy::FfOnly => remote::EffectivePull::FastForwardOnly,
            };
            (s.search_active, s.show_all_branches, pull_strategy)
        };
        let toolbar_action = self.toolbar.show(
            root_ui,
            search_active,
            show_all_branches,
            !self.commit_graph_view.has_pending_action(),
            pull_strategy,
        );
        if let Some(a) = toolbar_action {
            self.handle_toolbar(ctx, a);
        }

        let sidebar_id = egui::Id::new(("sidebar", &self.observed_repo));
        let sidebar_open = self.config.show_sidebar;
        // Persist sizes only once the slide has settled.
        let sidebar_settled =
            ctx.animate_bool_responsive(sidebar_id.with("animation"), sidebar_open) >= 1.0;
        let mut sidebar_shown = sidebar_open;
        {
            let sidebar_response = egui::Panel::left(sidebar_id)
                .resizable(true)
                .default_size(self.config.sidebar_width.unwrap_or(216.0))
                .size_range(180.0..=360.0)
                .frame(egui::Frame::NONE)
                .show_collapsible(root_ui, &mut sidebar_shown, |ui| {
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
            if let Some(sidebar_response) = sidebar_response.filter(|_| sidebar_settled) {
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
        }
        if sidebar_shown != sidebar_open {
            // Dragging a panel edge past its minimum collapses it.
            self.config.show_sidebar = sidebar_shown;
            self.save_config();
        }

        let right_panel_id = egui::Id::new(("right_panel_v2", &self.observed_repo));
        let right_panel_open = self.config.show_right_panel;
        // Persist sizes only once the slide has settled.
        let right_panel_settled =
            ctx.animate_bool_responsive(right_panel_id.with("animation"), right_panel_open) >= 1.0;
        let mut right_panel_shown = right_panel_open;
        {
            let right_response = egui::Panel::right(right_panel_id)
                .resizable(true)
                .default_size(self.config.right_panel_width.unwrap_or(344.0))
                .size_range(300.0..=520.0)
                .frame(egui::Frame::NONE)
                .show_collapsible(root_ui, &mut right_panel_shown, |ui| {
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

                    ui.scope_builder(egui::UiBuilder::new().max_rect(tab_rect), |ui| {
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
                                        Button::new(RichText::new(*lbl).size(14.0).color(text_col))
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
                    let mut content = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(content_rect)
                            .layout(egui::Layout::top_down(egui::Align::Min)),
                    );
                    content.set_clip_rect(content_rect);
                    match self.repo_ui.right_tab {
                        RightTab::Changes => self.staging_panel.show_with_shortcuts(
                            &mut content,
                            &self.config.default_editor,
                            &self.config.protected_branches,
                            &self.config.shortcuts,
                        ),
                        RightTab::Details => self.detail_panel.show(&mut content),
                    }
                    ui.set_min_size(ui.available_size());
                    if self.state.lock().unwrap().focused_pane == 3 {
                        focus_ring(ui);
                    }
                });
            if let Some(right_response) = right_response.filter(|_| right_panel_settled) {
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
        }
        if right_panel_shown != right_panel_open {
            // Dragging a panel edge past its minimum collapses it.
            self.config.show_right_panel = right_panel_shown;
            self.save_config();
        }

        if self.config.show_terminal {
            self.show_terminal_panel(root_ui);
        } else {
            // Hiding the panel must not stop draining its bounded output queue
            // or postpone cleanup of a completed command's descendants.
            let finished = self.terminal.poll();
            if finished.is_some() && finished == self.state.lock().unwrap().repo_path {
                crate::git::operations::refresh_repo(self.state.clone());
            }
            if self.terminal.needs_poll() {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
        }

        let diff_panel_id = egui::Id::new(("diff_panel", &self.observed_repo));
        let diff_panel_open = self.config.show_diff_panel;
        // Persist sizes only once the slide has settled.
        let diff_panel_settled =
            ctx.animate_bool_responsive(diff_panel_id.with("animation"), diff_panel_open) >= 1.0;
        let mut diff_panel_shown = diff_panel_open;
        {
            let max_height = (root_ui.available_height() - 180.0).clamp(120.0, 480.0);
            let diff_response = egui::Panel::bottom(diff_panel_id)
                .resizable(true)
                .default_size(
                    self.config
                        .diff_panel_height
                        .unwrap_or(300.0)
                        .min(max_height),
                )
                .size_range(120.0..=max_height)
                .frame(egui::Frame::NONE.fill(theme::bg_app()))
                .show_collapsible(root_ui, &mut diff_panel_shown, |ui| {
                    let r = ui.max_rect();
                    // In a short window the diff can need more height than its
                    // panel; clip it so it cannot cover (or take clicks from)
                    // the terminal panel below.
                    ui.shrink_clip_rect(r);
                    ui.painter().line_segment(
                        [r.left_top(), r.right_top()],
                        Stroke::new(1.0, theme::border()),
                    );
                    if self.commit_graph_view.has_pending_action() {
                        ui.disable();
                    }
                    // Lay the diff out in a child that cannot resize the panel:
                    // the panel keeps its own size, shrinking with the window,
                    // and its content is clipped to it rather than growing it.
                    let mut content = ui.new_child(egui::UiBuilder::new().max_rect(r));
                    self.diff_view.show(&mut content);
                    ui.advance_cursor_after_rect(r);
                    if self.state.lock().unwrap().focused_pane == 4 {
                        focus_ring(ui);
                    }
                });
            if let Some(diff_response) = diff_response.filter(|_| diff_panel_settled) {
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
        }
        if diff_panel_shown != diff_panel_open {
            // Dragging a panel edge past its minimum collapses it.
            self.config.show_diff_panel = diff_panel_shown;
            self.save_config();
        }

        // Central panel — commit graph (always)
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(theme::bg_app()))
            .show(root_ui, |ui| {
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
                crate::ui::kit::text_field(&mut self.repo_ui.rename_branch_input)
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

    /// A destructive confirmation with a red confirm button. Escape cancels.
    fn render_confirm_modal(
        &mut self,
        ctx: &Context,
        title: &str,
        question: &str,
        detail: &str,
        confirm_label: &str,
        on_confirm: impl FnOnce(&mut Self),
    ) {
        let mut confirm = false;
        let mut close = false;
        modal_window(ctx, title, |ui| {
            ui.label(
                RichText::new(question)
                    .color(theme::text_primary())
                    .size(14.0),
            );
            ui.add_space(6.0);
            ui.label(styled_muted(detail));
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                confirm = ui.add(danger_btn(confirm_label)).clicked();
                close = ui.add(secondary_btn("Cancel")).clicked()
                    || ui.input(|i| i.key_pressed(Key::Escape));
            });
        });
        if confirm {
            self.state.lock().unwrap().modal = ModalKind::None;
            on_confirm(self);
        } else if close {
            self.state.lock().unwrap().modal = ModalKind::None;
        }
    }

    fn render_reset_modal(&mut self, ctx: &Context, oid: String) {
        let short = &oid[..7.min(oid.len())];
        let (path, epoch, dirty) = {
            let s = self.state.lock().unwrap();
            (s.repo_path.clone(), s.repo_epoch, s.dirty_count)
        };
        let Some(path) = path else { return };
        if self
            .repo_ui
            .reset_preview
            .as_ref()
            .is_none_or(|preview| preview.oid != oid || preview.epoch != epoch)
        {
            match branches::ResetPreview::capture(&path, epoch, &oid) {
                Ok(preview) => self.repo_ui.reset_preview = Some(preview),
                Err(error) => {
                    let mut state = self.state.lock().unwrap();
                    state.error = Some(error);
                    state.modal = ModalKind::None;
                    self.repo_ui.reset_preview = None;
                    return;
                }
            }
        }
        let preview = self.repo_ui.reset_preview.clone().unwrap();
        let branch = preview
            .head_ref
            .strip_prefix("refs/heads/")
            .unwrap_or("detached HEAD");
        let target = format!("Reset {branch} to {short}");
        let mut confirm = false;
        let mut close = false;
        modal_window(ctx, "Reset to Commit", |ui| {
            ui.label(
                RichText::new(&target)
                    .color(theme::text_primary())
                    .size(14.0),
            );
            ui.label(styled_muted(&format!(
                "{}\nCurrent HEAD: {}",
                crate::ui::kit::display_path(&preview.path),
                preview.head_oid
            )));
            ui.add_space(8.0);
            crate::ui::kit::hint(
                ui,
                "Move the branch back to this commit and choose what happens to your file changes.",
            );
            for mode in branches::ResetMode::ALL {
                let destructive = mode == branches::ResetMode::Hard;
                crate::ui::kit::radio_value(
                    ui,
                    &mut self.repo_ui.reset_mode,
                    mode,
                    RichText::new(mode.label()).color(if destructive {
                        theme::accent_red()
                    } else {
                        theme::text_primary()
                    }),
                );
                crate::ui::kit::hint(
                    ui,
                    match mode {
                        branches::ResetMode::Soft => {
                            "Keep your file changes selected for the next commit (staged)."
                        }
                        branches::ResetMode::Mixed => {
                            "Keep your file changes, then choose what goes into the next commit."
                        }
                        branches::ResetMode::Hard => {
                            "Replace tracked files with this commit's versions \
                             and discard their changes."
                        }
                    },
                );
            }
            ui.add_space(8.0);
            ui.label(styled_muted(
                "A recovery snapshot of HEAD, the index and working files is saved first. \
                 Restore it from Repository settings > Recovery snapshots.",
            ));
            if self.repo_ui.reset_mode == branches::ResetMode::Hard && dirty > 0 {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!(
                        "{dirty} changed file(s) will be overwritten. Untracked files are kept."
                    ))
                    .color(theme::accent_red())
                    .size(12.0),
                );
            }
            ui.add_space(16.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let label = format!("{} Reset", self.repo_ui.reset_mode.label());
                confirm = if self.repo_ui.reset_mode == branches::ResetMode::Hard {
                    ui.add(danger_btn(&label)).clicked()
                } else {
                    ui.add(primary_btn(&label)).clicked()
                };
                close = ui.add(secondary_btn("Cancel")).clicked()
                    || ui.input(|i| i.key_pressed(Key::Escape));
            });
        });
        if confirm {
            self.state.lock().unwrap().modal = ModalKind::None;
            preview.submit(self.repo_ui.reset_mode, self.state.clone());
            self.repo_ui.reset_preview = None;
            self.repo_ui.reset_mode = Default::default();
        } else if close {
            self.state.lock().unwrap().modal = ModalKind::None;
            self.repo_ui.reset_preview = None;
            self.repo_ui.reset_mode = Default::default();
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
            ui.label("You'll be viewing this commit directly, not a branch. New commits you make here won't belong to any branch unless you create one.");
            ui.label(styled_muted("Git calls this detached HEAD. Create a branch from this commit to keep new commits on a branch."));
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
            crate::ui::kit::hint(
                ui,
                "A stash saves uncommitted changes and sets them aside \
                 so you can restore them later.",
            );
            ui.add_space(crate::ui::kit::space::SM);
            ui.label(styled_label("Stash message"));
            ui.add_space(4.0);
            let r = ui.add(
                crate::ui::kit::text_field(&mut self.repo_ui.stash_message_input)
                    .desired_width(f32::INFINITY)
                    .margin(egui::Vec2::new(8.0, 5.0))
                    .hint_text("WIP"),
            );
            r.request_focus();
            ui.add_space(8.0);
            ui.checkbox(
                &mut self.repo_ui.stash_include_untracked,
                styled_label("Include untracked files"),
            );
            ui.add_space(12.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
            branches::stash_save_with_message(
                &msg,
                self.repo_ui.stash_include_untracked,
                self.state.clone(),
            );
            self.repo_ui.stash_message_input = "WIP".into();
            self.state.lock().unwrap().modal = ModalKind::None;
        } else if close {
            self.repo_ui.stash_message_input = "WIP".into();
            self.state.lock().unwrap().modal = ModalKind::None;
        }
    }

    fn render_stash_branch_modal(&mut self, ctx: &Context, target: branches::StashTarget) {
        let mut close = false;
        let mut do_branch = false;
        modal_window(ctx, "Create Branch From Stash", |ui| {
            crate::ui::kit::hint(
                ui,
                "A stash is a saved set of changes you have not committed yet.",
            );
            ui.add_space(crate::ui::kit::space::SM);
            ui.label(styled_label("Branch name"));
            ui.add_space(4.0);
            let r = ui.add(
                crate::ui::kit::text_field(&mut self.repo_ui.stash_branch_input)
                    .desired_width(f32::INFINITY)
                    .margin(egui::Vec2::new(8.0, 5.0))
                    .hint_text("e.g. feat/my-feature"),
            );
            r.request_focus();
            ui.add_space(4.0);
            ui.label(styled_muted(
                "Create and switch to a branch where these changes started, restore them, \
                 then remove the stash after it is successfully applied.",
            ));
            ui.add_space(12.0);
            let can = !self.repo_ui.stash_branch_input.trim().is_empty();
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
                target,
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
                crate::ui::kit::text_field(&mut self.clone_url)
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
            ui.add_space(crate::ui::kit::space::SM);
            if ui.button("Browse connected accounts…").clicked() {
                self.repository_browser.open = true;
                close = true;
            }
            crate::ui::kit::disclosure(ui, "clone_advanced", "Advanced options", false, |ui| {
                crate::ui::kit::radio_value(
                    ui,
                    &mut self.clone_depth,
                    0,
                    "Download full history (recommended)",
                );
                if crate::ui::kit::radio(
                    ui,
                    self.clone_depth > 0,
                    "Only the latest N commits (faster for big repositories)",
                )
                .clicked()
                {
                    self.clone_depth = self.clone_depth.max(1);
                }
                let shallow = self.clone_depth > 0;
                let depth_id = ui.id().with("clone_depth");
                crate::ui::motion::reveal(ui, depth_id, shallow, |ui| {
                    ui.add_enabled_ui(shallow, |ui| {
                        ui.horizontal(|ui| {
                            let label = ui.label("Number of commits");
                            let mut depth = self.clone_depth.max(1);
                            if ui
                                .add(egui::DragValue::new(&mut depth).range(1..=1000000))
                                .labelled_by(label.id)
                                .changed()
                            {
                                self.clone_depth = depth;
                            }
                        });
                        crate::ui::kit::hint(
                            ui,
                            "Download the rest of the history later in repository settings.",
                        );
                    });
                });
            });
            let can_clone = !self.clone_url.trim().is_empty() && self.clone_dest.is_some();
            ui.add_space(crate::ui::kit::space::MD);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let ok = crate::ui::kit::explain_disabled(
                    ui.add_enabled(can_clone, primary_btn("Clone")),
                    "Enter a repository URL and a destination folder.",
                )
                .clicked()
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
            crate::ui::kit::hint(
                ui,
                "Initialising adds Git's history folder to your project so you can save changes \
                 as commits. The repository starts on this computer; you can publish it later.",
            );
            ui.add_space(crate::ui::kit::space::SM);
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
            ui.add_space(10.0);
            self.render_init_options(ui);
            ui.add_space(12.0);
            let can_init = self.init_repo_path.is_some();
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let ok = crate::ui::kit::explain_disabled(
                    ui.add_enabled(can_init, primary_btn("Initialize")),
                    "Choose a destination folder first.",
                )
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
                lifecycle::init_repo(path, self.init_options.clone(), self.state.clone());
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
        use crate::ui::{kit, motion};

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

        egui::Window::new("Recover a previous branch state")
            .id(egui::Id::new("Reflog"))
            .collapsible(false)
            .resizable(true)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .default_size([620.0, 480.0])
            .max_size(ctx.content_rect().size() * 0.9)
            .show(ctx, |ui| {
                let entries = self.state.lock().unwrap().reflog_entries.clone();
                let fade_id = ui.id().with("recovery_fade");
                motion::fade(ui, fade_id, true);
                kit::hint(
                    ui,
                    "Choose a previous state to keep on a new branch. Recover creates that branch \
                     at the selected commit; your current branch and files stay as they are.",
                );
                ui.add_space(kit::space::MD);

                egui::ScrollArea::vertical()
                    .id_salt("recovery_entries")
                    .max_height((ui.available_height() - 180.0).clamp(80.0, 340.0))
                    .show(ui, |ui| {
                        for entry in &entries {
                            ui.push_id((&entry.refname, &entry.oid), |ui| {
                                let mut select = false;
                                let response = kit::card(ui, |ui| {
                                    ui.vertical(|ui| {
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(&entry.message)
                                                    .strong()
                                                    .color(theme::text_primary()),
                                            )
                                            .wrap(),
                                        );
                                        select = ui
                                            .add(kit::secondary_button("Recover as Branch…"))
                                            .on_hover_text(
                                                "Choose a name for a new branch at this commit.",
                                            )
                                            .clicked();
                                        kit::disclosure(
                                            ui,
                                            "recovery_details",
                                            "Technical details",
                                            false,
                                            |ui| {
                                                kit::hint(
                                                    ui,
                                                    "Git's reflog records previous \
                                                     branch positions.",
                                                );
                                                ui.label(format!(
                                                    "Reflog entry: {}",
                                                    entry.refname,
                                                ));
                                                ui.add(
                                                    egui::Label::new(
                                                        RichText::new(format!(
                                                            "Commit ID: {}",
                                                            entry.oid,
                                                        ))
                                                        .monospace(),
                                                    )
                                                    .wrap()
                                                    .selectable(true),
                                                );
                                            },
                                        );
                                    })
                                    .response
                                });
                                response.context_menu(|ui| {
                                    ui.set_min_width(180.0);
                                    if ui.button("Recover as Branch…").clicked() {
                                        select = true;
                                        ui.close();
                                    }
                                    if ui.button("Copy SHA").clicked() {
                                        ui.ctx().copy_text(entry.oid.clone());
                                        ui.close();
                                    }
                                });
                                if select {
                                    self.repo_ui.reflog_recover_oid = Some(entry.oid.clone());
                                    self.repo_ui.reflog_recover_input =
                                        format!("recover-{}", &entry.short_oid);
                                }
                            });
                            ui.add_space(crate::ui::kit::space::SM);
                        }
                        if entries.is_empty() {
                            ui.label(
                                RichText::new("No reflog entries.")
                                    .color(theme::text_muted())
                                    .size(13.0),
                            );
                        }
                    });

                let recovering = self.repo_ui.reflog_recover_oid.is_some();
                let recovery_id = ui.id().with("recovery_name");
                motion::reveal(ui, recovery_id, recovering, |ui| {
                    ui.add_enabled_ui(recovering, |ui| {
                        ui.separator();
                        ui.add_space(crate::ui::kit::space::SM);
                        let label = ui.label(
                            RichText::new("Branch name for recovery:")
                                .color(theme::text_secondary())
                                .size(13.0),
                        );
                        ui.add_space(kit::space::XS);
                        ui.add(
                            crate::ui::kit::text_field(&mut self.repo_ui.reflog_recover_input)
                                .desired_width(f32::INFINITY)
                                .margin(egui::Vec2::new(8.0, 5.0)),
                        )
                        .labelled_by(label.id);
                        ui.add_space(crate::ui::kit::space::SM);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let can = !self.repo_ui.reflog_recover_input.trim().is_empty();
                            if kit::explain_disabled(
                                ui.add_enabled(can, primary_btn("Recover")),
                                "Enter a name for the recovery branch.",
                            )
                            .clicked()
                            {
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
                    });
                });

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

        let (repo_path, bisect_active, bisect_oid, selected_oid, first_bad) = {
            let s = self.state.lock().unwrap();
            (
                s.repo_path.clone(),
                s.bisect_in_progress,
                s.bisect_current_oid.clone(),
                s.selected_commit.clone(),
                s.bisect_first_bad.clone(),
            )
        };

        let mut close = false;
        let mut reveal: Option<String> = None;
        let mut action: Option<bisect::Step> = None;

        modal_window(ctx, "Bisect", |ui| {
            ui.set_min_width(380.0);

            if !bisect_active {
                ui.label(
                    RichText::new("Start a bisect session to find which commit introduced a bug.")
                        .color(theme::text_secondary())
                        .size(13.0),
                );
                ui.add_space(8.0);
                ui.label(RichText::new("After starting, right-click a known-bad commit in the graph and mark it bad, then mark a known-good commit good. Recorde checks out each commit to test and selects it.").color(theme::text_muted()).size(12.0));
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.add(primary_btn("Start Bisect")).clicked() {
                        action = Some(bisect::Step::Start);
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

                if let Some(ref oid) = first_bad {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("First bad commit:")
                                .color(theme::text_primary())
                                .size(13.0)
                                .strong(),
                        );
                        ui.label(
                            RichText::new(&oid[..7.min(oid.len())])
                                .monospace()
                                .color(theme::accent_red())
                                .size(13.0),
                        );
                        if ui.button("Show in graph").clicked() {
                            reveal = Some(oid.clone());
                        }
                    });
                    ui.label(styled_muted(
                        "Reset ends the session and returns to the branch you started on.",
                    ));
                    ui.add_space(6.0);
                } else if let Some(ref oid) = bisect_oid {
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
                            Button::new(
                                RichText::new("Mark Good")
                                    .color(theme::contrast_text(theme::accent_green())),
                            )
                            .fill(theme::accent_green())
                            .min_size([110.0, 26.0].into()),
                        )
                        .clicked()
                    {
                        action = Some(bisect::Step::Good);
                    }
                    ui.add_space(6.0);
                    if ui
                        .add_enabled(
                            has_sel,
                            Button::new(
                                RichText::new("Mark Bad")
                                    .color(theme::contrast_text(theme::accent_red())),
                            )
                            .fill(theme::accent_red())
                            .min_size([110.0, 26.0].into()),
                        )
                        .clicked()
                    {
                        action = Some(bisect::Step::Bad);
                    }
                    ui.add_space(12.0);
                    if ui.add(secondary_btn("Reset")).clicked() {
                        action = Some(bisect::Step::Reset);
                    }
                    if ui.add(secondary_btn("Close")).clicked()
                        || ui.input(|i| i.key_pressed(Key::Escape))
                    {
                        close = true;
                    }
                });
                ui.add_space(6.0);
                ui.label(styled_muted(
                    "Close keeps the session. Right-click commits in the graph to mark them good or bad.",
                ));
            }
        });

        if let Some(step) = action {
            if repo_path.is_some() {
                bisect::spawn_step(step, selected_oid, self.state.clone());
            }
            // Close after starting too: commits are chosen in the graph, which
            // modal dialogs disable. The commit menu offers Mark Good/Bad.
            if matches!(step, bisect::Step::Start | bisect::Step::Reset) {
                self.state.lock().unwrap().modal = ModalKind::None;
            }
        }
        if let Some(oid) = reveal {
            self.commit_graph_view.reveal_commit(&oid);
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
                        .id_salt("cmp_files")
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
                            .id_salt("cmp_diff")
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
        let branches_list = self.branch_choices();

        let mut close = false;
        let mut chosen: Option<String> = None;

        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(egui::Frame::window(&ctx.global_style()).inner_margin(egui::Margin::same(16)))
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
                            ui.label(styled_muted("No other branches."));
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
    fn show_in_progress_banner(&mut self, root_ui: &mut egui::Ui) {
        use crate::ui::{kit, motion};

        let (caps, conflicts, bisect_active) = {
            let state = self.state.lock().unwrap();
            (
                state.capabilities.clone().unwrap_or_default(),
                state.conflicted_files.len(),
                state.bisect_in_progress,
            )
        };
        let animation_id = egui::Id::new("git_operation_banner");

        let (label, show_skip, is_bisect) = if caps.rebase_in_progress {
            ("Rebase in progress", true, false)
        } else if caps.merge_in_progress {
            (
                if caps.squash_merge_in_progress {
                    "Squash merge in progress"
                } else {
                    "Merge in progress"
                },
                false,
                false,
            )
        } else if caps.cherry_pick_in_progress {
            ("Cherry-pick in progress", false, false)
        } else if caps.revert_in_progress {
            ("Revert in progress", false, false)
        } else if bisect_active {
            ("Bisect in progress", false, true)
        } else {
            motion::progress(root_ui.ctx(), animation_id, false, motion::SLOW);
            return;
        };

        let guidance = if is_bisect {
            "Test this commit, then use Bisect to mark it good or bad and find where the bug began."
                .to_owned()
        } else if conflicts > 0 {
            format!(
                "{conflicts} {} — resolve them in Changes, then stage the resolved files.",
                if conflicts == 1 {
                    "file has conflicts"
                } else {
                    "files have conflicts"
                },
            )
        } else if caps.rebase_split {
            "Stage files or lines, commit each part, then continue.".to_owned()
        } else if caps.rebase_edit {
            "Amend this commit, or split it into smaller commits, then click Continue.".to_owned()
        } else {
            "All conflicts resolved — click Continue.".to_owned()
        };

        let mut do_continue = false;
        let mut do_skip = false;
        let mut do_abort = false;
        let mut do_split = false;
        let first_show = root_ui.ctx().data_mut(|data| {
            let seen = data.get_temp::<bool>(animation_id).unwrap_or(false);
            data.insert_temp(animation_id, true);
            !seen
        });
        if first_show {
            motion::progress(root_ui.ctx(), animation_id, false, motion::SLOW);
        }
        let banner_progress = motion::progress(root_ui.ctx(), animation_id, true, motion::SLOW);

        egui::Panel::top("in_progress_banner")
            .default_size(0.0)
            .min_size(0.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(theme::accent_yellow().gamma_multiply(0.10 * banner_progress))
                    .stroke(Stroke::new(
                        1.0,
                        theme::accent_yellow().gamma_multiply(0.45 * banner_progress),
                    ))
                    .inner_margin(egui::Margin::symmetric(16, 8)),
            )
            .show(root_ui, |ui| {
                ui.multiply_opacity(banner_progress);
                ui.add_space(motion::slide(banner_progress, -8.0));
                ui.horizontal_wrapped(|ui| {
                    kit::status_chip(ui, kit::Status::Warning, label);
                    kit::hint(ui, &guidance);
                });
                if is_bisect {
                    return;
                }
                ui.add_space(kit::space::XS);
                ui.horizontal_wrapped(|ui| {
                    if caps.rebase_edit && !caps.rebase_split {
                        if ui
                            .add(kit::secondary_button("Split commit"))
                            .on_hover_text(
                                "Uncommit this edit step, keep its files, then stage and commit \
                                 each part in Changes.",
                            )
                            .clicked()
                        {
                            do_split = true;
                        }
                    }
                    if ui
                        .add(kit::primary_button("Continue"))
                        .on_hover_text(
                            "Continue the operation after resolving and staging conflicts.",
                        )
                        .clicked()
                    {
                        do_continue = true;
                    }
                    if show_skip && !caps.rebase_split {
                        if ui
                            .add(kit::secondary_button("Skip"))
                            .on_hover_text("Skip this commit and continue the rebase.")
                            .clicked()
                        {
                            do_skip = true;
                        }
                    }
                    if ui
                        .add(kit::danger_outline_button("Abort"))
                        .on_hover_text("Stop the operation and return to its starting state.")
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
        {
            let s = self.state.lock().unwrap();
            self.auto_fetch_failures = s
                .background_fetch
                .as_ref()
                .filter(|status| status.repo_path == path)
                .map_or(0, |status| status.consecutive_failures);
        }
        let interval = (self.config.auto_fetch_interval_secs.max(30) as u64)
            .saturating_mul(1_u64 << self.auto_fetch_failures.min(7))
            .min(3600);
        let failed_at = {
            let s = self.state.lock().unwrap();
            s.background_fetch
                .as_ref()
                .filter(|status| status.repo_path == path)
                .map(|status| status.failed_at)
        };
        let since = failed_at.map_or(self.last_auto_fetch, |failed| {
            failed.max(self.last_auto_fetch)
        });
        let due = changed || since.elapsed().as_secs() >= interval;
        if self.config.auto_fetch && due && !busy && !sequence && !self.runner.has_jobs(&path) {
            self.runner
                .submit(crate::git::job::JobKind::BackgroundFetch, path.clone());
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
                self.commit_graph_view
                    .set_preferences(layout.graph_preferences.clone());
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

    /// Draws the favourite toggle, Workspaces menu and repository tabs into the
    /// title bar region it is given; the tabs scroll sideways when crowded.
    fn show_repo_tabs(&mut self, ui: &mut egui::Ui) {
        let mut tab_action: Option<ToolbarAction> = None;
        let mut favorite_toggle: Option<std::path::PathBuf> = None;
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
        ui.scope(|ui| {
            if self.commit_graph_view.has_pending_action() {
                ui.disable();
            }
            egui::ScrollArea::horizontal()
                .id_salt("repository_tabs")
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        // Workspaces is a quiet text menu like the panel toggles; the
                        // favourite star moved into it and into each tab's context menu.
                        let mut title = egui::text::LayoutJob::default();
                        title.append(
                            "Workspaces",
                            0.0,
                            egui::TextFormat {
                                font_id: egui::FontId::proportional(13.0),
                                color: theme::text_secondary(),
                                valign: egui::Align::Center,
                                ..Default::default()
                            },
                        );
                        title.append(
                            &crate::icons::EXPAND_MORE.to_string(),
                            4.0,
                            egui::TextFormat {
                                font_id: crate::icons::font(16.0),
                                color: theme::text_muted(),
                                valign: egui::Align::Center,
                                ..Default::default()
                            },
                        );
                        let picker = ui
                            .scope(|ui| {
                                ui.spacing_mut().button_padding = egui::vec2(10.0, 4.0);
                                ui.add(
                                    Button::new(title)
                                        .fill(Color32::TRANSPARENT)
                                        .stroke(Stroke::NONE)
                                        .corner_radius(6.0)
                                        .min_size(egui::vec2(0.0, 26.0)),
                                )
                            })
                            .inner
                            .on_hover_text("Favourite repositories and saved sets of tabs");
                        // Keep the picker's accessible role and value for assistive
                        // technology and the native acceptance inspector.
                        picker.widget_info(|| {
                            let mut info = egui::WidgetInfo::new(egui::WidgetType::ComboBox);
                            info.enabled = ui.is_enabled();
                            info.current_text_value = Some("Workspaces".into());
                            info
                        });
                        egui::Popup::menu(&picker)
                            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                            .show(|ui| {
                                ui.set_min_width(240.0);
                                if let Some(path) = &active {
                                    let favorite = self.config.favorites.contains(path);
                                    if ui.button(favorite_label(path, favorite)).clicked() {
                                        favorite_toggle = Some(path.clone());
                                        ui.close();
                                    }
                                    ui.separator();
                                }
                                ui.strong("Favorites");
                                if self.config.favorites.is_empty() {
                                    ui.label(
                                        RichText::new("No favourites yet.")
                                            .size(crate::theme::font_size::SMALL)
                                            .color(theme::text_muted()),
                                    );
                                }
                                for path in &self.config.favorites {
                                    if ui.button(crate::ui::kit::display_path(path)).clicked() {
                                        open = Some(path.clone());
                                        ui.close();
                                    }
                                }
                                ui.separator();
                                ui.strong("Saved workspaces");
                                for (name, paths) in &self.config.workspaces {
                                    if ui.button(name).clicked() {
                                        workspace = Some(paths.clone());
                                        ui.close();
                                    }
                                }
                                ui.separator();
                                ui.add(
                                    crate::ui::kit::text_field(&mut self.workspace_name)
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
                            let tab = repo_tab(ui, label.as_ref(), active.as_ref() == Some(path))
                                .on_hover_text(crate::ui::kit::display_path(path));
                            if tab.clicked() {
                                open = Some(path.clone());
                            }
                            let favorite = self.config.favorites.contains(path);
                            tab.context_menu(|ui| {
                                if ui.button(favorite_label(path, favorite)).clicked() {
                                    favorite_toggle = Some(path.clone());
                                    ui.close();
                                }
                            });
                            if self.config.open_repos.len() > 1 {
                                let response =
                                    ui.small_button("×").on_hover_text("Close repository tab");
                                response.widget_info(|| {
                                    egui::WidgetInfo::labeled(
                                        egui::WidgetType::Button,
                                        ui.is_enabled(),
                                        format!("Close repository tab {label}"),
                                    )
                                });
                                if response.clicked() {
                                    close = Some(path.clone());
                                }
                            }
                            ui.separator();
                        }
                        // One place to open, clone, create or reopen a repository;
                        // the toolbar no longer repeats the name in a second picker.
                        ui.menu_button("+", |ui| {
                            ui.set_min_width(220.0);
                            if ui.button("Open Repository…").clicked() {
                                open = rfd::FileDialog::new().pick_folder();
                                ui.close();
                            }
                            if ui.button("Clone Repository…").clicked() {
                                tab_action = Some(ToolbarAction::CloneRepo);
                                ui.close();
                            }
                            if ui.button("Initialize Repository…").clicked() {
                                tab_action = Some(ToolbarAction::InitRepo);
                                ui.close();
                            }
                            ui.separator();
                            if let Some(selected) = crate::ui::toolbar::recent_repositories_menu(
                                ui,
                                &self.config.recent_repos,
                                active.as_deref(),
                            ) {
                                tab_action = Some(selected);
                            }
                        })
                        .response
                        .on_hover_text(
                            "Open, clone or create a repository, or reopen a recent one",
                        );
                    });
                });
        });
        if let Some(path) = favorite_toggle {
            if self.config.favorites.contains(&path) {
                self.config.favorites.retain(|p| p != &path);
            } else {
                self.config.favorites.push(path);
            }
            self.save_config();
        }
        if let Some(action) = tab_action {
            let ctx = ui.ctx().clone();
            self.handle_toolbar(&ctx, action);
        }
        if let Some(paths) = workspace {
            if !paths.is_empty() {
                self.request_navigation(lifecycle_guards::Navigation::Workspace(paths));
            }
        }
        if let Some(path) = close {
            self.request_navigation(lifecycle_guards::Navigation::Close(path));
        }
        if let Some(path) = open {
            self.config.add_recent(path.clone());
            load_repo(path, self.state.clone());
        }
    }

    fn show_operation_overlays(&mut self, ctx: &Context) {
        let (legacy_op, active_ops) = {
            let s = self.state.lock().unwrap();
            (
                s.remote_op.clone(),
                s.active_ops
                    .iter()
                    .filter(|op| !op.background)
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
                        let progress = s.local_job_progress.get(&job.id);
                        (
                            format!("local_{}", job.id),
                            progress.map_or_else(
                                || {
                                    format!(
                                        "{status}{} ({})",
                                        job.label,
                                        job.path.file_name().unwrap_or_default().to_string_lossy()
                                    )
                                },
                                |(message, _)| message.clone(),
                            ),
                            progress.and_then(|(_, pct)| *pct),
                            job.cancel.clone(),
                        )
                    }))
                    .collect::<Vec<_>>(),
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
                    egui::Frame::NONE
                        .fill(theme::bg_elevated())
                        .stroke(Stroke::new(1.0, theme::accent_teal()))
                        .corner_radius(8.0)
                        .inner_margin(egui::Margin::symmetric(14, 10))
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
                    egui::Frame::NONE
                        .fill(theme::bg_elevated())
                        .stroke(Stroke::new(1.0, theme::accent_teal()))
                        .corner_radius(8.0)
                        .inner_margin(egui::Margin::symmetric(14, 10))
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
                    egui::Frame::NONE
                        .fill(theme::bg_diff_del())
                        .stroke(Stroke::new(1.5, theme::accent_red()))
                        .corner_radius(8.0)
                        .inner_margin(egui::Margin::symmetric(16, 12))
                        .show(ui, |ui| {
                            ui.set_max_width(640.0);
                            ui.horizontal_centered(|ui| {
                                ui.label(
                                    RichText::new(&desc).color(theme::text_primary()).size(13.0),
                                );
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

        let queued_errors = {
            let mut state = self.state.lock().unwrap();
            state.take_pending_error();
            state.errors.clone()
        };
        let mut errors = queued_errors.clone();
        if crate::ui::error_notice::show(ctx, &mut errors) {
            self.git_readiness.retry();
            self.open_settings(SettingsTab::Git);
        }
        let dismissed = queued_errors.len() - errors.len();
        if dismissed > 0 {
            let mut state = self.state.lock().unwrap();
            // Only dismiss the displayed prefix; newly arriving failures remain pending.
            for error in queued_errors.iter().take(dismissed) {
                if state.errors.front() != Some(error) {
                    break;
                }
                state.errors.pop_front();
            }
        }
    }

    fn show_welcome(&mut self, root_ui: &mut egui::Ui) {
        use crate::ui::{kit, motion};

        let context = root_ui.ctx().clone();
        let ctx = &context;
        let mut action = None;
        let card = |ui: &mut egui::Ui, id: &str, title: &str, description: &str, large: bool| {
            let inner = ui.scope_builder(
                egui::UiBuilder::new()
                    .id_salt(id)
                    .sense(egui::Sense::click()),
                |ui| {
                    kit::card(ui, |ui| {
                        let background = ui.painter().add(egui::Shape::Noop);
                        if large {
                            ui.set_min_height(44.0);
                        }
                        ui.add(
                            egui::Label::new(
                                RichText::new(title)
                                    .strong()
                                    .size(if large { 15.0 } else { 14.0 })
                                    .color(theme::text_primary()),
                            )
                            .wrap()
                            .selectable(false),
                        );
                        ui.add(
                            egui::Label::new(
                                RichText::new(description)
                                    .small()
                                    .color(theme::text_muted()),
                            )
                            .wrap()
                            .selectable(false),
                        );
                        background
                    })
                },
            );
            let response = inner
                .response
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            let hover = motion::hover(ui, &response);
            ui.painter().set(
                inner.inner,
                egui::Shape::rect_filled(
                    response.rect.shrink(1.0),
                    kit::CARD_RADIUS,
                    motion::mix(Color32::TRANSPARENT, theme::bg_hover(), hover),
                ),
            );
            if response.has_focus() {
                ui.painter().rect_stroke(
                    response.rect.shrink(1.0),
                    kit::CARD_RADIUS,
                    Stroke::new(1.5, theme::accent_teal()),
                    egui::StrokeKind::Inside,
                );
            }
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), title)
            });
            response
        };

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(theme::bg_app()))
            .show(root_ui, |ui| {
                let fade_id = ui.id().with("welcome_fade");
                let first_show = ctx.data_mut(|data| {
                    let seen = data.get_temp::<bool>(fade_id).unwrap_or(false);
                    data.insert_temp(fade_id, true);
                    !seen
                });
                if first_show {
                    motion::progress(ctx, fade_id, false, motion::FAST);
                }
                motion::fade(ui, fade_id, true);
                egui::ScrollArea::vertical()
                    .id_salt("welcome_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add_space(kit::space::XL);
                        let width = (ui.available_width() - 2.0 * kit::space::LG).clamp(0.0, 620.0);
                        let inset = ((ui.available_width() - width) / 2.0).max(0.0);
                        ui.horizontal_top(|ui| {
                            ui.add_space(inset);
                            ui.allocate_ui_with_layout(
                                egui::vec2(width, 0.0),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    ui.set_width(width);
                                    ui.vertical_centered(|ui| {
                                        ui.label(
                                            crate::icons::rich(crate::icons::SOURCE, 40.0)
                                                .color(theme::accent_teal()),
                                        );
                                        kit::page_header(
                                            ui,
                                            "Recorde",
                                            "Explore your history. Review your changes. \
                                             Make your next commit.",
                                        );
                                    });
                                    if card(
                                        ui,
                                        "welcome_open",
                                        "Open a repository on this computer",
                                        "Continue a project you already have in a local folder.",
                                        true,
                                    )
                                    .clicked()
                                    {
                                        if let Some(path) = rfd::FileDialog::new().pick_folder() {
                                            action = Some(ToolbarAction::OpenRepo(path));
                                        }
                                    }
                                    ui.add_space(crate::ui::kit::space::SM);
                                    if card(
                                        ui,
                                        "welcome_clone",
                                        "Clone a repository from GitHub, GitLab or a URL",
                                        "Download a project and its history to this computer.",
                                        true,
                                    )
                                    .clicked()
                                    {
                                        action = Some(ToolbarAction::CloneRepo);
                                    }
                                    ui.add_space(crate::ui::kit::space::SM);
                                    if card(
                                        ui,
                                        "welcome_create",
                                        "Create a new repository",
                                        "Start tracking changes for a new project.",
                                        true,
                                    )
                                    .clicked()
                                    {
                                        action = Some(ToolbarAction::InitRepo);
                                    }
                                    if !self.config.recent_repos.is_empty() {
                                        ui.add_space(kit::space::XL);
                                        kit::group_label(ui, "Pick up where you left off");
                                        ui.add_space(crate::ui::kit::space::SM);
                                        for path in self.config.recent_repos.iter().take(8) {
                                            let name = path
                                                .file_name()
                                                .unwrap_or_default()
                                                .to_string_lossy();
                                            let full_path = path.display().to_string();
                                            if card(ui, &full_path, &name, &full_path, false)
                                                .on_hover_text(&full_path)
                                                .clicked()
                                            {
                                                action =
                                                    Some(ToolbarAction::OpenRepo(path.clone()));
                                            }
                                            ui.add_space(kit::space::XS);
                                        }
                                    }
                                    ui.add_space(kit::space::LG);
                                    if ui.link("New to Git? Read the User Guide").clicked() {
                                        self.help.open_page("User-Guide");
                                    }
                                    ui.add_space(crate::ui::kit::space::SM);
                                    kit::hint(
                                        ui,
                                        "Ctrl/Cmd+K  Command palette     Ctrl/Cmd+B  Switch branch",
                                    );
                                    ui.add_space(kit::space::XL);
                                },
                            );
                        });
                    });
            });
        if let Some(action) = action {
            self.handle_toolbar(ctx, action);
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Mark the pane that receives keyboard shortcuts with an accent line along its
/// top edge. A full outline drew a frame around the whole panel after every click.
fn focus_ring(ui: &egui::Ui) {
    let rect = ui.max_rect();
    ui.painter().rect_filled(
        egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), 2.0)),
        0.0,
        theme::accent_teal(),
    );
}

/// Menu label that adds or removes a repository from favourites.
fn favorite_label(path: &std::path::Path, favorite: bool) -> String {
    let name = path
        .file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy();
    if favorite {
        format!("Remove {name} from favourites")
    } else {
        format!("Add {name} to favourites")
    }
}

/// A repository tab with the same padding whether idle, hovered or active, so
/// hovering never shifts its neighbours. egui's selectable label only adds its
/// frame margin once highlighted.
fn repo_tab(ui: &mut egui::Ui, label: &str, active: bool) -> egui::Response {
    let response = ui
        .scope(|ui| {
            ui.spacing_mut().button_padding = egui::vec2(12.0, 4.0);
            ui.add(
                Button::new(RichText::new(label).size(13.0).color(if active {
                    theme::text_primary()
                } else {
                    theme::text_secondary()
                }))
                .fill(if active {
                    theme::bg_selected()
                } else {
                    Color32::TRANSPARENT
                })
                .stroke(Stroke::NONE)
                .corner_radius(6.0)
                .min_size(egui::vec2(0.0, 26.0)),
            )
        })
        .inner;
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            active,
            label,
        )
    });
    response
}

fn panel_toggle(ui: &mut egui::Ui, visible: &mut bool, label: &str, tip: &str) -> bool {
    use crate::ui::motion;
    // Colours ease between the shown and hidden states instead of snapping.
    let t = motion::progress(
        ui.ctx(),
        egui::Id::new(("panel_toggle", label)),
        *visible,
        motion::FAST,
    );
    // Quiet chrome: shown panels read as primary text on a subtle fill, hidden
    // ones recede; no border or accent competes with the toolbar's actions.
    let color = motion::mix(theme::text_muted(), theme::text_primary(), t);
    let fill = motion::mix(Color32::TRANSPARENT, theme::bg_panel_alt(), t);
    let state = if *visible { "shown" } else { "hidden" };
    let resp = ui
        .add(
            Button::new(RichText::new(label).color(color).size(12.0))
                .fill(fill)
                .stroke(Stroke::NONE)
                .corner_radius(5.0)
                .min_size(egui::Vec2::new(0.0, 24.0)),
        )
        .on_hover_text(format!("{tip} (currently {state})"));
    resp.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            *visible,
            label,
        )
    });
    if resp.clicked() {
        *visible = !*visible;
        true
    } else {
        false
    }
}

fn modal_window(ctx: &Context, title: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
    egui::Window::new(title)
        .id(egui::Id::new(("recorde_modal", title)))
        .collapsible(false)
        // Sized to its content. No outer ScrollArea: dialog footers lay out
        // right-to-left centred on the available height, which a scroll area
        // makes unbounded, pushing the actions out of view. Long lists scroll
        // inside their own dialogs.
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .frame(egui::Frame::window(&ctx.global_style()).inner_margin(egui::Margin::same(16)))
        .show(ctx, |ui| {
            crate::ui::motion::fade(ui, egui::Id::new(("recorde_modal_fade", title)), true);
            ui.set_min_width(320.0);
            ui.set_max_width(560.0);
            add_contents(ui);
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
            .frame(egui::Frame::window(&ctx.global_style()).inner_margin(egui::Margin::same(14)))
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
                            egui::ScrollArea::vertical().id_salt("iss_list").show(
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
                                        ui.scope_builder(egui::UiBuilder::new().max_rect(row), |ui| {
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
                                        .id_salt("iss_body")
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
                    crate::ui::kit::text_field(&mut self.repo_ui.branch_from_issue_input)
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
    crate::ui::kit::primary_button(label)
}

fn danger_btn(label: &str) -> Button<'_> {
    crate::ui::kit::danger_button(label)
}

fn secondary_btn(label: &str) -> Button<'_> {
    crate::ui::kit::secondary_button(label)
}

fn styled_label(text: &str) -> RichText {
    RichText::new(text)
        .color(theme::text_secondary())
        .size(13.0)
}

fn styled_muted(text: &str) -> RichText {
    RichText::new(text).color(theme::text_muted()).size(12.0)
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
            crate::ui::kit::text_field(value).margin(egui::Vec2::new(8.0, 5.0)),
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
        None => crate::profiles::global_config_for_write().map_err(|e| e.message().to_string())?,
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

fn open_repo_in_editor(
    repo_path: Option<std::path::PathBuf>,
    configured_editor: &str,
) -> Result<(), String> {
    let path =
        repo_path.ok_or_else(|| "Open a repository before launching an editor.".to_string())?;
    let editor = crate::external::editor_command(configured_editor)?;
    crate::external::open_editor(&path, &editor)
}

fn open_url(url: &str) -> Result<(), String> {
    crate::external::open_url(url)
}

#[cfg(test)]
mod repo_settings_tests {
    use super::*;
    use crate::git::test_support::TestRepo;

    #[test]
    fn snapshot_builder_reads_remotes_and_repository_settings() {
        let fixture = TestRepo::new();
        fixture.commit("base.txt", b"base\n");
        let repo = fixture.open();
        repo.remote("origin", "https://example.com/upstream.git")
            .unwrap();
        repo.remote("fork", "https://example.com/fork.git").unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("remote.pushDefault", "fork").unwrap();
        config.set_str("recorde.providerRemote", "origin").unwrap();
        config.set_str("branch.main.pushRemote", "origin").unwrap();
        config.set_i64("recorde.recoveryRetentionDays", 90).unwrap();

        let snapshot = RepoSettingsSnapshot::from_repo(&repo).unwrap();
        assert_eq!(snapshot.retention_days, 90);
        assert_eq!(snapshot.provider_remote, "origin");
        assert_eq!(snapshot.push_default, "fork");
        assert_eq!(snapshot.branch.as_deref(), Some("main"));
        assert_eq!(snapshot.branch_push_remote, "origin");
        assert_eq!(snapshot.remotes.len(), 2);
        assert!(snapshot
            .remotes
            .contains(&("origin".into(), "https://example.com/upstream.git".into(),)));
        assert!(snapshot
            .remotes
            .contains(&("fork".into(), "https://example.com/fork.git".into())));
        assert!(snapshot.recovery.is_empty());
    }

    #[test]
    fn snapshot_builder_handles_unborn_and_detached_head() {
        let fixture = TestRepo::new();
        let repo = fixture.open();
        let snapshot = RepoSettingsSnapshot::from_repo(&repo).unwrap();
        assert!(snapshot.branch.is_none());
        assert!(snapshot.branch_push_remote.is_empty());
        assert!(snapshot.remotes.is_empty());
        assert_eq!(snapshot.retention_days, 0);
        let head = fixture.commit("base.txt", b"base\n");
        repo.set_head_detached(head).unwrap();
        assert!(RepoSettingsSnapshot::from_repo(&repo)
            .unwrap()
            .branch
            .is_none());
    }
}

#[cfg(test)]
mod repo_tab_tests {
    use super::repo_tab;

    /// Hovering a repository tab must change only its fill, never its size, so
    /// neighbouring tabs do not shift.
    #[test]
    fn repository_tabs_keep_their_size_when_hovered() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::icons::font_definitions());
        let draw = |events: Vec<egui::Event>| {
            ctx.begin_pass(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 100.0),
                )),
                events,
                ..Default::default()
            });
            let mut ui = egui::Ui::new(
                ctx.clone(),
                egui::Id::new("repo_tab_test"),
                egui::UiBuilder::new().max_rect(ctx.content_rect()),
            );
            let rects = ui
                .horizontal(|ui| {
                    [
                        repo_tab(ui, "repo", false).rect,
                        repo_tab(ui, "workspace-repo", true).rect,
                    ]
                })
                .inner;
            ctx.end_pass().textures_delta.clear();
            rects
        };
        // Fonts set on the context apply from the next frame, so warm up first.
        draw(vec![]);
        let idle = draw(vec![egui::Event::PointerGone]);
        // A second hovered frame lets hover state settle.
        draw(vec![egui::Event::PointerMoved(idle[0].center())]);
        let hovered = draw(vec![egui::Event::PointerMoved(idle[0].center())]);
        assert_eq!(
            idle, hovered,
            "Hover must not resize or move repository tabs"
        );
        // 12 pixels of padding each side keeps the label clear of the pill edge.
        assert!(idle[0].width() >= 24.0 + 28.0);
    }
}
