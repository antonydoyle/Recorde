use std::path::PathBuf;

use egui::{Context, RichText};

use crate::state::SharedState;
use crate::theme;

pub enum ToolbarAction {
    OpenRepo(PathBuf),
    Stash,
    CreateBranch,
    Fetch,
    Pull,
    Push,
    ToggleTheme,
    ToggleSearch,
}

pub struct Toolbar {
    state: SharedState,
}

impl Toolbar {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    pub fn show(&mut self, ctx: &Context, recent_repos: &[PathBuf], dark_theme: bool) -> Option<ToolbarAction> {
        let mut action = None;

        egui::TopBottomPanel::top("toolbar")
            .exact_height(36.0)
            .frame(
                egui::Frame::none()
                    .fill(theme::BG_ELEVATED)
                    .inner_margin(egui::Margin::symmetric(8.0, 0.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    // ── Open repo + recent dropdown ──────────────────────
                    let open_btn = ui.add(
                        egui::Button::new(
                            RichText::new("⊞  Open Repository")
                                .color(theme::TEXT_PRIMARY)
                                .size(12.5),
                        )
                        .fill(theme::BG_SURFACE)
                        .stroke(egui::Stroke::new(1.0, theme::BORDER)),
                    );
                    if open_btn.clicked() {
                        if let Some(path) = rfd::FileDialog::new().pick_folder() {
                            action = Some(ToolbarAction::OpenRepo(path));
                        }
                    }

                    // Recent repos drop-down arrow
                    if !recent_repos.is_empty() {
                        let arrow = ui.add(
                            egui::Button::new(RichText::new("▾").color(theme::TEXT_MUTED).size(11.0))
                                .fill(theme::BG_SURFACE)
                                .stroke(egui::Stroke::new(1.0, theme::BORDER))
                                .min_size(egui::Vec2::new(18.0, 0.0)),
                        );
                        arrow.context_menu(|ui| {
                            ui.set_min_width(280.0);
                            ui.label(RichText::new("Recent Repositories").color(theme::TEXT_MUTED).size(10.5));
                            ui.separator();
                            for path in recent_repos {
                                let name = path.file_name()
                                    .map(|n| n.to_string_lossy().into_owned())
                                    .unwrap_or_else(|| path.display().to_string());
                                let display = format!("{}  —  {}", name, path.display());
                                if ui.button(RichText::new(display).size(12.0)).clicked() {
                                    action = Some(ToolbarAction::OpenRepo(path.clone()));
                                    ui.close_menu();
                                }
                            }
                        });
                        if arrow.clicked() {
                            // also open context menu on plain click
                        }
                    }

                    ui.add_space(6.0);
                    sep(ui);
                    ui.add_space(6.0);

                    let (has_repo, dirty, op_label, search_active) = {
                        let s = self.state.lock().unwrap();
                        (s.repo_path.is_some(), s.dirty_count > 0, s.remote_op.clone(), s.search_active)
                    };
                    let busy = op_label.is_some();

                    // ── Branch / stash ───────────────────────────────────
                    if icon_btn(ui, "⎇  New Branch", has_repo && !busy).clicked() {
                        action = Some(ToolbarAction::CreateBranch);
                    }
                    ui.add_space(2.0);
                    if icon_btn(ui, "⚑  Stash", has_repo && dirty && !busy)
                        .on_hover_text("Stash all uncommitted changes")
                        .clicked()
                    {
                        action = Some(ToolbarAction::Stash);
                    }

                    ui.add_space(6.0);
                    sep(ui);
                    ui.add_space(6.0);

                    // ── Remote ops ───────────────────────────────────────
                    if icon_btn(ui, "↓  Fetch", has_repo && !busy)
                        .on_hover_text("Fetch from all remotes")
                        .clicked() { action = Some(ToolbarAction::Fetch); }
                    ui.add_space(2.0);
                    if icon_btn(ui, "⇅  Pull", has_repo && !busy)
                        .on_hover_text("Fetch + fast-forward current branch")
                        .clicked() { action = Some(ToolbarAction::Pull); }
                    ui.add_space(2.0);
                    if icon_btn(ui, "↑  Push", has_repo && !busy)
                        .on_hover_text("Push current branch to upstream")
                        .clicked() { action = Some(ToolbarAction::Push); }

                    ui.add_space(8.0);
                    sep(ui);
                    ui.add_space(8.0);

                    // Repo name
                    if has_repo {
                        let path = self.state.lock().unwrap().repo_path.clone().unwrap();
                        let name = path.file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| path.display().to_string());
                        ui.label(RichText::new(name).color(theme::TEXT_PRIMARY).size(13.0).strong());
                    }

                    // Right-aligned: spinner, search, theme toggle
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(8.0);

                        // Theme toggle
                        let theme_icon = if dark_theme { "☀" } else { "🌙" };
                        let theme_tip  = if dark_theme { "Switch to light theme" } else { "Switch to dark theme" };
                        if ui.add(egui::Button::new(
                            RichText::new(theme_icon).color(theme::TEXT_MUTED).size(14.0),
                        ).fill(egui::Color32::TRANSPARENT).stroke(egui::Stroke::NONE))
                            .on_hover_text(theme_tip)
                            .clicked()
                        {
                            action = Some(ToolbarAction::ToggleTheme);
                        }

                        ui.add_space(4.0);

                        // Search toggle (Ctrl+F hint)
                        let search_color = if search_active { theme::ACCENT_BLUE } else { theme::TEXT_MUTED };
                        if ui.add(egui::Button::new(
                            RichText::new("🔍").color(search_color).size(13.0),
                        ).fill(if search_active { egui::Color32::from_rgb(0x1E, 0x2D, 0x4A) } else { egui::Color32::TRANSPARENT })
                            .stroke(egui::Stroke::NONE))
                            .on_hover_text("Search commits  (Ctrl+F)")
                            .clicked()
                        {
                            action = Some(ToolbarAction::ToggleSearch);
                        }

                        // Progress spinner
                        if let Some(ref label) = op_label {
                            ui.add_space(6.0);
                            ui.label(RichText::new(label).color(theme::TEXT_MUTED).size(11.5));
                            ui.spinner();
                        }
                    });
                });
            });

        action
    }
}

fn icon_btn(ui: &mut egui::Ui, label: &str, enabled: bool) -> egui::Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(label).color(theme::TEXT_SECONDARY).size(12.0))
            .fill(theme::BG_SURFACE)
            .stroke(egui::Stroke::new(1.0, theme::BORDER)),
    )
}

fn sep(ui: &mut egui::Ui) {
    ui.add(egui::Separator::default().vertical().spacing(4.0));
}
