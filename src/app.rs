use egui::{Color32, Context, RichText, Stroke, Visuals};

use crate::config::Config;
use crate::git::{branches, operations::load_repo, remote};
use crate::state::{ModalKind, new_shared_state, SharedState};
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
enum RightTab { Changes, Details }

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
    // Interactive rebase
    rebase_modal: Option<RebaseModal>,
}

impl RecordeApp {
    pub fn new(cc: &eframe::CreationContext<'_>, initial_repo: Option<std::path::PathBuf>) -> Self {
        let config = Config::load();
        let dark_theme = config.dark_theme;
        apply_theme(&cc.egui_ctx, dark_theme);

        let state = new_shared_state();

        // Open CLI arg or last-used repo
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
            rebase_modal: None,
        }
    }

    fn handle_toolbar_action(&mut self, ctx: &Context, action: ToolbarAction) {
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
                apply_theme(ctx, self.dark_theme);
            }
            ToolbarAction::ToggleSearch => {
                let mut s = self.state.lock().unwrap();
                s.search_active = !s.search_active;
                if !s.search_active { s.search_query.clear(); }
            }
        }
    }

    fn handle_sidebar_action(&mut self, action: SidebarAction) {
        match action {
            SidebarAction::CheckoutBranch(name) =>
                branches::checkout_branch(&name, self.state.clone()),
            SidebarAction::OpenCreateBranch { from_oid } =>
                self.state.lock().unwrap().modal = ModalKind::CreateBranch { from_oid },
            SidebarAction::DeleteBranch(name) =>
                self.state.lock().unwrap().modal = ModalKind::ConfirmDeleteBranch { branch_name: name },
            SidebarAction::CreateTag(_) =>
                self.state.lock().unwrap().modal = ModalKind::CreateTag { from_oid: None },
            SidebarAction::DeleteTag(name) =>
                self.state.lock().unwrap().modal = ModalKind::ConfirmDeleteTag { tag_name: name },
            SidebarAction::StashApply(i) => branches::stash_apply(i, self.state.clone()),
            SidebarAction::StashPop(i)   => branches::stash_pop(i, self.state.clone()),
            SidebarAction::StashDrop(i)  => branches::stash_drop(i, self.state.clone()),
        }
    }

    /// Build and open the rebase modal for the given upstream commit OID.
    fn open_rebase_modal(&mut self, upstream_oid: String) {
        let repo_path = self.state.lock().unwrap().repo_path.clone();
        let Some(path) = repo_path else { return };

        // Get the upstream commit's summary for display
        let upstream_summary = (|| -> Option<String> {
            let repo = git2::Repository::open(&path).ok()?;
            let oid  = git2::Oid::from_str(&upstream_oid).ok()?;
            let commit = repo.find_commit(oid).ok()?;
            commit.summary().map(str::to_owned)
        })()
        .unwrap_or_default();

        let entries = crate::git::rebase::load_rebase_entries(&path, &upstream_oid);
        match entries {
            Some(e) if !e.is_empty() => {
                self.rebase_modal = Some(RebaseModal::new(upstream_oid, upstream_summary, e));
            }
            _ => {
                self.state.lock().unwrap().error =
                    Some("No commits to rebase between this commit and HEAD.".into());
            }
        }
    }

    fn render_modals(&mut self, ctx: &Context) {
        let modal_kind = self.state.lock().unwrap().modal.clone();
        match modal_kind {
            ModalKind::None => {}

            ModalKind::CreateBranch { from_oid } => {
                modal_window(ctx, "Create Branch", |ui| {
                    ui.label(RichText::new("Branch name").color(theme::TEXT_SECONDARY).size(12.0));
                    ui.add_space(4.0);
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut self.create_branch_name)
                            .desired_width(f32::INFINITY)
                            .hint_text("e.g. feat/my-feature"),
                    );
                    resp.request_focus();
                    if let Some(ref oid) = from_oid {
                        ui.add_space(4.0);
                        ui.label(RichText::new(format!("From  {}", &oid[..7.min(oid.len())])).color(theme::TEXT_MUTED).size(11.0));
                    }
                    ui.add_space(8.0);
                    ui.checkbox(&mut self.create_branch_checkout,
                        RichText::new("Checkout immediately").color(theme::TEXT_SECONDARY).size(12.0));
                    ui.add_space(16.0);
                    let can = !self.create_branch_name.trim().is_empty();
                    ui.horizontal(|ui| {
                        let ok = ui.add_enabled(can, egui::Button::new(
                            RichText::new("Create Branch").color(Color32::WHITE)).fill(theme::ACCENT_BLUE)).clicked()
                            || (can && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                        let cancel = ui.button(RichText::new("Cancel").color(theme::TEXT_SECONDARY)).clicked()
                            || ui.input(|i| i.key_pressed(egui::Key::Escape));
                        if ok {
                            let name = self.create_branch_name.trim().to_string();
                            branches::create_branch(&name, from_oid.as_deref(), self.create_branch_checkout, self.state.clone());
                            self.state.lock().unwrap().modal = ModalKind::None;
                            self.create_branch_name.clear();
                        } else if cancel {
                            self.state.lock().unwrap().modal = ModalKind::None;
                            self.create_branch_name.clear();
                        }
                    });
                });
            }

            ModalKind::ConfirmDeleteBranch { branch_name } => {
                modal_window(ctx, "Delete Branch", |ui| {
                    ui.label(RichText::new(format!("Delete \"{}\"?", branch_name)).color(theme::TEXT_PRIMARY).size(13.0));
                    ui.label(RichText::new("Unmerged commits will be lost.").color(theme::TEXT_MUTED).size(11.5));
                    ui.add_space(16.0);
                    ui.horizontal(|ui| {
                        let confirmed = ui.add(egui::Button::new(RichText::new("Delete").color(Color32::WHITE)).fill(theme::ACCENT_RED)).clicked();
                        let cancel = ui.button(RichText::new("Cancel").color(theme::TEXT_SECONDARY)).clicked()
                            || ui.input(|i| i.key_pressed(egui::Key::Escape));
                        if confirmed {
                            branches::delete_branch(&branch_name, self.state.clone());
                            self.state.lock().unwrap().modal = ModalKind::None;
                        } else if cancel {
                            self.state.lock().unwrap().modal = ModalKind::None;
                        }
                    });
                });
            }

            ModalKind::CreateTag { from_oid } => {
                modal_window(ctx, "Create Tag", |ui| {
                    ui.label(RichText::new("Tag name").color(theme::TEXT_SECONDARY).size(12.0));
                    ui.add_space(4.0);
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut self.create_tag_name)
                            .desired_width(f32::INFINITY)
                            .hint_text("e.g. v1.0.0"),
                    );
                    resp.request_focus();
                    if let Some(ref oid) = from_oid {
                        ui.add_space(4.0);
                        ui.label(RichText::new(format!("At commit  {}", &oid[..7.min(oid.len())])).color(theme::TEXT_MUTED).size(11.0));
                    }
                    ui.add_space(16.0);
                    let can = !self.create_tag_name.trim().is_empty();
                    ui.horizontal(|ui| {
                        let ok = ui.add_enabled(can, egui::Button::new(
                            RichText::new("Create Tag").color(Color32::WHITE)).fill(theme::ACCENT_BLUE)).clicked()
                            || (can && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                        let cancel = ui.button(RichText::new("Cancel").color(theme::TEXT_SECONDARY)).clicked()
                            || ui.input(|i| i.key_pressed(egui::Key::Escape));
                        if ok {
                            let name = self.create_tag_name.trim().to_string();
                            branches::create_tag(&name, from_oid.as_deref(), self.state.clone());
                            self.state.lock().unwrap().modal = ModalKind::None;
                            self.create_tag_name.clear();
                        } else if cancel {
                            self.state.lock().unwrap().modal = ModalKind::None;
                            self.create_tag_name.clear();
                        }
                    });
                });
            }

            ModalKind::ConfirmDeleteTag { tag_name } => {
                modal_window(ctx, "Delete Tag", |ui| {
                    ui.label(RichText::new(format!("Delete tag \"{}\"?", tag_name)).color(theme::TEXT_PRIMARY).size(13.0));
                    ui.add_space(16.0);
                    ui.horizontal(|ui| {
                        let confirmed = ui.add(egui::Button::new(RichText::new("Delete").color(Color32::WHITE)).fill(theme::ACCENT_RED)).clicked();
                        let cancel = ui.button(RichText::new("Cancel").color(theme::TEXT_SECONDARY)).clicked()
                            || ui.input(|i| i.key_pressed(egui::Key::Escape));
                        if confirmed {
                            branches::delete_tag(&tag_name, self.state.clone());
                            self.state.lock().unwrap().modal = ModalKind::None;
                        } else if cancel {
                            self.state.lock().unwrap().modal = ModalKind::None;
                        }
                    });
                });
            }
        }
    }
}

impl eframe::App for RecordeApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        // Keep debug overlays off every frame — egui can reset them internally.
        ctx.style_mut(kill_debug_overlays);
        ctx.options_mut(|o| o.warn_on_id_clash = false);

        // Check if the graph context menu triggered a rebase open
        let pending = self.state.lock().unwrap().pending_open_rebase.take();
        if let Some(oid) = pending {
            self.open_rebase_modal(oid);
        }

        self.render_modals(ctx);

        // Render the interactive rebase modal (separate from ModalKind to keep mutable form state local)
        if self.rebase_modal.is_some() {
            let mut close = false;
            let mut execute_entries: Option<(String, Vec<crate::state::RebaseEntry>)> = None;

            egui::Window::new("Interactive Rebase")
                .collapsible(false)
                .resizable(true)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .frame(egui::Frame::window(&ctx.style()).inner_margin(egui::Margin::same(16.0)))
                .show(ctx, |ui| {
                    if let Some(ref mut modal) = self.rebase_modal {
                        match modal.show(ui) {
                            Some(RebaseModalResult::Execute) => {
                                execute_entries = Some((
                                    modal.upstream_oid.clone(),
                                    modal.entries.clone(),
                                ));
                                close = true;
                            }
                            Some(RebaseModalResult::Cancel) => close = true,
                            None => {}
                        }
                    }
                });

            if close {
                self.rebase_modal = None;
            }
            if let Some((upstream, entries)) = execute_entries {
                crate::git::rebase::start_rebase(&upstream, entries, self.state.clone());
            }
        }

        self.status_bar.show(ctx);

        let toolbar_action = self.toolbar.show(ctx, &self.config.recent_repos, self.dark_theme);
        if let Some(action) = toolbar_action {
            self.handle_toolbar_action(ctx, action);
        }

        egui::SidePanel::left("sidebar")
            .resizable(true)
            .default_width(220.0)
            .min_width(150.0)
            .frame(egui::Frame::none())
            .show(ctx, |ui| {
                let action = self.sidebar.show(ui);
                if let Some(a) = action { self.handle_sidebar_action(a); }
            });

        egui::SidePanel::right("detail_panel")
            .resizable(true)
            .default_width(320.0)
            .min_width(200.0)
            .frame(egui::Frame::none())
            .show(ctx, |ui| {
                let has_commit = self.state.lock().unwrap().selected_commit.is_some();
                let has_dirty  = self.state.lock().unwrap().dirty_count > 0;
                if has_commit && self.right_tab == RightTab::Changes {
                    self.right_tab = RightTab::Details;
                }

                let tab_h = 32.0;
                let tab_rect = egui::Rect::from_min_size(
                    ui.min_rect().min,
                    egui::Vec2::new(ui.available_width(), tab_h),
                );
                ui.painter().rect_filled(tab_rect, 0.0, theme::BG_ELEVATED);
                ui.painter().line_segment(
                    [egui::Pos2::new(tab_rect.min.x, tab_rect.max.y),
                     egui::Pos2::new(tab_rect.max.x, tab_rect.max.y)],
                    egui::Stroke::new(1.0, theme::BORDER),
                );
                ui.allocate_ui_at_rect(tab_rect, |ui| {
                    ui.horizontal_centered(|ui| {
                        ui.add_space(8.0);
                        for (tab, label) in &[
                            (RightTab::Changes, if has_dirty { "  Changes ●  " } else { "  Changes  " }),
                            (RightTab::Details, "  Details  "),
                        ] {
                            let selected = self.right_tab == *tab;
                            let btn = ui.add(
                                egui::Button::new(RichText::new(*label).size(12.0).color(
                                    if selected { theme::TEXT_PRIMARY } else { theme::TEXT_MUTED },
                                ))
                                .fill(if selected { theme::BG_SURFACE } else { Color32::TRANSPARENT })
                                .stroke(egui::Stroke::new(if selected { 1.0 } else { 0.0 }, theme::BORDER)),
                            );
                            if btn.clicked() { self.right_tab = *tab; }
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

        let show_diff = self.state.lock().unwrap().selected_file.is_some();
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(theme::BG_BASE))
            .show(ctx, |ui| {
                if show_diff {
                    self.diff_view.show(ui);
                } else {
                    self.commit_graph_view.show(ui);
                }
            });
    }
}

// ── Shared modal helper ───────────────────────────────────────────────────────

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

// ── Theme ─────────────────────────────────────────────────────────────────────

fn apply_theme(ctx: &Context, dark: bool) {
    if !dark {
        let mut v = Visuals::light();
        v.window_stroke = Stroke::new(1.0, Color32::from_gray(210));
        v.popup_shadow = egui::Shadow::NONE;
        v.window_shadow = egui::Shadow::NONE;
        ctx.set_visuals(v);
    } else {
        let mut v = Visuals::dark();
        v.panel_fill = theme::BG_BASE;
        v.window_fill = theme::BG_SURFACE;
        v.faint_bg_color = theme::BG_SURFACE;
        v.extreme_bg_color = Color32::from_rgb(0x12, 0x14, 0x18);
        v.override_text_color = Some(theme::TEXT_PRIMARY);
        v.widgets.noninteractive.bg_fill = theme::BG_SURFACE;
        v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, theme::TEXT_SECONDARY);
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, theme::BORDER);
        v.widgets.inactive.bg_fill = theme::BG_ELEVATED;
        v.widgets.inactive.fg_stroke = Stroke::new(1.0, theme::TEXT_SECONDARY);
        v.widgets.inactive.bg_stroke = Stroke::new(1.0, theme::BORDER);
        v.widgets.hovered.bg_fill = Color32::from_rgb(0x35, 0x3A, 0x4A);
        v.widgets.hovered.fg_stroke = Stroke::new(1.0, theme::TEXT_PRIMARY);
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, theme::ACCENT_BLUE);
        v.widgets.active.bg_fill = theme::ACCENT_BLUE;
        v.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
        v.selection.bg_fill = Color32::from_rgba_premultiplied(0x4D, 0x88, 0xFF, 50);
        v.selection.stroke = Stroke::new(1.0, theme::ACCENT_BLUE);
        v.window_stroke = Stroke::new(1.0, theme::BORDER);
        v.popup_shadow = egui::Shadow::NONE;
        v.window_shadow = egui::Shadow::NONE;
        ctx.set_visuals(v);
    }

    ctx.style_mut(|style| {
        if dark {
            style.spacing.item_spacing = egui::Vec2::new(6.0, 3.0);
            style.spacing.window_margin = egui::Margin::same(0.0);
            style.spacing.button_padding = egui::Vec2::new(8.0, 4.0);
        } else {
            style.spacing.item_spacing = egui::Vec2::new(6.0, 3.0);
            style.spacing.button_padding = egui::Vec2::new(8.0, 4.0);
        }
        kill_debug_overlays(style);
    });
}

/// Zero out every egui debug visualisation flag.
/// Called both at startup and on every frame to prevent any internal reset.
fn kill_debug_overlays(style: &mut egui::Style) {
    style.debug.show_expand_width     = false;
    style.debug.show_expand_height    = false;
    style.debug.show_resize           = false;
    style.debug.show_interactive_widgets = false;
    style.debug.show_widget_hits      = false;
    // Fields that only exist in debug builds
    #[cfg(debug_assertions)]
    {
        style.debug.debug_on_hover                    = false;
        style.debug.debug_on_hover_with_all_modifiers = false;
        style.debug.hover_shows_next                  = false;
    }
}
