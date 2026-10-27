use crate::ui::button::Button;
use std::path::PathBuf;

use egui::{RichText, Stroke};

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
    OpenProvider,
    Undo,
    Redo,
}

// ── Overflow item kind (Clone+Copy so we can collect them without owning actions) ──

#[derive(Clone, Copy, PartialEq)]
enum OverflowKind {
    Fetch,
    Pull,
    Push,
    NewBranch,
    Stash,
    BranchFilter,
}

struct OverflowItem {
    kind: OverflowKind,
    icon: char,
    label: &'static str,
    enabled: bool,
}

impl OverflowKind {
    fn to_action(self) -> ToolbarAction {
        match self {
            Self::Fetch => ToolbarAction::Fetch,
            Self::Pull => ToolbarAction::Pull,
            Self::Push => ToolbarAction::Push,
            Self::NewBranch => ToolbarAction::CreateBranch,
            Self::Stash => ToolbarAction::Stash,
            Self::BranchFilter => ToolbarAction::ToggleAllBranches,
        }
    }
}

// Approximate rendered widths of each left-section group (buttons + spacers)
const REPO_PILL_MIN_W: f32 = 110.0; // repo name capped at this
const SYNC_GROUP_W: f32 = 280.0; // Fetch + Pull + Push
const BRANCH_OPS_W: f32 = 260.0; // New Branch + Stash
const FILTER_W: f32 = 148.0; // All Branches toggle
const OVERFLOW_BTN_W: f32 = 46.0; // the ⋯ button
                                  // Minimum width we keep on the right before overflow kicks in
const RIGHT_MIN_W: f32 = 580.0;

pub struct Toolbar {
    state: SharedState,
}

impl Toolbar {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    pub fn show(
        &mut self,
        root_ui: &mut egui::Ui,
        recent_repos: &[PathBuf],
        dark_theme: bool,
        search_active: bool,
        show_all_branches: bool,
        enabled: bool,
    ) -> Option<ToolbarAction> {
        let mut action: Option<ToolbarAction> = None;

        egui::Panel::top("toolbar")
            .exact_size(62.0)
            .frame(
                egui::Frame::NONE
                    .fill(theme::bg_toolbar())
                    .inner_margin(egui::Margin::symmetric(12, 0)),
            )
            .show(root_ui, |ui| {
                if !enabled {
                    ui.disable();
                }
                let r = ui.max_rect();
                ui.painter().line_segment(
                    [r.left_bottom(), r.right_bottom()],
                    Stroke::new(1.0, theme::border()),
                );

                // Total usable width (minus horizontal margins already applied by the frame)
                let total_w = r.width() - 24.0;

                // Left section gets everything minus the guaranteed right section
                let left_w = (total_w - RIGHT_MIN_W).max(REPO_PILL_MIN_W + OVERFLOW_BTN_W + 20.0);

                ui.horizontal_centered(|ui| {
                    let (has_repo, dirty, op_label) = {
                        let s = self.state.lock().unwrap();
                        (
                            s.repo_path.is_some(),
                            s.dirty_count > 0,
                            s.remote_op.clone(),
                        )
                    };
                    let busy = op_label.is_some();

                    // ── Left section: fixed budget ────────────────────────
                    let mut overflow: Vec<OverflowItem> = Vec::new();

                    ui.allocate_ui_with_layout(
                        egui::Vec2::new(left_w, 62.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            // ── Repo pill (always visible) ────────────────
                            {
                                let (repo_path, name) = {
                                    let s = self.state.lock().unwrap();
                                    let rp = s.repo_path.clone();
                                    let n = rp
                                        .as_ref()
                                        .and_then(|p| p.file_name())
                                        .map(|n| n.to_string_lossy().into_owned())
                                        .unwrap_or_else(|| "No repo".into());
                                    (rp, n)
                                };
                                // Truncate long repo names so the pill doesn't consume the budget
                                let display = if name.len() > 18 {
                                    format!("{}…", name.chars().take(17).collect::<String>())
                                } else {
                                    name
                                };

                                let btn = widgets::toolbar_btn_icon(
                                    ui,
                                    icons::FOLDER_OPEN,
                                    &display,
                                    true,
                                )
                                .on_hover_text("Open repository");
                                if btn.clicked() {
                                    if let Some(path) = rfd::FileDialog::new().pick_folder() {
                                        action = Some(ToolbarAction::OpenRepo(path));
                                    }
                                }
                                btn.context_menu(|ui| {
                                    ui.set_min_width(200.0);
                                    if repo_path.is_some() {
                                        if ui.button("Copy Path").clicked() {
                                            action = Some(ToolbarAction::CopyRepoPath);
                                            ui.close();
                                        }
                                        if ui.button("Open External Terminal").clicked() {
                                            action = Some(ToolbarAction::OpenTerminal);
                                            ui.close();
                                        }
                                        if ui.button("Reveal in File Manager").clicked() {
                                            action = Some(ToolbarAction::RevealInFolder);
                                            ui.close();
                                        }
                                        ui.separator();
                                    }
                                    if ui.button("Clone Repository…").clicked() {
                                        action = Some(ToolbarAction::CloneRepo);
                                        ui.close();
                                    }
                                    if ui.button("Initialize Repository…").clicked() {
                                        action = Some(ToolbarAction::InitRepo);
                                        ui.close();
                                    }
                                });

                                let recent = ui.menu_button(
                                    icons::rich(icons::EXPAND_MORE, 18.0)
                                        .color(theme::text_secondary()),
                                    |ui| {
                                        if let Some(selected) = recent_repositories_menu(
                                            ui,
                                            recent_repos,
                                            repo_path.as_deref(),
                                        ) {
                                            action = Some(selected);
                                        }
                                    },
                                );
                                recent.response.on_hover_text("Recent repositories");
                            }

                            ui.add_space(6.0);
                            vsep(ui);
                            ui.add_space(6.0);

                            // ── Sync group: Fetch / Pull / Push ───────────
                            //    Hide into overflow if it won't fit alongside the overflow btn
                            let avail = ui.available_width();
                            if avail >= SYNC_GROUP_W + OVERFLOW_BTN_W {
                                if widgets::toolbar_btn_icon(
                                    ui,
                                    icons::DOWNLOAD,
                                    "Fetch",
                                    has_repo && !busy,
                                )
                                .on_hover_text("Fetch all remotes  (Ctrl+Shift+F)")
                                .clicked()
                                {
                                    action = Some(ToolbarAction::Fetch);
                                }

                                ui.add_space(3.0);

                                let pull_btn = widgets::toolbar_btn_icon(
                                    ui,
                                    icons::SWAP_VERT,
                                    "Pull",
                                    has_repo && !busy,
                                )
                                .on_hover_text("Fetch + fast-forward  (Ctrl+Shift+P)");
                                if pull_btn.clicked() {
                                    action = Some(ToolbarAction::Pull);
                                }
                                pull_btn.context_menu(|ui| {
                                    ui.set_min_width(190.0);
                                    if ui.button("Pull (fast-forward)").clicked() {
                                        action = Some(ToolbarAction::Pull);
                                        ui.close();
                                    }
                                    if ui.button("Pull with Rebase").clicked() {
                                        action = Some(ToolbarAction::PullWithRebase);
                                        ui.close();
                                    }
                                    if ui.button("Pull (ff-only)").clicked() {
                                        action = Some(ToolbarAction::PullFfOnly);
                                        ui.close();
                                    }
                                    ui.separator();
                                    if ui.button("Merge Branch…").clicked() {
                                        action = Some(ToolbarAction::MergeBranch);
                                        ui.close();
                                    }
                                    if ui.button("Rebase Onto…").clicked() {
                                        action = Some(ToolbarAction::RebaseOnto);
                                        ui.close();
                                    }
                                });

                                ui.add_space(3.0);

                                let push_btn = widgets::toolbar_btn_icon(
                                    ui,
                                    icons::UPLOAD,
                                    "Push",
                                    has_repo && !busy,
                                )
                                .on_hover_text("Push current branch  (Ctrl+Shift+U)");
                                if push_btn.clicked() {
                                    action = Some(ToolbarAction::Push);
                                }
                                push_btn.context_menu(|ui| {
                                    ui.set_min_width(200.0);
                                    if ui.button("Push").clicked() {
                                        action = Some(ToolbarAction::Push);
                                        ui.close();
                                    }
                                    if ui.button("Force Push with Lease").clicked() {
                                        action = Some(ToolbarAction::PushWithLease);
                                        ui.close();
                                    }
                                    if ui.button("Push and Set Upstream").clicked() {
                                        action = Some(ToolbarAction::PushSetUpstream);
                                        ui.close();
                                    }
                                });

                                ui.add_space(6.0);
                                vsep(ui);
                                ui.add_space(6.0);
                            } else {
                                overflow.push(OverflowItem {
                                    kind: OverflowKind::Fetch,
                                    icon: icons::DOWNLOAD,
                                    label: "Fetch",
                                    enabled: has_repo && !busy,
                                });
                                overflow.push(OverflowItem {
                                    kind: OverflowKind::Pull,
                                    icon: icons::SWAP_VERT,
                                    label: "Pull",
                                    enabled: has_repo && !busy,
                                });
                                overflow.push(OverflowItem {
                                    kind: OverflowKind::Push,
                                    icon: icons::UPLOAD,
                                    label: "Push",
                                    enabled: has_repo && !busy,
                                });
                            }

                            // ── Branch ops: New Branch / Stash ────────────
                            let avail = ui.available_width();
                            if avail >= BRANCH_OPS_W + OVERFLOW_BTN_W {
                                if widgets::toolbar_btn_icon(
                                    ui,
                                    icons::HUB,
                                    "New Branch",
                                    has_repo && !busy,
                                )
                                .on_hover_text("Create a new branch")
                                .clicked()
                                {
                                    action = Some(ToolbarAction::CreateBranch);
                                }

                                ui.add_space(3.0);

                                if widgets::toolbar_btn_icon(
                                    ui,
                                    icons::HISTORY,
                                    "Stash",
                                    has_repo && dirty && !busy,
                                )
                                .on_hover_text("Stash uncommitted changes")
                                .clicked()
                                {
                                    action = Some(ToolbarAction::Stash);
                                }

                                ui.add_space(6.0);
                                vsep(ui);
                                ui.add_space(6.0);
                            } else {
                                overflow.push(OverflowItem {
                                    kind: OverflowKind::NewBranch,
                                    icon: icons::HUB,
                                    label: "New Branch",
                                    enabled: has_repo && !busy,
                                });
                                overflow.push(OverflowItem {
                                    kind: OverflowKind::Stash,
                                    icon: icons::HISTORY,
                                    label: "Stash",
                                    enabled: has_repo && dirty && !busy,
                                });
                            }

                            // ── Branch filter toggle ───────────────────────
                            let avail = ui.available_width();
                            if avail >= FILTER_W + OVERFLOW_BTN_W {
                                let filter_color = if show_all_branches {
                                    theme::accent_blue()
                                } else {
                                    theme::text_secondary()
                                };
                                let (fi, fl) = if show_all_branches {
                                    (icons::SOURCE, "All Branches")
                                } else {
                                    (icons::COMMIT, "This Branch")
                                };
                                if widgets::toolbar_btn_icon_colored(
                                    ui,
                                    fi,
                                    fl,
                                    filter_color,
                                    true,
                                    theme::bg_panel(),
                                    if show_all_branches {
                                        theme::accent_blue().gamma_multiply(0.4)
                                    } else {
                                        theme::border()
                                    },
                                )
                                .on_hover_text("Toggle: show all branches or current branch only")
                                .clicked()
                                {
                                    action = Some(ToolbarAction::ToggleAllBranches);
                                }
                            } else {
                                overflow.push(OverflowItem {
                                    kind: OverflowKind::BranchFilter,
                                    icon: icons::SOURCE,
                                    label: "Toggle Branch Filter",
                                    enabled: true,
                                });
                            }

                            // ── Overflow ⋯ button ─────────────────────────
                            if !overflow.is_empty() {
                                ui.add_space(4.0);
                                let more_id = ui.make_persistent_id("toolbar_overflow_popup");
                                let more_btn = widgets::icon_btn_mat(
                                    ui,
                                    icons::MORE_VERT,
                                    18.0,
                                    "More actions",
                                );
                                let mut overflow_action: Option<ToolbarAction> = None;
                                egui::Popup::from_toggle_button_response(&more_btn)
                                    .id(more_id)
                                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                                    .show(|ui: &mut egui::Ui| {
                                        ui.set_min_width(190.0);
                                        for item in &overflow {
                                            if ui
                                                .scope(|ui| {
                                                    ui.spacing_mut().interact_size.x = 180.0;
                                                    widgets::toolbar_btn_icon(
                                                        ui,
                                                        item.icon,
                                                        item.label,
                                                        item.enabled,
                                                    )
                                                })
                                                .inner
                                                .clicked()
                                            {
                                                overflow_action = Some(item.kind.to_action());
                                                ui.close();
                                            }
                                        }
                                    });
                                if overflow_action.is_some() {
                                    action = overflow_action;
                                }
                            }
                        },
                    ); // end allocate_ui_with_layout (left section)

                    // ── Right section ─────────────────────────────────────
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(8.0);

                        if widgets::icon_btn_mat(ui, icons::SETTINGS, 19.0, "Settings and accounts")
                            .clicked()
                        {
                            action = Some(ToolbarAction::OpenUserMenu);
                        }

                        ui.add_space(8.0);
                        vsep(ui);
                        ui.add_space(8.0);

                        // Right-to-left: Redo is placed first so Undo appears on its left.
                        let (undo, redo) = {
                            let s = self.state.lock().unwrap();
                            (s.undo_step.clone(), s.redo_step.clone())
                        };
                        let key = if cfg!(target_os = "macos") {
                            "Cmd"
                        } else {
                            "Ctrl"
                        };
                        for (step, icon, redo, idle) in [
                            (&redo, icons::REDO, true, "Nothing to redo"),
                            (&undo, icons::UNDO, false, "Nothing to undo"),
                        ] {
                            let tip = match step {
                                Some((_, label)) if redo => format!("{label}  ({key}+Shift+Z)"),
                                Some((_, label)) => format!("{label}  ({key}+Z)"),
                                None => idle.to_string(),
                            };
                            let response = ui
                                .add_enabled_ui(has_repo && step.is_some() && !busy, |ui| {
                                    widgets::icon_btn_mat(ui, icon, 18.0, &tip)
                                })
                                .inner;
                            if response.clicked() {
                                action = Some(if redo {
                                    ToolbarAction::Redo
                                } else {
                                    ToolbarAction::Undo
                                });
                            }
                            ui.add_space(2.0);
                        }

                        ui.add_space(6.0);
                        vsep(ui);
                        ui.add_space(8.0);

                        if widgets::icon_btn_mat(ui, icons::HISTORY, 18.0, "Reflog").clicked() {
                            action = Some(ToolbarAction::OpenReflog);
                        }

                        ui.add_space(4.0);

                        if widgets::icon_btn_mat(
                            ui,
                            icons::BUG_REPORT,
                            18.0,
                            "Provider integrations  (issues, CI, releases)",
                        )
                        .clicked()
                        {
                            action = Some(ToolbarAction::OpenProvider);
                        }

                        ui.add_space(4.0);

                        let theme_icon = if dark_theme {
                            icons::LIGHT_MODE
                        } else {
                            icons::DARK_MODE
                        };
                        if widgets::icon_btn_mat(ui, theme_icon, 18.0, "Toggle theme").clicked() {
                            action = Some(ToolbarAction::ToggleTheme);
                        }

                        ui.add_space(4.0);

                        if widgets::icon_btn_mat(
                            ui,
                            icons::TERMINAL,
                            18.0,
                            "Command palette  (Ctrl+K)",
                        )
                        .clicked()
                        {
                            action = Some(ToolbarAction::OpenCommandPalette);
                        }

                        ui.add_space(4.0);

                        let sc = if search_active {
                            theme::accent_teal()
                        } else {
                            theme::text_muted()
                        };
                        if widgets::icon_btn_mat_colored(
                            ui,
                            icons::SEARCH,
                            18.0,
                            "Search commits  (Ctrl+F)",
                            sc,
                        )
                        .clicked()
                        {
                            action = Some(ToolbarAction::ToggleSearch);
                        }

                        ui.add_space(8.0);
                        vsep(ui);
                        ui.add_space(8.0);

                        // Branch switcher — cap branch name to prevent right section bloat
                        {
                            let branch_raw = self
                                .state
                                .lock()
                                .unwrap()
                                .head_branch
                                .clone()
                                .unwrap_or_else(|| "HEAD".into());
                            let branch = if branch_raw.len() > 22 {
                                format!("{}…", &branch_raw.chars().take(21).collect::<String>())
                            } else {
                                branch_raw
                            };
                            if widgets::toolbar_btn_icon_colored(
                                ui,
                                icons::COMMIT,
                                &branch,
                                theme::accent_teal(),
                                true,
                                theme::bg_panel(),
                                theme::accent_teal().gamma_multiply(0.4),
                            )
                            .on_hover_text("Switch branch  (Ctrl+B)")
                            .clicked()
                            {
                                action = Some(ToolbarAction::OpenBranchSwitcher);
                            }
                        }

                        // Spinner when remote op in flight
                        if let Some(ref label) = op_label {
                            ui.add_space(10.0);
                            ui.label(RichText::new(label).color(theme::text_muted()).size(12.0));
                            ui.spinner();
                        }
                    });
                });
            });

        action
    }
}

fn vsep(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(4.0, 24.0), egui::Sense::hover());
    ui.painter().line_segment(
        [rect.center_top(), rect.center_bottom()],
        Stroke::new(1.0, theme::border()),
    );
}

/// Shared recent-repository picker for the toolbar and File menu.
/// Only reads supplied paths; rendering never probes the filesystem.
pub fn recent_repositories_menu(
    ui: &mut egui::Ui,
    recent: &[PathBuf],
    current: Option<&std::path::Path>,
) -> Option<ToolbarAction> {
    let mut action = None;
    ui.set_width(
        400.0_f32
            .min(ui.ctx().content_rect().width() - 48.0)
            .max(240.0),
    );
    ui.spacing_mut().item_spacing = egui::vec2(8.0, 4.0);
    ui.spacing_mut().scroll = egui::style::ScrollStyle::thin();
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        ui.label(
            RichText::new("Recent repositories")
                .size(14.0)
                .color(theme::text_primary()),
        );
        ui.label(
            RichText::new(recent.len().to_string())
                .size(12.0)
                .color(theme::text_muted()),
        );
    });
    ui.add_space(6.0);
    ui.separator();
    egui::ScrollArea::vertical()
        .id_salt("recent_repository_list")
        .max_height((ui.ctx().content_rect().height() - 280.0).clamp(112.0, 360.0))
        .auto_shrink([false, true])
        .show(ui, |ui| {
            if recent.is_empty() {
                ui.add_space(12.0);
                ui.label(
                    RichText::new("Repositories you open will appear here.")
                        .size(13.0)
                        .color(theme::text_muted()),
                );
                ui.add_space(12.0);
            }
            for path in recent {
                ui.push_id(path, |ui| {
                    let name = path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path.display().to_string());
                    let parent = path
                        .parent()
                        .map(|path| path.display().to_string())
                        .unwrap_or_default();
                    let active = current == Some(path.as_path());
                    ui.horizontal(|ui| {
                        let width = (ui.available_width() - 44.0).max(160.0);
                        let row = ui.add_sized(
                            [width, 56.0],
                            Button::new("")
                                .fill(if active {
                                    theme::bg_selected()
                                } else {
                                    egui::Color32::TRANSPARENT
                                })
                                .stroke(Stroke::NONE),
                        );
                        let painter = ui.painter().with_clip_rect(row.rect);
                        if row.hovered() || row.has_focus() {
                            painter.rect_filled(row.rect, 7.0, theme::bg_hover());
                            if row.has_focus() {
                                painter.rect_stroke(
                                    row.rect,
                                    7.0,
                                    Stroke::new(1.0, theme::accent_blue()),
                                    egui::StrokeKind::Middle,
                                );
                            }
                        }
                        let icon = if active {
                            icons::CHECK
                        } else {
                            icons::FOLDER_OPEN
                        };
                        let icon_color = if active {
                            theme::accent_teal()
                        } else {
                            theme::text_muted()
                        };
                        let icon =
                            painter.layout_no_wrap(icon.to_string(), icons::font(18.0), icon_color);
                        let icon_position = egui::pos2(row.rect.left() + 22.0, row.rect.center().y)
                            - icon.mesh_bounds.center().to_vec2();
                        painter.galley(icon_position, icon, icon_color);
                        let text_left = row.rect.left() + 42.0;
                        let text_width = row.rect.width() - 54.0;
                        for (text, size, color, y, reserved) in [
                            (
                                name.as_str(),
                                14.0,
                                theme::text_primary(),
                                row.rect.top() + 9.0,
                                if active { 52.0 } else { 0.0 },
                            ),
                            (
                                parent.as_str(),
                                12.0,
                                theme::text_muted(),
                                row.rect.top() + 32.0,
                                0.0,
                            ),
                        ] {
                            let mut job = egui::text::LayoutJob::simple(
                                text.to_owned(),
                                egui::FontId::proportional(size),
                                color,
                                (text_width - reserved).max(40.0),
                            );
                            job.wrap.max_rows = 1;
                            job.wrap.break_anywhere = true;
                            let galley = painter.layout_job(job);
                            painter.galley(egui::pos2(text_left, y), galley, color);
                        }
                        if active {
                            painter.text(
                                egui::pos2(row.rect.right() - 10.0, row.rect.top() + 17.0),
                                egui::Align2::RIGHT_CENTER,
                                "Current",
                                egui::FontId::proportional(12.0),
                                theme::accent_teal(),
                            );
                        }
                        row.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::Button,
                                ui.is_enabled(),
                                format!("Open {}", path.display()),
                            )
                        });
                        if row.on_hover_text(path.display().to_string()).clicked() {
                            if !active {
                                action = Some(ToolbarAction::OpenRepo(path.clone()));
                            }
                            ui.close();
                        }
                        if widgets::icon_btn_mat(
                            ui,
                            icons::CLOSE,
                            16.0,
                            &format!(
                                "Remove {name} from recent repositories (keeps files on disk)"
                            ),
                        )
                        .clicked()
                        {
                            action = Some(ToolbarAction::RemoveFromRecent(path.clone()));
                            ui.close();
                        }
                    });
                });
            }
        });
    ui.separator();
    ui.add_space(4.0);
    if widgets::toolbar_btn_icon(ui, icons::FOLDER_OPEN, "Open another repository…", true).clicked()
    {
        ui.close();
        if let Some(path) = rfd::FileDialog::new().pick_folder() {
            action = Some(ToolbarAction::OpenRepo(path));
        }
    }
    ui.add_space(4.0);
    action
}
