use egui::{Color32, Context, RichText, Stroke, Visuals};

use crate::git::{branches, operations::load_repo, remote};
use crate::state::{ModalKind, new_shared_state, SharedState};
use crate::theme;
use crate::ui::{
    commit_graph::CommitGraphView,
    detail_panel::DetailPanel,
    diff_view::DiffView,
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
    // Ephemeral state for the Create Branch modal form
    create_branch_name: String,
    create_branch_checkout: bool,
}

impl RecordeApp {
    pub fn new(cc: &eframe::CreationContext<'_>, initial_repo: Option<std::path::PathBuf>) -> Self {
        apply_theme(&cc.egui_ctx);

        let state = new_shared_state();

        if let Some(path) = initial_repo {
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
            state,
            create_branch_name: String::new(),
            create_branch_checkout: true,
        }
    }

    fn handle_toolbar_action(&mut self, action: ToolbarAction) {
        match action {
            ToolbarAction::OpenRepo(path) => load_repo(path, self.state.clone()),
            ToolbarAction::Stash => branches::stash_save(self.state.clone()),
            ToolbarAction::CreateBranch => {
                self.state.lock().unwrap().modal = ModalKind::CreateBranch { from_oid: None };
            }
            ToolbarAction::Fetch => remote::fetch_all(self.state.clone()),
            ToolbarAction::Pull  => remote::pull_branch(self.state.clone()),
            ToolbarAction::Push  => remote::push_branch(self.state.clone()),
        }
    }

    fn handle_sidebar_action(&mut self, action: SidebarAction) {
        match action {
            SidebarAction::CheckoutBranch(name) => {
                branches::checkout_branch(&name, self.state.clone());
            }
            SidebarAction::OpenCreateBranch { from_oid } => {
                self.state.lock().unwrap().modal = ModalKind::CreateBranch { from_oid };
            }
            SidebarAction::DeleteBranch(name) => {
                self.state.lock().unwrap().modal =
                    ModalKind::ConfirmDeleteBranch { branch_name: name };
            }
            SidebarAction::StashApply(i) => branches::stash_apply(i, self.state.clone()),
            SidebarAction::StashPop(i) => branches::stash_pop(i, self.state.clone()),
            SidebarAction::StashDrop(i) => branches::stash_drop(i, self.state.clone()),
        }
    }

    fn render_modals(&mut self, ctx: &Context) {
        let modal_kind = self.state.lock().unwrap().modal.clone();

        match modal_kind {
            ModalKind::None => {}

            ModalKind::CreateBranch { from_oid } => {
                egui::Window::new("Create Branch")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .frame(egui::Frame::window(&ctx.style()).inner_margin(egui::Margin::same(16.0)))
                    .show(ctx, |ui| {
                    ui.set_min_width(320.0);
                    ui.add_space(4.0);

                    ui.label(
                        RichText::new("Create Branch")
                            .color(theme::TEXT_PRIMARY)
                            .size(15.0)
                            .strong(),
                    );
                    ui.add_space(12.0);

                    ui.label(RichText::new("Branch name").color(theme::TEXT_SECONDARY).size(12.0));
                    ui.add_space(4.0);
                    let name_resp = ui.add(
                        egui::TextEdit::singleline(&mut self.create_branch_name)
                            .desired_width(f32::INFINITY)
                            .hint_text("e.g. feat/my-feature"),
                    );
                    name_resp.request_focus();

                    if let Some(ref oid) = from_oid {
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(format!("From commit  {}", &oid[..7.min(oid.len())]))
                                .color(theme::TEXT_MUTED)
                                .size(11.0),
                        );
                    }

                    ui.add_space(8.0);
                    ui.checkbox(
                        &mut self.create_branch_checkout,
                        RichText::new("Checkout immediately").color(theme::TEXT_SECONDARY).size(12.0),
                    );

                    ui.add_space(16.0);
                    ui.horizontal(|ui| {
                        let can_create = !self.create_branch_name.trim().is_empty();
                        let confirm = ui
                            .add_enabled(
                                can_create,
                                egui::Button::new(
                                    RichText::new("Create Branch").color(Color32::WHITE),
                                )
                                .fill(theme::ACCENT_BLUE),
                            )
                            .clicked();

                        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                        let cancel = ui.button(RichText::new("Cancel").color(theme::TEXT_SECONDARY)).clicked();
                        let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));

                        if (confirm || enter) && can_create {
                            let name = self.create_branch_name.trim().to_string();
                            let checkout = self.create_branch_checkout;
                            branches::create_branch(
                                &name,
                                from_oid.as_deref(),
                                checkout,
                                self.state.clone(),
                            );
                            self.state.lock().unwrap().modal = ModalKind::None;
                            self.create_branch_name.clear();
                        } else if cancel || escape {
                            self.state.lock().unwrap().modal = ModalKind::None;
                            self.create_branch_name.clear();
                        }
                    });
                    ui.add_space(4.0);
                });
            }

            ModalKind::ConfirmDeleteBranch { branch_name } => {
                egui::Window::new("Delete Branch")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .frame(egui::Frame::window(&ctx.style()).inner_margin(egui::Margin::same(16.0)))
                    .show(ctx, |ui| {
                    ui.set_min_width(300.0);
                    ui.add_space(4.0);

                    ui.label(
                        RichText::new("Delete Branch")
                            .color(theme::ACCENT_RED)
                            .size(15.0)
                            .strong(),
                    );
                    ui.add_space(12.0);
                    ui.label(
                        RichText::new(format!("Delete \"{}\"?", branch_name))
                            .color(theme::TEXT_PRIMARY)
                            .size(13.0),
                    );
                    ui.label(
                        RichText::new("Unmerged commits on this branch will be lost.")
                            .color(theme::TEXT_MUTED)
                            .size(11.5),
                    );
                    ui.add_space(16.0);
                    ui.horizontal(|ui| {
                        let confirmed = ui
                            .add(
                                egui::Button::new(RichText::new("Delete").color(Color32::WHITE))
                                    .fill(theme::ACCENT_RED),
                            )
                            .clicked();
                        let cancelled = ui.button(RichText::new("Cancel").color(theme::TEXT_SECONDARY)).clicked();
                        let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));

                        if confirmed {
                            branches::delete_branch(&branch_name, self.state.clone());
                            self.state.lock().unwrap().modal = ModalKind::None;
                        } else if cancelled || escape {
                            self.state.lock().unwrap().modal = ModalKind::None;
                        }
                    });
                    ui.add_space(4.0);
                });
            }
        }
    }
}

impl eframe::App for RecordeApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        // Modals rendered first (float above everything)
        self.render_modals(ctx);

        self.status_bar.show(ctx);

        let toolbar_action = self.toolbar.show(ctx);
        if let Some(action) = toolbar_action {
            self.handle_toolbar_action(action);
        }

        egui::SidePanel::left("sidebar")
            .resizable(true)
            .default_width(220.0)
            .min_width(150.0)
            .frame(egui::Frame::none())
            .show(ctx, |ui| {
                let action = self.sidebar.show(ui);
                if let Some(a) = action {
                    self.handle_sidebar_action(a);
                }
            });

        egui::SidePanel::right("detail_panel")
            .resizable(true)
            .default_width(320.0)
            .min_width(200.0)
            .frame(egui::Frame::none())
            .show(ctx, |ui| {
                // Auto-switch: commit selected → Details tab.
                let has_commit = self.state.lock().unwrap().selected_commit.is_some();
                let has_dirty  = self.state.lock().unwrap().dirty_count > 0;
                if has_commit && self.right_tab == RightTab::Changes {
                    self.right_tab = RightTab::Details;
                }

                // Tab bar
                let tab_height = 32.0;
                let tab_rect = egui::Rect::from_min_size(
                    ui.min_rect().min,
                    egui::Vec2::new(ui.available_width(), tab_height),
                );
                ui.painter().rect_filled(tab_rect, 0.0, theme::BG_ELEVATED);
                ui.painter().line_segment(
                    [
                        egui::Pos2::new(tab_rect.min.x, tab_rect.max.y),
                        egui::Pos2::new(tab_rect.max.x, tab_rect.max.y),
                    ],
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
                            let text = egui::RichText::new(*label)
                                .size(12.0)
                                .color(if selected { theme::TEXT_PRIMARY } else { theme::TEXT_MUTED });
                            let btn = ui.add(
                                egui::Button::new(text)
                                    .fill(if selected { theme::BG_SURFACE } else { Color32::TRANSPARENT })
                                    .stroke(egui::Stroke::new(
                                        if selected { 1.0 } else { 0.0 },
                                        theme::BORDER,
                                    )),
                            );
                            if btn.clicked() {
                                self.right_tab = *tab;
                            }
                        }
                    });
                });

                // Panel content below tab bar
                let content_rect = egui::Rect::from_min_max(
                    egui::Pos2::new(ui.min_rect().min.x, ui.min_rect().min.y + tab_height),
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

fn apply_theme(ctx: &Context) {
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

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::Vec2::new(6.0, 3.0);
    style.spacing.window_margin = egui::Margin::same(0.0);
    style.spacing.button_padding = egui::Vec2::new(8.0, 4.0);
    ctx.set_style(style);
}
