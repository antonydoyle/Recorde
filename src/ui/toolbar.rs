use std::path::{Path, PathBuf};

use egui::{Context, RichText, Stroke};

use crate::icons;
use crate::state::SharedState;
use crate::theme;
use crate::ui::widgets;

pub enum ToolbarAction {
    OpenRepo(PathBuf),
    CloneRepo,
    InitRepo,
    RemoveFromRecent(PathBuf),
    OpenTerminal,
    RevealInFolder,
    CopyRepoPath,
    Stash,
    CreateBranch,
    Fetch,
    Pull,
    PullWithRebase,
    PullFfOnly,
    Push,
    PushWithLease,
    PushSetUpstream,
    MergeBranch,
    RebaseOnto,
    ToggleTheme,
    ToggleSearch,
    OpenCommandPalette,
    OpenBranchSwitcher,
    ToggleAllBranches,
    OpenUserMenu,
    OpenReflog,
    OpenBisect,
}

pub struct Toolbar {
    state: SharedState,
}

impl Toolbar {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    pub fn show(
        &mut self,
        ctx: &Context,
        recent_repos: &[PathBuf],
        dark_theme: bool,
        search_active: bool,
        show_all_branches: bool,
    ) -> Option<ToolbarAction> {
        let mut action = None;

        egui::TopBottomPanel::top("toolbar")
            .exact_height(54.0)
            .frame(
                egui::Frame::none()
                    .fill(theme::bg_toolbar())
                    .inner_margin(egui::Margin::symmetric(12.0, 0.0)),
            )
            .show(ctx, |ui| {
                // Bottom separator
                let r = ui.max_rect();
                ui.painter().line_segment(
                    [r.left_bottom(), r.right_bottom()],
                    Stroke::new(1.0, theme::border()),
                );

                ui.horizontal_centered(|ui| {
                    let (has_repo, dirty, op_label) = {
                        let s = self.state.lock().unwrap();
                        (s.repo_path.is_some(), s.dirty_count > 0, s.remote_op.clone())
                    };
                    let busy = op_label.is_some();

                    // ── Repo name pill ───────────────────────────────────
                    {
                        let (repo_path, name) = {
                            let s = self.state.lock().unwrap();
                            let rp = s.repo_path.clone();
                            let n = rp.as_ref()
                                .and_then(|p| p.file_name())
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_else(|| "No repo".into());
                            (rp, n)
                        };

                        let btn = widgets::toolbar_btn_icon(ui, icons::FOLDER_OPEN, &name, true);
                        if btn.clicked() {
                            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                                action = Some(ToolbarAction::OpenRepo(path));
                            }
                        }
                        // Right-click on the repo pill
                        btn.context_menu(|ui| {
                            ui.set_min_width(200.0);
                            if let Some(ref rp) = repo_path {
                                if ui.button("Copy Path").clicked() {
                                    action = Some(ToolbarAction::CopyRepoPath);
                                    ui.close_menu();
                                }
                                if ui.button("Open Terminal").clicked() {
                                    action = Some(ToolbarAction::OpenTerminal);
                                    ui.close_menu();
                                }
                                if ui.button("Reveal in File Manager").clicked() {
                                    action = Some(ToolbarAction::RevealInFolder);
                                    ui.close_menu();
                                }
                                ui.separator();
                            }
                            if ui.button("Clone Repository…").clicked() {
                                action = Some(ToolbarAction::CloneRepo);
                                ui.close_menu();
                            }
                            if ui.button("Initialize Repository…").clicked() {
                                action = Some(ToolbarAction::InitRepo);
                                ui.close_menu();
                            }
                        });

                        // Recent repos drop-down
                        if !recent_repos.is_empty() {
                            let arrow = ui.add(
                                egui::Button::new(RichText::new("▾").color(theme::text_muted()).size(11.0))
                                    .fill(theme::bg_panel())
                                    .stroke(Stroke::new(1.0, theme::border()))
                                    .min_size(egui::Vec2::new(20.0, 0.0))
                                    .rounding(6.0),
                            );
                            arrow.context_menu(|ui| {
                                ui.set_min_width(280.0);
                                ui.label(RichText::new("Recent").color(theme::text_muted()).size(10.5));
                                ui.separator();
                                let mut remove: Option<PathBuf> = None;
                                for p in recent_repos {
                                    let n = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                                    ui.horizontal(|ui| {
                                        if ui.add(
                                            egui::Button::new(RichText::new(&n).size(12.5))
                                                .fill(egui::Color32::TRANSPARENT)
                                                .stroke(egui::Stroke::NONE)
                                                .min_size(egui::Vec2::new(220.0, 0.0)),
                                        ).clicked() {
                                            action = Some(ToolbarAction::OpenRepo(p.clone()));
                                            ui.close_menu();
                                        }
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            if ui.add(
                                                egui::Button::new(RichText::new("×").color(theme::text_muted()).size(12.0))
                                                    .fill(egui::Color32::TRANSPARENT)
                                                    .stroke(egui::Stroke::NONE),
                                            ).on_hover_text("Remove from list").clicked() {
                                                remove = Some(p.clone());
                                                ui.close_menu();
                                            }
                                        });
                                    });
                                }
                                if let Some(p) = remove {
                                    action = Some(ToolbarAction::RemoveFromRecent(p));
                                }
                            });
                        }
                    }

                    ui.add_space(6.0);
                    vsep(ui);
                    ui.add_space(6.0);

                    // ── Git actions ──────────────────────────────────────
                    if widgets::toolbar_btn_icon(ui, icons::DOWNLOAD, "Fetch", has_repo && !busy)
                        .on_hover_text("Fetch all remotes  (Ctrl+Shift+F)").clicked()
                    { action = Some(ToolbarAction::Fetch); }

                    ui.add_space(3.0);

                    {
                        let pull_btn = widgets::toolbar_btn_icon(ui, icons::SWAP_VERT, "Pull", has_repo && !busy)
                            .on_hover_text("Fetch + fast-forward  (Ctrl+Shift+P)");
                        if pull_btn.clicked() { action = Some(ToolbarAction::Pull); }
                        pull_btn.context_menu(|ui| {
                            ui.set_min_width(190.0);
                            if ui.button("Pull (fast-forward)").clicked() {
                                action = Some(ToolbarAction::Pull); ui.close_menu();
                            }
                            if ui.button("Pull with Rebase").clicked() {
                                action = Some(ToolbarAction::PullWithRebase); ui.close_menu();
                            }
                            if ui.button("Pull (ff-only)").clicked() {
                                action = Some(ToolbarAction::PullFfOnly); ui.close_menu();
                            }
                            ui.separator();
                            if ui.button("Merge Branch…").clicked() {
                                action = Some(ToolbarAction::MergeBranch); ui.close_menu();
                            }
                            if ui.button("Rebase Onto…").clicked() {
                                action = Some(ToolbarAction::RebaseOnto); ui.close_menu();
                            }
                        });
                    }

                    ui.add_space(3.0);

                    {
                        let push_btn = widgets::toolbar_btn_icon(ui, icons::UPLOAD, "Push", has_repo && !busy)
                            .on_hover_text("Push current branch  (Ctrl+Shift+U)");
                        if push_btn.clicked() { action = Some(ToolbarAction::Push); }
                        push_btn.context_menu(|ui| {
                            ui.set_min_width(200.0);
                            if ui.button("Push").clicked() {
                                action = Some(ToolbarAction::Push); ui.close_menu();
                            }
                            if ui.button("Force Push with Lease").clicked() {
                                action = Some(ToolbarAction::PushWithLease); ui.close_menu();
                            }
                            if ui.button("Push and Set Upstream").clicked() {
                                action = Some(ToolbarAction::PushSetUpstream); ui.close_menu();
                            }
                        });
                    }

                    ui.add_space(6.0);
                    vsep(ui);
                    ui.add_space(6.0);

                    if widgets::toolbar_btn_icon(ui, icons::HUB, "New Branch", has_repo && !busy)
                        .on_hover_text("Create a new branch").clicked()
                    { action = Some(ToolbarAction::CreateBranch); }

                    ui.add_space(3.0);

                    if widgets::toolbar_btn_icon(ui, icons::HISTORY, "Stash", has_repo && dirty && !busy)
                        .on_hover_text("Stash uncommitted changes").clicked()
                    { action = Some(ToolbarAction::Stash); }

                    ui.add_space(6.0);
                    vsep(ui);
                    ui.add_space(6.0);

                    // Branch filter toggle
                    let (filter_icon, filter_label) = if show_all_branches {
                        (icons::SOURCE, "All Branches")
                    } else {
                        (icons::COMMIT, "This Branch")
                    };
                    let filter_color = if show_all_branches { theme::ACCENT_BLUE } else { theme::text_secondary() };
                    let mut fj = egui::text::LayoutJob::default();
                    fj.append(&filter_icon.to_string(), 0.0,
                        egui::TextFormat { font_id: icons::font(15.0), color: filter_color, line_height: Some(16.0), valign: egui::Align::Center, ..Default::default() });
                    fj.append(&format!("  {filter_label}"), 0.0,
                        egui::TextFormat { font_id: egui::FontId::proportional(12.5), color: filter_color, line_height: Some(16.0), valign: egui::Align::TOP, ..Default::default() });
                    if ui.add(
                        egui::Button::new(fj)
                            .fill(theme::bg_panel())
                            .stroke(egui::Stroke::new(1.0, if show_all_branches { theme::ACCENT_BLUE.gamma_multiply(0.4) } else { theme::border() }))
                            .rounding(6.0),
                    ).on_hover_text("Toggle: show all branches or current branch only").clicked() {
                        action = Some(ToolbarAction::ToggleAllBranches);
                    }

                    // ── Right side ───────────────────────────────────────
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(8.0);

                        // Avatar — initials from git config user.name
                        let initials = {
                            let s = self.state.lock().unwrap();
                            s.git_user_initials.clone()
                        };
                        let (av_rect, av_resp) = ui.allocate_exact_size(
                            egui::Vec2::splat(28.0), egui::Sense::click(),
                        );
                        ui.painter().circle_filled(av_rect.center(), 14.0, theme::ACCENT_PURPLE);
                        ui.painter().text(
                            av_rect.center(), egui::Align2::CENTER_CENTER,
                            &initials, egui::FontId::proportional(10.0), egui::Color32::WHITE,
                        );
                        let av_resp = av_resp.on_hover_text("Account settings");
                        if av_resp.clicked() {
                            action = Some(ToolbarAction::OpenUserMenu);
                        }

                        ui.add_space(8.0);
                        vsep(ui);
                        ui.add_space(8.0);

                        // Reflog
                        if widgets::icon_btn_mat(ui, icons::HISTORY, 18.0, "Reflog").clicked()
                        { action = Some(ToolbarAction::OpenReflog); }

                        ui.add_space(4.0);

                        // Theme toggle
                        let theme_icon = if dark_theme { icons::LIGHT_MODE } else { icons::DARK_MODE };
                        if widgets::icon_btn_mat(ui, theme_icon, 18.0, "Toggle theme").clicked()
                        { action = Some(ToolbarAction::ToggleTheme); }

                        ui.add_space(4.0);

                        // Command palette
                        if widgets::icon_btn_mat(ui, icons::TERMINAL, 18.0, "Command palette  (Ctrl+K)").clicked()
                        { action = Some(ToolbarAction::OpenCommandPalette); }

                        ui.add_space(4.0);

                        // Search
                        let sc = if search_active { theme::ACCENT_TEAL } else { theme::text_muted() };
                        if ui.add(egui::Button::new(
                            icons::rich(icons::SEARCH, 18.0).color(sc),
                        ).fill(egui::Color32::TRANSPARENT).stroke(Stroke::NONE))
                        .on_hover_text("Search commits  (Ctrl+F)")
                        .clicked()
                        { action = Some(ToolbarAction::ToggleSearch); }

                        ui.add_space(8.0);
                        vsep(ui);
                        ui.add_space(8.0);

                        // Branch switcher
                        {
                            let s = self.state.lock().unwrap();
                            let branch = s.head_branch.clone().unwrap_or_else(|| "HEAD".into());
                            drop(s);
                            let mut bj = egui::text::LayoutJob::default();
                            bj.append(&icons::COMMIT.to_string(), 0.0,
                                egui::TextFormat { font_id: icons::font(15.0), color: theme::ACCENT_TEAL, line_height: Some(16.0), valign: egui::Align::Center, ..Default::default() });
                            bj.append(&format!("  {branch}"), 0.0,
                                egui::TextFormat { font_id: egui::FontId::proportional(12.5), color: theme::ACCENT_TEAL, line_height: Some(16.0), valign: egui::Align::TOP, ..Default::default() });
                            if ui.add(
                                egui::Button::new(bj)
                                    .fill(theme::bg_panel())
                                    .stroke(Stroke::new(1.0, theme::ACCENT_TEAL.gamma_multiply(0.4)))
                                    .rounding(6.0),
                            )
                            .on_hover_text("Switch branch  (Ctrl+B)")
                            .clicked() {
                                action = Some(ToolbarAction::OpenBranchSwitcher);
                            }
                        }

                        // Spinner + progress when remote op in flight
                        if let Some(ref label) = op_label {
                            ui.add_space(10.0);
                            ui.label(RichText::new(label).color(theme::text_muted()).size(11.0));
                            ui.spinner();
                        }
                    });
                });
            });

        action
    }
}

fn vsep(ui: &mut egui::Ui) {
    ui.add(egui::Separator::default().vertical().spacing(4.0));
}
