use std::path::PathBuf;

use egui::{Context, RichText, Stroke};

use crate::state::SharedState;
use crate::theme;
use crate::ui::widgets;

pub enum ToolbarAction {
    OpenRepo(PathBuf),
    Stash,
    CreateBranch,
    Fetch,
    Pull,
    Push,
    ToggleTheme,
    ToggleSearch,
    OpenCommandPalette,
    OpenBranchSwitcher,
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
    ) -> Option<ToolbarAction> {
        let mut action = None;

        egui::TopBottomPanel::top("toolbar")
            .exact_height(54.0)
            .frame(
                egui::Frame::none()
                    .fill(theme::BG_TOOLBAR)
                    .inner_margin(egui::Margin::symmetric(12.0, 0.0)),
            )
            .show(ctx, |ui| {
                // Bottom separator
                let r = ui.max_rect();
                ui.painter().line_segment(
                    [r.left_bottom(), r.right_bottom()],
                    Stroke::new(1.0, theme::BORDER),
                );

                ui.horizontal_centered(|ui| {
                    let (has_repo, dirty, op_label) = {
                        let s = self.state.lock().unwrap();
                        (s.repo_path.is_some(), s.dirty_count > 0, s.remote_op.clone())
                    };
                    let busy = op_label.is_some();

                    // ── Repo name pill ───────────────────────────────────
                    {
                        let s = self.state.lock().unwrap();
                        let name = s.repo_path.as_ref()
                            .and_then(|p| p.file_name())
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "No repo".into());
                        drop(s);

                        let btn = ui.add(
                            egui::Button::new(
                                RichText::new(format!("⊞  {}", name))
                                    .color(theme::TEXT_PRIMARY)
                                    .size(13.0),
                            )
                            .fill(theme::BG_PANEL)
                            .stroke(Stroke::new(1.0, theme::BORDER))
                            .rounding(6.0),
                        );
                        if btn.clicked() {
                            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                                action = Some(ToolbarAction::OpenRepo(path));
                            }
                        }
                        // Recent repos drop-down
                        if !recent_repos.is_empty() {
                            let arrow = ui.add(
                                egui::Button::new(RichText::new("▾").color(theme::TEXT_MUTED).size(11.0))
                                    .fill(theme::BG_PANEL)
                                    .stroke(Stroke::new(1.0, theme::BORDER))
                                    .min_size(egui::Vec2::new(20.0, 0.0))
                                    .rounding(6.0),
                            );
                            arrow.context_menu(|ui| {
                                ui.set_min_width(260.0);
                                ui.label(RichText::new("Recent").color(theme::TEXT_MUTED).size(10.5));
                                ui.separator();
                                for p in recent_repos {
                                    let n = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                                    if ui.button(RichText::new(n).size(12.5)).clicked() {
                                        action = Some(ToolbarAction::OpenRepo(p.clone()));
                                        ui.close_menu();
                                    }
                                }
                            });
                        }
                    }

                    ui.add_space(6.0);
                    vsep(ui);
                    ui.add_space(6.0);

                    // ── Git actions ──────────────────────────────────────
                    if widgets::toolbar_btn(ui, "↓  Fetch", has_repo && !busy)
                        .on_hover_text("Fetch all remotes  (Ctrl+Shift+F)").clicked()
                    { action = Some(ToolbarAction::Fetch); }

                    ui.add_space(3.0);

                    if widgets::toolbar_btn(ui, "⇅  Pull", has_repo && !busy)
                        .on_hover_text("Fetch + fast-forward  (Ctrl+Shift+P)").clicked()
                    { action = Some(ToolbarAction::Pull); }

                    ui.add_space(3.0);

                    if widgets::toolbar_btn(ui, "↑  Push", has_repo && !busy)
                        .on_hover_text("Push current branch  (Ctrl+Shift+U)").clicked()
                    { action = Some(ToolbarAction::Push); }

                    ui.add_space(6.0);
                    vsep(ui);
                    ui.add_space(6.0);

                    if widgets::toolbar_btn(ui, "⎇  New Branch", has_repo && !busy).clicked()
                    { action = Some(ToolbarAction::CreateBranch); }

                    ui.add_space(3.0);

                    if widgets::toolbar_btn(ui, "⚑  Stash", has_repo && dirty && !busy)
                        .on_hover_text("Stash uncommitted changes").clicked()
                    { action = Some(ToolbarAction::Stash); }

                    // ── Right side ───────────────────────────────────────
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(8.0);

                        // Avatar placeholder
                        let (av_rect, _) = ui.allocate_exact_size(
                            egui::Vec2::splat(28.0), egui::Sense::hover(),
                        );
                        ui.painter().circle_filled(av_rect.center(), 14.0, theme::ACCENT_PURPLE);
                        ui.painter().text(
                            av_rect.center(), egui::Align2::CENTER_CENTER,
                            "AD", egui::FontId::proportional(10.0), egui::Color32::WHITE,
                        );

                        ui.add_space(8.0);
                        vsep(ui);
                        ui.add_space(8.0);

                        // Theme toggle
                        let ti = if dark_theme { "☀" } else { "🌙" };
                        if icon_btn(ui, ti, "Toggle theme").clicked()
                        { action = Some(ToolbarAction::ToggleTheme); }

                        ui.add_space(4.0);

                        // Command palette
                        if icon_btn(ui, "⌘", "Command palette  (Ctrl+K)").clicked()
                        { action = Some(ToolbarAction::OpenCommandPalette); }

                        ui.add_space(4.0);

                        // Search
                        let sc = if search_active { theme::ACCENT_TEAL } else { theme::TEXT_MUTED };
                        if ui.add(egui::Button::new(
                            RichText::new("🔍").color(sc).size(14.0),
                        ).fill(egui::Color32::TRANSPARENT).stroke(Stroke::NONE)).clicked()
                        { action = Some(ToolbarAction::ToggleSearch); }

                        ui.add_space(8.0);
                        vsep(ui);
                        ui.add_space(8.0);

                        // Branch switcher
                        {
                            let s = self.state.lock().unwrap();
                            let branch = s.head_branch.clone().unwrap_or_else(|| "HEAD".into());
                            drop(s);
                            if ui.add(
                                egui::Button::new(
                                    RichText::new(format!("⎇  {}", branch))
                                        .color(theme::ACCENT_TEAL)
                                        .size(12.5),
                                )
                                .fill(theme::BG_PANEL)
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
                            ui.label(RichText::new(label).color(theme::TEXT_MUTED).size(11.0));
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

fn icon_btn<'a>(ui: &mut egui::Ui, icon: &str, tip: &'a str) -> egui::Response {
    ui.add(egui::Button::new(RichText::new(icon).color(theme::TEXT_MUTED).size(14.0))
        .fill(egui::Color32::TRANSPARENT)
        .stroke(Stroke::NONE))
        .on_hover_text(tip)
}
