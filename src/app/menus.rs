//! The application menu bar (File, Edit, View, Repository, Branch, Help).
#![allow(unused_imports)]
use super::*;

fn menu_entry(
    ui: &mut egui::Ui,
    label: &str,
    command: Option<crate::commands::Command>,
    state: &crate::state::SharedState,
    shortcuts: &crate::commands::Shortcuts,
    tooltip: &str,
) -> egui::Response {
    menu_entry_gated(ui, label, command, state, shortcuts, tooltip, true)
}

fn menu_entry_gated(
    ui: &mut egui::Ui,
    label: &str,
    command: Option<crate::commands::Command>,
    state: &crate::state::SharedState,
    shortcuts: &crate::commands::Shortcuts,
    tooltip: &str,
    gate: bool,
) -> egui::Response {
    let snapshot = state.lock().unwrap();
    let reason = command
        .and_then(|command| command.disabled_reason(&snapshot))
        .or_else(|| {
            (!gate).then_some(if label == "Export Patch…" {
                "Select a file change first"
            } else {
                "Open a repository first"
            })
        });
    let enabled = gate && command.is_none_or(|command| command.available(&snapshot));
    let shortcut = command
        .and_then(|command| shortcuts.get(command))
        .map(|binding| binding.label());
    let mut explanation = tooltip.to_owned();
    if let Some(reason) = reason {
        explanation.push(' ');
        explanation.push_str(reason);
    }
    drop(snapshot);
    crate::ui::kit::menu_item(ui, label, shortcut.as_deref(), enabled, &explanation)
}

impl RecordeApp {
    pub(super) fn show_menu_contents(&mut self, ctx: &Context, ui: &mut egui::Ui) {
        if self.commit_graph_view.has_pending_action() {
            ui.disable();
        }
        let icon_texture = self.app_icon_texture.clone();
        egui::MenuBar::new().ui(ui, |ui| {
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
                if menu_entry(
                    ui,
                    "Open Repository…",
                    Some(crate::commands::Command::Open),
                    &self.state,
                    &self.config.shortcuts,
                    "Choose a repository folder to open.",
                )
                .clicked()
                {
                    if let Some(path) = rfd::FileDialog::new().pick_folder() {
                        self.config.add_recent(path.clone());
                        load_repo(path, self.state.clone());
                    }
                    ui.close();
                }
                if menu_entry(
                    ui,
                    "Clone Repository…",
                    Some(crate::commands::Command::Clone),
                    &self.state,
                    &self.config.shortcuts,
                    "Copy a remote repository to this computer.",
                )
                .clicked()
                {
                    self.state.lock().unwrap().modal = ModalKind::CloneRepo;
                    ui.close();
                }
                if menu_entry(
                    ui,
                    "Initialize Repository…",
                    Some(crate::commands::Command::Init),
                    &self.state,
                    &self.config.shortcuts,
                    "Start tracking a folder with Git.",
                )
                .clicked()
                {
                    self.state.lock().unwrap().modal = ModalKind::InitRepo;
                    ui.close();
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
                if menu_entry(
                    ui,
                    "Settings",
                    Some(crate::commands::Command::Settings),
                    &self.state,
                    &self.config.shortcuts,
                    "Change Recorde preferences.",
                )
                .clicked()
                {
                    self.open_settings(SettingsTab::App);
                    ui.close();
                }
                if menu_entry(
                    ui,
                    "Quit",
                    None,
                    &self.state,
                    &self.config.shortcuts,
                    "Close Recorde.",
                )
                .clicked()
                {
                    self.request_quit(ctx);
                    ui.close();
                }
            });

            ui.menu_button("Edit", |ui| {
                if menu_entry(
                    ui,
                    "Undo",
                    Some(crate::commands::Command::Undo),
                    &self.state,
                    &self.config.shortcuts,
                    "Restore the state from before the last operation.",
                )
                .clicked()
                {
                    self.open_undo(false);
                    ui.close();
                }
                if menu_entry(
                    ui,
                    "Redo",
                    Some(crate::commands::Command::Redo),
                    &self.state,
                    &self.config.shortcuts,
                    "Repeat the operation you just undid.",
                )
                .clicked()
                {
                    self.open_undo(true);
                    ui.close();
                }
                ui.separator();
                if menu_entry(
                    ui,
                    "Copy Selected Commit SHA",
                    Some(crate::commands::Command::CopyCommit),
                    &self.state,
                    &self.config.shortcuts,
                    "Copy the selected commit identifier.",
                )
                .clicked()
                {
                    if let Some(oid) = self.state.lock().unwrap().selected_commit.clone() {
                        ui.ctx().copy_text(oid);
                    }
                    ui.close();
                }
                if menu_entry(
                    ui,
                    "Find Commits",
                    Some(crate::commands::Command::Search),
                    &self.state,
                    &self.config.shortcuts,
                    "Find commits by message, author or change.",
                )
                .clicked()
                {
                    self.state.lock().unwrap().search_active = true;
                    ui.close();
                }
                if menu_entry(
                    ui,
                    "Clear Selection",
                    None,
                    &self.state,
                    &self.config.shortcuts,
                    "Clear the selected commit and file.",
                )
                .clicked()
                {
                    let mut s = self.state.lock().unwrap();
                    s.selected_commits.clear();
                    s.selected_commit = None;
                    s.selected_commit_detail = None;
                    s.selected_file = None;
                    s.current_diff = None;
                    ui.close();
                }
            });

            ui.menu_button("View", |ui| {
                let show_all = self.state.lock().unwrap().show_all_branches;
                let branch_filter_label = if show_all {
                    "Show Current Branch Only"
                } else {
                    "Show All Branches"
                };
                if menu_entry(
                    ui,
                    branch_filter_label,
                    Some(crate::commands::Command::AllBranches),
                    &self.state,
                    &self.config.shortcuts,
                    "Choose whether the history includes every branch.",
                )
                .clicked()
                {
                    self.handle_toolbar(ctx, ToolbarAction::ToggleAllBranches);
                    ui.close();
                }
                if menu_entry(
                    ui,
                    "Toggle Theme",
                    Some(crate::commands::Command::Theme),
                    &self.state,
                    &self.config.shortcuts,
                    "Switch between light and dark appearance.",
                )
                .clicked()
                {
                    self.handle_toolbar(ctx, ToolbarAction::ToggleTheme);
                    ui.close();
                }
                ui.separator();
                if crate::ui::kit::menu_check(ui, &mut self.config.show_sidebar, "Show Sidebar")
                    .on_hover_text("Show or hide the repository list.")
                    .changed()
                {
                    self.save_config();
                }
                if crate::ui::kit::menu_check(
                    ui,
                    &mut self.config.show_right_panel,
                    "Show Changes / Details Panel",
                )
                .on_hover_text("Show or hide the Changes and commit details panel.")
                .changed()
                {
                    self.save_config();
                }
                if crate::ui::kit::menu_check(
                    ui,
                    &mut self.config.show_diff_panel,
                    "Show Diff Panel",
                )
                .on_hover_text("Show or hide the selected file's changes.")
                .changed()
                {
                    self.save_config();
                }
                if crate::ui::kit::menu_check(ui, &mut self.config.show_terminal, "Show Terminal")
                    .on_hover_text("Show or hide the integrated terminal.")
                    .changed()
                {
                    if self.config.show_terminal {
                        self.terminal.focus();
                    }
                    self.save_config();
                }
                if menu_entry(
                    ui,
                    "Reset Layout",
                    None,
                    &self.state,
                    &self.config.shortcuts,
                    "Restore the default panel arrangement.",
                )
                .clicked()
                {
                    self.reset_layout();
                    ui.close();
                }
                ui.separator();
                // Name each entry's command so the menu shows its effective
                // shortcut (Ctrl+1 to Ctrl+4 by default), like the other menus.
                for (label, pane, command) in [
                    ("Focus Sidebar", 1, crate::commands::Command::Sidebar),
                    ("Focus Commit Graph", 2, crate::commands::Command::Graph),
                    ("Focus Changes", 3, crate::commands::Command::Changes),
                    ("Focus Diff", 4, crate::commands::Command::Diff),
                ] {
                    if menu_entry(
                        ui,
                        label,
                        Some(command),
                        &self.state,
                        &self.config.shortcuts,
                        "Move keyboard focus to this part of the window.",
                    )
                    .clicked()
                    {
                        self.focus_pane(pane);
                        ui.close();
                    }
                }
            });

            ui.menu_button("Repository", |ui| {
                let has_repo = self.state.lock().unwrap().repo_path.is_some();
                if menu_entry_gated(
                    ui,
                    "Open in Editor",
                    None,
                    &self.state,
                    &self.config.shortcuts,
                    "Open this repository in your configured editor.",
                    has_repo,
                )
                .clicked()
                {
                    if let Err(e) = open_repo_in_editor(
                        self.state.lock().unwrap().repo_path.clone(),
                        &self.config.default_editor,
                    ) {
                        self.state.lock().unwrap().error = Some(e);
                    }
                    ui.close();
                }
                if menu_entry_gated(
                    ui,
                    "Integrated Terminal",
                    Some(crate::commands::Command::IntegratedTerminal),
                    &self.state,
                    &self.config.shortcuts,
                    "Open a terminal inside Recorde at this repository.",
                    has_repo,
                )
                .clicked()
                {
                    self.toggle_terminal();
                    ui.close();
                }
                if menu_entry_gated(
                    ui,
                    "Open External Terminal",
                    Some(crate::commands::Command::Terminal),
                    &self.state,
                    &self.config.shortcuts,
                    "Open a terminal window in this repository folder.",
                    has_repo,
                )
                .clicked()
                {
                    let repo_path = self.state.lock().unwrap().repo_path.clone();
                    if let Some(rp) = repo_path {
                        if let Err(error) = lifecycle::try_open_terminal(&rp) {
                            self.state.lock().unwrap().error = Some(error);
                        }
                    }
                    ui.close();
                }
                if menu_entry_gated(
                    ui,
                    "Reveal in File Manager",
                    Some(crate::commands::Command::Reveal),
                    &self.state,
                    &self.config.shortcuts,
                    "Open this repository folder in your file manager.",
                    has_repo,
                )
                .clicked()
                {
                    if let Some(rp) = self.state.lock().unwrap().repo_path.clone() {
                        lifecycle::reveal_in_file_manager(&rp);
                    }
                    ui.close();
                }
                if menu_entry_gated(
                    ui,
                    "Copy Path",
                    Some(crate::commands::Command::CopyPath),
                    &self.state,
                    &self.config.shortcuts,
                    "Copy the repository folder path.",
                    has_repo,
                )
                .clicked()
                {
                    let path_str = self
                        .state
                        .lock()
                        .unwrap()
                        .repo_path
                        .as_ref()
                        .map(|p| crate::ui::kit::display_path(p))
                        .unwrap_or_default();
                    ctx.copy_text(path_str);
                    ui.close();
                }
                if menu_entry_gated(
                    ui,
                    "Pull Requests…",
                    Some(crate::commands::Command::Provider),
                    &self.state,
                    &self.config.shortcuts,
                    "Review pull requests and issues for this repository.",
                    has_repo,
                )
                .clicked()
                {
                    self.repo_ui.pull_requests_open = true;
                    ui.close();
                }
                ui.separator();
                if menu_entry(
                    ui,
                    "Fetch",
                    Some(crate::commands::Command::Fetch),
                    &self.state,
                    &self.config.shortcuts,
                    "Check the remote for new commits without changing your files.",
                )
                .clicked()
                {
                    remote::fetch_all(&mut self.runner, &self.state);
                    ui.close();
                }
                if menu_entry(
                    ui,
                    "Pull",
                    Some(crate::commands::Command::Pull),
                    &self.state,
                    &self.config.shortcuts,
                    "Download new commits and update your current branch.",
                )
                .clicked()
                {
                    remote::pull_branch(&mut self.runner, &self.state);
                    ui.close();
                }
                if menu_entry(
                    ui,
                    "Push",
                    Some(crate::commands::Command::Push),
                    &self.state,
                    &self.config.shortcuts,
                    "Upload your commits to the remote.",
                )
                .clicked()
                {
                    remote::push_branch(&mut self.runner, &self.state);
                    ui.close();
                }
                if menu_entry(
                    ui,
                    "Refresh",
                    Some(crate::commands::Command::Refresh),
                    &self.state,
                    &self.config.shortcuts,
                    "Re-read the repository from disk.",
                )
                .clicked()
                {
                    if let Some(path) = self.state.lock().unwrap().repo_path.clone() {
                        load_repo(path, self.state.clone());
                    }
                    ui.close();
                }
                ui.separator();
                let has_diff = self.state.lock().unwrap().current_diff.is_some();
                if menu_entry_gated(
                    ui,
                    "Export Patch…",
                    None,
                    &self.state,
                    &self.config.shortcuts,
                    "Save the selected file changes as a patch.",
                    has_diff,
                )
                .clicked()
                {
                    let diff = self.state.lock().unwrap().current_diff.clone();
                    if let Some(ref d) = diff {
                        let name = d.file_path.replace('/', "_");
                        if let Some(dest) = rfd::FileDialog::new()
                            .add_filter("Patch", &["patch", "diff"])
                            .set_file_name(format!("{}.patch", name))
                            .save_file()
                        {
                            if let Err(e) = crate::git::patch::export_patch(d, &dest) {
                                self.state.lock().unwrap().error = Some(e);
                            }
                        }
                    }
                    ui.close();
                }
                if menu_entry_gated(
                    ui,
                    "Apply Patch File…",
                    None,
                    &self.state,
                    &self.config.shortcuts,
                    "Apply changes from a patch file to this repository.",
                    has_repo,
                )
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
                    ui.close();
                }
                ui.separator();
                if menu_entry(
                    ui,
                    "Repository Settings",
                    None,
                    &self.state,
                    &self.config.shortcuts,
                    "Change settings for this repository.",
                )
                .clicked()
                {
                    self.open_settings(SettingsTab::Repository);
                    ui.close();
                }
            });

            ui.menu_button("Branch", |ui| {
                if menu_entry(
                    ui,
                    "Switch Branch…",
                    Some(crate::commands::Command::SwitchBranch),
                    &self.state,
                    &self.config.shortcuts,
                    "Choose another branch. A branch is a separate line of work.",
                )
                .clicked()
                {
                    self.branch_switcher_open = true;
                    ui.close();
                }
                if menu_entry(
                    ui,
                    "New Branch…",
                    Some(crate::commands::Command::Branch),
                    &self.state,
                    &self.config.shortcuts,
                    "Create a separate line of work.",
                )
                .clicked()
                {
                    self.state.lock().unwrap().modal = ModalKind::CreateBranch { from_oid: None };
                    ui.close();
                }
                ui.separator();
                if menu_entry(
                    ui,
                    "Merge Branch…",
                    Some(crate::commands::Command::Merge),
                    &self.state,
                    &self.config.shortcuts,
                    "Bring another branch's commits into this branch.",
                )
                .clicked()
                {
                    self.state.lock().unwrap().modal = ModalKind::MergeBranch { branch: None };
                    ui.close();
                }
                if menu_entry(
                    ui,
                    "Git Flow…",
                    Some(crate::commands::Command::GitFlow),
                    &self.state,
                    &self.config.shortcuts,
                    "Create and finish feature, release and hotfix branches.",
                )
                .clicked()
                {
                    self.open_git_flow();
                    ui.close();
                }
                if menu_entry(
                    ui,
                    "Rebase Onto…",
                    Some(crate::commands::Command::Rebase),
                    &self.state,
                    &self.config.shortcuts,
                    "Replay this branch's commits on top of another branch.",
                )
                .clicked()
                {
                    self.state.lock().unwrap().modal = ModalKind::RebaseOnto;
                    ui.close();
                }
                ui.separator();
                if menu_entry(
                    ui,
                    "Stash Changes…",
                    Some(crate::commands::Command::Stash),
                    &self.state,
                    &self.config.shortcuts,
                    "Set aside uncommitted changes so you can switch tasks.",
                )
                .clicked()
                {
                    self.repo_ui.stash_message_input = "WIP".into();
                    self.state.lock().unwrap().modal = ModalKind::StashWithMessage;
                    ui.close();
                }
            });

            ui.menu_button("Help", |ui| {
                if ui.button("Offline support report…").clicked() {
                    self.support.open = true;
                    ui.close();
                }
                if let Some(value) = &self.git_readiness.value {
                    ui.label(value.summary());
                }
                if ui.button("Git readiness…").clicked() {
                    self.open_settings(SettingsTab::Git);
                    ui.close();
                }
                ui.separator();
                for (label, page) in [
                    ("User Guide", "User-Guide"),
                    ("Feature Reference", "Feature-Reference"),
                    ("Keyboard Shortcuts", "Keyboard-Shortcuts"),
                    ("Troubleshooting", "Troubleshooting"),
                ] {
                    if menu_entry(
                        ui,
                        label,
                        None,
                        &self.state,
                        &self.config.shortcuts,
                        "Open this help page.",
                    )
                    .clicked()
                    {
                        self.help.open_page(page);
                        ui.close();
                    }
                }
                ui.separator();
                if menu_entry(
                    ui,
                    "About Recorde",
                    None,
                    &self.state,
                    &self.config.shortcuts,
                    "View Recorde version and update information.",
                )
                .clicked()
                {
                    self.open_settings(SettingsTab::About);
                    ui.close();
                }
            });
        });
    }
}
