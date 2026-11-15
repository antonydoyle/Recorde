//! Dialogs and panels for undo/redo, merge options, Git Flow, profiles,
//! SSH keys, repository templates, the integrated terminal and the editor.
use egui::{Context, Key, RichText};

use super::{
    git_identity, modal_window, primary_btn, secondary_btn, settings_text_field, styled_label,
    styled_muted, RecordeApp, SettingsTab,
};
use crate::git::gitflow::{FlowConfig, Kind};
use crate::git::merge::MergeMode;
use crate::state::ModalKind;
use crate::theme;

/// Result of a background SSH key task.
pub(super) enum SshDone {
    Generated(std::path::PathBuf, String),
    Message(String),
}

/// Exact local targets reviewed by the user before a multi-step Git Flow finish.
#[derive(Clone)]
pub(super) struct FlowFinish {
    path: std::path::PathBuf,
    epoch: u64,
    flow: FlowConfig,
    kind: Kind,
    name: String,
    source: String,
    refs: Vec<(String, Option<String>)>,
    head: String,
    head_ref: Option<String>,
}

impl FlowFinish {
    fn reference(repo: &git2::Repository, name: &str) -> Result<Option<String>, String> {
        match repo.find_reference(name) {
            Ok(reference) => {
                if reference
                    .symbolic_target()
                    .map_err(|e| e.to_string())?
                    .is_some()
                {
                    return Err(format!("Cannot finish using symbolic reference {name}."));
                }
                Ok(Some(
                    reference
                        .target()
                        .ok_or("Reference has no direct target.")?
                        .to_string(),
                ))
            }
            Err(error) if error.code() == git2::ErrorCode::NotFound => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    fn capture(path: &std::path::Path, epoch: u64, kind: Kind, name: &str) -> Result<Self, String> {
        let flow = crate::git::gitflow::config(path)
            .ok_or("Git Flow configuration changed; reopen Git Flow.")?;
        let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
        let source = format!("{}{name}", kind.prefix(&flow));
        if name.trim().is_empty() || source == flow.main || source == flow.develop {
            return Err("Choose a feature, release or hotfix branch to finish.".into());
        }
        let mut names = vec![
            format!("refs/heads/{source}"),
            format!("refs/heads/{}", flow.develop),
        ];
        if kind != Kind::Feature {
            names.push(format!("refs/heads/{}", flow.main));
        }
        let mut refs = Vec::new();
        for reference in names {
            let target = Self::reference(&repo, &reference)?
                .ok_or_else(|| format!("Required branch {reference} is missing."))?;
            refs.push((reference, Some(target)));
        }
        if kind != Kind::Feature {
            let reference = format!("refs/tags/{}{name}", flow.version_tag);
            if !git2::Reference::is_valid_name(&reference) {
                return Err("The version tag is invalid.".into());
            }
            refs.push((reference.clone(), Self::reference(&repo, &reference)?));
        }
        let head = repo
            .head()
            .map_err(|e| e.to_string())?
            .target()
            .ok_or("HEAD has no commit.")?
            .to_string();
        let head_ref = repo
            .find_reference("HEAD")
            .map_err(|e| e.to_string())?
            .symbolic_target()
            .map_err(|e| e.to_string())?
            .map(str::to_owned);
        Ok(Self {
            path: path.into(),
            epoch,
            flow,
            kind,
            name: name.into(),
            source,
            refs,
            head,
            head_ref,
        })
    }

    fn validate_session(&self, state: &crate::state::SharedState) -> Result<(), String> {
        let state = state.lock().unwrap();
        if state.repo_path.as_ref() != Some(&self.path) || state.repo_epoch != self.epoch {
            return Err("The repository session changed after Git Flow Finish was reviewed. Review it again.".into());
        }
        Ok(())
    }

    fn validate(
        &self,
        path: &std::path::Path,
        state: &crate::state::SharedState,
    ) -> Result<(), String> {
        self.validate_session(state)?;
        if path != self.path || crate::git::gitflow::config(path).as_ref() != Some(&self.flow) {
            return Err("Git Flow configuration changed after review. Review Finish again.".into());
        }
        let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
        for (reference, target) in &self.refs {
            if Self::reference(&repo, reference)? != *target {
                return Err(format!(
                    "{reference} changed after review. Review Finish again."
                ));
            }
        }
        let head = repo
            .head()
            .map_err(|e| e.to_string())?
            .target()
            .map(|oid| oid.to_string());
        let head_ref = repo
            .find_reference("HEAD")
            .map_err(|e| e.to_string())?
            .symbolic_target()
            .map_err(|e| e.to_string())?
            .map(str::to_owned);
        if head.as_deref() != Some(self.head.as_str()) || head_ref != self.head_ref {
            return Err(
                "HEAD or the checked-out branch changed after review. Review Finish again.".into(),
            );
        }
        self.validate_session(state)
    }
}

impl RecordeApp {
    /// Ask to confirm the next undo or redo step.
    pub(super) fn open_undo(&mut self, redo: bool) {
        let mut s = self.state.lock().unwrap();
        let step = if redo {
            s.redo_step.clone()
        } else {
            s.undo_step.clone()
        };
        match step {
            Some((id, label)) => s.modal = ModalKind::ConfirmUndo { id, label, redo },
            None => {
                s.error = Some(if redo {
                    "There is nothing to redo.".into()
                } else {
                    "There is nothing to undo yet.".into()
                })
            }
        }
    }

    /// Local branches first, then remote-tracking branches, excluding HEAD.
    pub(super) fn branch_choices(&self) -> Vec<String> {
        let s = self.state.lock().unwrap();
        let head = s.head_branch.clone().unwrap_or_default();
        let Some(graph) = &s.commit_graph else {
            return Vec::new();
        };
        let mut local: Vec<String> = graph
            .ref_targets
            .keys()
            .filter_map(|r| r.strip_prefix("refs/heads/"))
            .filter(|name| *name != head)
            .map(str::to_owned)
            .collect();
        let mut remote: Vec<String> = graph
            .ref_targets
            .keys()
            .filter_map(|r| r.strip_prefix("refs/remotes/"))
            .filter(|name| !name.ends_with("/HEAD"))
            .map(str::to_owned)
            .collect();
        local.sort();
        remote.sort();
        local.extend(remote);
        local
    }

    pub(super) fn render_merge_modal(&mut self, ctx: &Context, preset: Option<String>) {
        if let Some(branch) = preset {
            self.repo_ui.merge_choice = Some(branch);
            self.state.lock().unwrap().modal = ModalKind::MergeBranch { branch: None };
        }
        let choices = self.branch_choices();
        let head = self
            .state
            .lock()
            .unwrap()
            .head_branch
            .clone()
            .unwrap_or_else(|| "HEAD".into());
        let mut merge = false;
        let mut close = false;
        modal_window(ctx, "Merge Branch", |ui| {
            ui.set_min_width(420.0);
            ui.label(styled_label(&format!("Merge into {head}")));
            ui.add_space(6.0);
            egui::ScrollArea::vertical()
                .id_salt("merge_branch_list")
                .max_height(200.0)
                .show(ui, |ui| {
                    for name in &choices {
                        let selected = self.repo_ui.merge_choice.as_deref() == Some(name);
                        if ui.selectable_label(selected, name).clicked() {
                            self.repo_ui.merge_choice = Some(name.clone());
                        }
                    }
                    if choices.is_empty() {
                        ui.label(styled_muted("No other branches."));
                    }
                });
            ui.add_space(8.0);
            for mode in MergeMode::ALL {
                crate::ui::kit::radio_value(ui, &mut self.repo_ui.merge_mode, mode, mode.label());
            }
            if self.repo_ui.merge_mode.takes_message() {
                ui.add_space(4.0);
                ui.add(
                    egui::TextEdit::multiline(&mut self.repo_ui.merge_message)
                        .desired_width(f32::INFINITY)
                        .desired_rows(2)
                        .hint_text("Commit message (optional; Git's default is used when empty)"),
                );
            }
            if self.repo_ui.merge_mode == MergeMode::Squash {
                ui.label(styled_muted(
                    "Squash records the branch's changes as one new commit. The branch itself is \
                     not marked as merged, so Git will not delete it with a normal delete.",
                ));
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                merge = ui
                    .add_enabled(self.repo_ui.merge_choice.is_some(), primary_btn("Merge"))
                    .clicked();
                close = ui.add(secondary_btn("Cancel")).clicked()
                    || ui.input(|i| i.key_pressed(Key::Escape));
            });
        });
        if merge {
            if let Some(branch) = self.repo_ui.merge_choice.take() {
                let message = Some(std::mem::take(&mut self.repo_ui.merge_message))
                    .filter(|m| !m.trim().is_empty());
                crate::git::merge::merge_branch_with(
                    &branch,
                    self.repo_ui.merge_mode,
                    message,
                    self.state.clone(),
                );
            }
            self.state.lock().unwrap().modal = ModalKind::None;
        } else if close {
            self.repo_ui.merge_choice = None;
            self.state.lock().unwrap().modal = ModalKind::None;
        }
    }

    pub(super) fn render_git_flow_modal(&mut self, ctx: &Context) {
        if let Some(review) = self.repo_ui.flow_finish.clone() {
            let mut confirm = false;
            let mut cancel = false;
            modal_window(ctx, "Confirm Git Flow Finish", |ui| {
                ui.set_min_width(440.0);
                ui.label(styled_label(&format!(
                    "Finish {} {}?",
                    review.kind.label().to_lowercase(),
                    review.name
                )));
                ui.label(styled_muted(&format!(
                    "Repository: {}",
                    review.path.display()
                )));
                ui.label(format!("Source branch: {}", review.source));
                if review.kind == Kind::Feature {
                    ui.label(format!("Merge into {}.", review.flow.develop));
                } else {
                    ui.label(format!(
                        "Merge into {} and {}.",
                        review.flow.main, review.flow.develop
                    ));
                    let tag = format!("{}{}", review.flow.version_tag, review.name);
                    let exists = review
                        .refs
                        .last()
                        .is_some_and(|(_, target)| target.is_some());
                    ui.label(if exists {
                        format!("Reuse tag {tag} only if it matches this resumed finish.")
                    } else {
                        format!("Create annotated tag: {tag}")
                    });
                }
                ui.label(format!(
                    "Then check out {} and delete {} locally.",
                    review.flow.develop, review.source
                ));
                ui.label(styled_muted("Nothing is pushed. Git Flow Finish has no Undo step. A conflict or failure may leave completed steps in place; resolve the conflict and review Finish again to resume."));
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    confirm = ui.add(primary_btn("Confirm Finish")).clicked();
                    cancel = ui.add(secondary_btn("Cancel")).clicked()
                        || ui.input(|i| i.key_pressed(Key::Escape));
                });
            });
            if cancel {
                self.repo_ui.flow_finish = None;
            } else if confirm {
                self.repo_ui.flow_finish = None;
                {
                    let mut state = self.state.lock().unwrap();
                    state.modal = ModalKind::None;
                    state.remote_op = Some(format!(
                        "Finish {} {}",
                        review.kind.label().to_lowercase(),
                        review.name
                    ));
                }
                let state = self.state.clone();
                crate::git::mutation::spawn_operation(
                    review.path.clone(),
                    self.state.clone(),
                    move |path| {
                        review.validate(path, &state)?;
                        crate::git::gitflow::finish(path, review.kind, &review.name).map(|_| ())
                    },
                );
            }
            return;
        }
        let (path, flow, refs) = {
            let s = self.state.lock().unwrap();
            (
                s.repo_path.clone(),
                s.git_flow.clone(),
                s.commit_graph
                    .as_ref()
                    .map(|g| g.ref_targets.keys().cloned().collect::<Vec<_>>())
                    .unwrap_or_default(),
            )
        };
        let Some(path) = path else {
            self.state.lock().unwrap().modal = ModalKind::None;
            return;
        };
        let mut close = false;
        let mut finish = None;
        let mut job: Option<(
            String,
            Box<dyn FnOnce(&std::path::Path) -> Result<(), String> + Send>,
        )> = None;
        modal_window(ctx, "Git Flow", |ui| {
            ui.set_min_width(440.0);
            match &flow {
                None => {
                    ui.label(styled_muted(
                        "Git Flow keeps finished work on a production branch and ongoing work on a \
                         development branch. Settings are stored in this repository's Git config \
                         and work with the git-flow command-line tool.",
                    ));
                    ui.add_space(8.0);
                    let config = &mut self.repo_ui.flow_config;
                    settings_text_field(ui, "Production ", &mut config.main);
                    settings_text_field(ui, "Development", &mut config.develop);
                    settings_text_field(ui, "Feature    ", &mut config.feature);
                    settings_text_field(ui, "Release    ", &mut config.release);
                    settings_text_field(ui, "Hotfix     ", &mut config.hotfix);
                    settings_text_field(ui, "Version tag", &mut config.version_tag);
                    ui.add_space(8.0);
                    if ui.add(primary_btn("Initialize Git Flow")).clicked() {
                        let config = self.repo_ui.flow_config.clone();
                        job = Some((
                            "Initialize Git Flow".into(),
                            Box::new(move |repo| crate::git::gitflow::init(repo, &config)),
                        ));
                    }
                }
                Some(flow) => {
                    ui.label(styled_muted(&format!(
                        "Production: {}   Development: {}",
                        flow.main, flow.develop
                    )));
                    ui.add_space(8.0);
                    ui.label(styled_label("Start"));
                    ui.horizontal(|ui| {
                        for kind in Kind::ALL {
                            crate::ui::kit::radio_value(
                                ui,
                                &mut self.repo_ui.flow_kind,
                                kind,
                                kind.label(),
                            );
                        }
                    });
                    let kind = self.repo_ui.flow_kind;
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(kind.prefix(flow)).monospace());
                        ui.add(
                            crate::ui::kit::text_field(&mut self.repo_ui.flow_name)
                                .desired_width(220.0)
                                .hint_text(if kind == Kind::Feature {
                                    "name"
                                } else {
                                    "1.2.0"
                                }),
                        );
                        let can = !self.repo_ui.flow_name.trim().is_empty();
                        if ui.add_enabled(can, secondary_btn("Start")).clicked() {
                            let name = std::mem::take(&mut self.repo_ui.flow_name);
                            job = Some((
                                format!("Start {}", kind.label().to_lowercase()),
                                Box::new(move |repo| {
                                    crate::git::gitflow::start(repo, kind, &name).map(|_| ())
                                }),
                            ));
                        }
                    });
                    ui.label(styled_muted(&format!("Starts from {}.", kind.base(flow))));
                    ui.add_space(10.0);
                    ui.label(styled_label("In progress"));
                    let mut any = false;
                    for kind in Kind::ALL {
                        let prefix = format!("refs/heads/{}", kind.prefix(flow));
                        let mut names: Vec<String> = refs
                            .iter()
                            .filter_map(|r| r.strip_prefix(&prefix))
                            .filter(|n| !n.is_empty())
                            .map(str::to_owned)
                            .collect();
                        names.sort();
                        for name in names {
                            any = true;
                            ui.horizontal(|ui| {
                                ui.label(format!("{} {name}", kind.label()));
                                let target = if kind == Kind::Feature {
                                    flow.develop.clone()
                                } else {
                                    format!("{} and {}", flow.main, flow.develop)
                                };
                                if ui
                                    .button("Finish")
                                    .on_hover_text(format!(
                                        "Merge into {target}{}, then delete the branch",
                                        if kind == Kind::Feature {
                                            ""
                                        } else {
                                            ", tag it"
                                        }
                                    ))
                                    .clicked()
                                {
                                    finish = Some((kind, name.clone()));
                                }
                            });
                        }
                    }
                    if !any {
                        ui.label(styled_muted("No feature, release or hotfix branches."));
                    }
                }
            }
            ui.add_space(12.0);
            close = ui.add(secondary_btn("Close")).clicked()
                || ui.input(|i| i.key_pressed(Key::Escape));
        });
        if let Some((kind, name)) = finish {
            let epoch = self.state.lock().unwrap().repo_epoch;
            match FlowFinish::capture(&path, epoch, kind, &name) {
                Ok(review) => self.repo_ui.flow_finish = Some(review),
                Err(error) => self.state.lock().unwrap().error = Some(error),
            }
        } else if let Some((label, work)) = job {
            {
                let mut s = self.state.lock().unwrap();
                s.modal = ModalKind::None;
                s.remote_op = Some(label);
            }
            crate::git::mutation::spawn_operation(path, self.state.clone(), work);
        } else if close {
            self.state.lock().unwrap().modal = ModalKind::None;
        }
    }

    /// Prepare the Git Flow dialog with sensible branch names.
    pub(super) fn open_git_flow(&mut self) {
        self.repo_ui.flow_finish = None;
        let path = self.state.lock().unwrap().repo_path.clone();
        let mut config = FlowConfig::default();
        if let Some(repo) = path.and_then(|p| git2::Repository::open(p).ok()) {
            let exists = |name: &str| repo.find_branch(name, git2::BranchType::Local).is_ok();
            if !exists("main") && exists("master") {
                config.main = "master".into();
            }
        }
        self.repo_ui.flow_config = config;
        self.state.lock().unwrap().modal = ModalKind::GitFlow;
    }

    pub(super) fn open_squash(&mut self, oids: Vec<String>) {
        self.start_history_plan(super::history_plan::Request::Squash(oids));
    }

    pub(super) fn open_drop(&mut self, oids: Vec<String>) {
        self.start_history_plan(super::history_plan::Request::Drop(oids));
    }
    pub(super) fn render_init_options(&mut self, ui: &mut egui::Ui) {
        use crate::git::templates::{License, GITIGNORE};
        let options = &mut self.init_options;
        ui.horizontal(|ui| {
            ui.label(styled_label("Default branch"));
            ui.add(crate::ui::kit::text_field(&mut options.default_branch).desired_width(160.0));
        });
        egui::ComboBox::from_label(".gitignore template")
            .selected_text(
                options
                    .gitignore
                    .and_then(|i| GITIGNORE.get(i))
                    .map(|(name, _)| *name)
                    .unwrap_or("None"),
            )
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut options.gitignore, None, "None");
                for (index, (name, _)) in GITIGNORE.iter().enumerate() {
                    ui.selectable_value(&mut options.gitignore, Some(index), *name);
                }
            });
        egui::ComboBox::from_label("License")
            .selected_text(options.license.map(License::label).unwrap_or("None"))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut options.license, None, "None");
                for license in License::ALL {
                    ui.selectable_value(&mut options.license, Some(license), license.label());
                }
            });
        if options.license.is_some() {
            ui.horizontal(|ui| {
                ui.label(styled_label("Copyright holder"));
                ui.add(crate::ui::kit::text_field(&mut options.author).desired_width(220.0));
            });
        }
        ui.checkbox(&mut options.readme, "Add README.md");
        ui.checkbox(&mut options.initial_commit, "Commit the new files")
            .on_hover_text("Creates an initial commit containing only the generated files");
        ui.label(styled_muted("Existing files are never overwritten."));
    }

    pub(super) fn render_profiles(&mut self, ui: &mut egui::Ui) {
        use crate::ui::{kit, motion};

        kit::section(
            ui,
            "Profiles",
            "Switch between identities, for example work and personal. Profiles also choose \
             hosting accounts and can restore saved colours and external tools when used \
             everywhere.",
            |ui| {
                let repo = self.state.lock().unwrap().repo_path.clone();
                let (global_name, global_email) =
                    (self.git_name_input.clone(), self.git_email_input.clone());
                let mut apply: Option<(usize, bool)> = None;
                let mut remove: Option<usize> = None;
                let form_id = ui.make_persistent_id("profile_form");
                if self.config.profiles.is_empty() {
                    kit::hint(
                        ui,
                        "Save a profile below to reuse a work or personal identity.",
                    );
                }
                for (index, profile) in self.config.profiles.iter().enumerate() {
                    ui.push_id(&profile.name, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(&profile.name).strong());
                            if profile.matches(&global_name, &global_email) {
                                kit::status_chip(ui, kit::Status::Ok, "Global identity");
                            }
                        });
                        ui.add(
                            egui::Label::new(format!(
                                "{} <{}>",
                                profile.git_name, profile.git_email
                            ))
                            .wrap(),
                        );
                        kit::hint(
                            ui,
                            &format!(
                                "Applies commit identity and {} account choice(s){}{}.",
                                profile.provider_accounts.len(),
                                if profile.signing_key.trim().is_empty() {
                                    ""
                                } else {
                                    ", plus a signing key"
                                },
                                if profile.preferences.is_some() {
                                    "; saved colours and tools apply only with Use everywhere"
                                } else {
                                    ""
                                },
                            ),
                        );
                        let confirm_id = ui.make_persistent_id("delete_profile");
                        let mut confirm = motion::is_open(ui.ctx(), confirm_id, false);
                        ui.horizontal_wrapped(|ui| {
                            if ui
                                .add(kit::small_button("Use everywhere"))
                                .on_hover_text(
                                    "Apply this identity and account choices globally, plus any \
                                     saved colours and tools. Repository overrides still take \
                                     precedence.",
                                )
                                .clicked()
                            {
                                apply = Some((index, false));
                            }
                            if ui
                                .add_enabled(
                                    repo.is_some(),
                                    kit::small_button("Use for this repository"),
                                )
                                .on_hover_text(
                                    "Apply this identity and account choices only to the open \
                                     repository.",
                                )
                                .on_disabled_hover_text(
                                    "Open a repository to use this profile there.",
                                )
                                .clicked()
                            {
                                apply = Some((index, true));
                            }
                            if ui.add(kit::small_button("Edit")).clicked() {
                                self.profile_input = profile.clone();
                                motion::set_open(ui.ctx(), form_id, true);
                            }
                            if ui.add(kit::danger_outline_button("Delete…")).clicked() {
                                confirm = true;
                            }
                        });
                        motion::reveal(ui, confirm_id, confirm, |ui| {
                            kit::callout(
                                ui,
                                kit::Status::Warning,
                                &format!(
                                    "Delete profile “{}”? This removes the saved profile. Your \
                                     current Git identity, hosting accounts and SSH keys stay \
                                     in place.",
                                    profile.name,
                                ),
                            );
                            ui.horizontal_wrapped(|ui| {
                                if ui.add(kit::secondary_button("Cancel")).clicked() {
                                    confirm = false;
                                }
                                if ui.add(kit::danger_button("Delete profile")).clicked() {
                                    remove = Some(index);
                                    confirm = false;
                                }
                            });
                        });
                        motion::set_open(ui.ctx(), confirm_id, confirm);
                    });
                    kit::row_divider(ui);
                }
                if let Some((index, local)) = apply {
                    let profile = self.config.profiles[index].clone();
                    let target = if local { repo.as_deref() } else { None };
                    match profile
                        .validate_accounts(&self.config.accounts)
                        .and_then(|()| profile.apply(target))
                    {
                        Ok(()) => {
                            let mut updated = self.config.clone();
                            if let Some(path) = target {
                                updated
                                    .per_repo
                                    .entry(path.display().to_string())
                                    .or_default()
                                    .provider_accounts = profile.provider_accounts.clone();
                            } else {
                                updated.provider_accounts = profile.provider_accounts.clone();
                                if let Some(preferences) = &profile.preferences {
                                    preferences.apply(&mut updated);
                                }
                            }
                            self.load_git_identity_inputs();
                            self.state.lock().unwrap().git_user_initials =
                                theme::author_initials(&self.repo_ui.repo_git_name_input);
                            if let Err(error) = updated.try_save() {
                                self.profile_message = Some(format!(
                                    "Git identity applied, but account and appearance settings \
                                     could not be saved: {error}"
                                ));
                                return;
                            }
                            self.config = updated;
                            self.dark_theme = self.config.dark_theme;
                            self.high_contrast = self.config.high_contrast;
                            theme::apply_palette(
                                ui.ctx(),
                                self.dark_theme,
                                self.high_contrast,
                                self.config.theme_palette,
                            );
                            self.default_editor_input = self.config.default_editor.clone();
                            self.terminal_input = self.config.terminal.clone();
                            self.diff_tool_input = self.config.diff_tool.clone();
                            self.merge_tool_input = self.config.merge_tool.clone();
                            self.state.lock().unwrap().invalidate_provider_data();
                            for session in std::iter::once(&mut self.repo_ui)
                                .chain(self.repo_sessions.values_mut())
                            {
                                session.issues_fetched = false;
                                session.ci_fetched = false;
                                session.releases_fetched = false;
                            }
                            self.profile_message = Some(format!(
                                "Using {} {}.",
                                profile.name,
                                if local {
                                    "for this repository"
                                } else {
                                    "globally"
                                }
                            ));
                        }
                        Err(error) => self.profile_message = Some(error),
                    }
                }
                if let Some(index) = remove {
                    self.config.profiles.remove(index);
                    self.save_config();
                }

                kit::disclosure(ui, "profile_form", "Add or update a profile", false, |ui| {
                    kit::setting_row(
                        ui,
                        "Profile name",
                        "A label such as Work or Personal.",
                        |ui| {
                            ui.add(
                                crate::ui::kit::text_field(&mut self.profile_input.name)
                                    .desired_width(ui.available_width()),
                            );
                        },
                    );
                    kit::setting_row(
                        ui,
                        "Commit author name",
                        "Written into each new commit.",
                        |ui| {
                            ui.add(
                                crate::ui::kit::text_field(&mut self.profile_input.git_name)
                                    .desired_width(ui.available_width()),
                            );
                        },
                    );
                    kit::setting_row(
                        ui,
                        "Commit email",
                        "Use an email linked to your hosting account.",
                        |ui| {
                            ui.add(
                                crate::ui::kit::text_field(&mut self.profile_input.git_email)
                                    .desired_width(ui.available_width()),
                            );
                        },
                    );
                    kit::setting_row(
                        ui,
                        "Signing key ID (optional)",
                        "Identifies your commit signing key. Leave blank to keep the current key.",
                        |ui| {
                            ui.add(
                                crate::ui::kit::text_field(&mut self.profile_input.signing_key)
                                    .desired_width(ui.available_width()),
                            );
                        },
                    );
                    kit::row_divider(ui);
                    let hosts: std::collections::BTreeSet<_> = self
                        .config
                        .accounts
                        .iter()
                        .map(|account| account.host.clone())
                        .collect();
                    if !hosts.is_empty() {
                        kit::group_label(ui, "Hosting accounts in this profile");
                        kit::hint(
                            ui,
                            "Choose one account per host, or let Recorde choose automatically.",
                        );
                    }
                    for host in hosts {
                        let mut selected = self
                            .profile_input
                            .provider_accounts
                            .get(&host)
                            .cloned()
                            .unwrap_or_default();
                        kit::setting_row(ui, &host, "", |ui| {
                            egui::ComboBox::from_id_salt(("profile_account", &host))
                                .selected_text(if selected.is_empty() {
                                    "Automatic"
                                } else {
                                    &selected
                                })
                                .width(ui.available_width().min(260.0))
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut selected, String::new(), "Automatic");
                                    for account in self
                                        .config
                                        .accounts
                                        .iter()
                                        .filter(|account| account.host == host)
                                    {
                                        ui.selectable_value(
                                            &mut selected,
                                            account.username.clone(),
                                            &account.username,
                                        );
                                    }
                                });
                        });
                        if selected.is_empty() {
                            self.profile_input.provider_accounts.remove(&host);
                        } else {
                            self.profile_input.provider_accounts.insert(host, selected);
                        }
                    }
                    let mut preferences = self.profile_input.preferences.is_some();
                    if ui
                        .checkbox(
                            &mut preferences,
                            "Include current colours and external tools when used globally",
                        )
                        .changed()
                    {
                        self.profile_input.preferences = preferences
                            .then(|| crate::profiles::ProfilePreferences::capture(&self.config));
                    }
                    let capture_id = ui.make_persistent_id("capture_profile_preferences");
                    motion::reveal(ui, capture_id, preferences, |ui| {
                        if ui
                            .add(kit::small_button("Capture current preferences"))
                            .clicked()
                        {
                            self.profile_input.preferences =
                                Some(crate::profiles::ProfilePreferences::capture(&self.config));
                        }
                    });
                    ui.add_space(kit::space::SM);
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .add(kit::secondary_button("Fill from global identity"))
                            .clicked()
                        {
                            let (name, email) = git_identity(None);
                            self.profile_input.git_name = name;
                            self.profile_input.git_email = email;
                        }
                        if ui.add(kit::primary_button("Save profile")).clicked() {
                            match crate::profiles::upsert(
                                &mut self.config.profiles,
                                self.profile_input.clone(),
                            ) {
                                Ok(()) => {
                                    self.save_config();
                                    self.profile_message =
                                        Some(format!("Saved {}.", self.profile_input.name.trim()));
                                    self.profile_input = Default::default();
                                }
                                Err(error) => self.profile_message = Some(error),
                            }
                        }
                    });
                });
                let message_id = ui.make_persistent_id("profile_message");
                motion::reveal(ui, message_id, self.profile_message.is_some(), |ui| {
                    if let Some(message) = &self.profile_message {
                        kit::hint(ui, message);
                    }
                });
            },
        );
    }

    pub(super) fn render_ssh_keygen(&mut self, ui: &mut egui::Ui) {
        use crate::ui::{kit, motion};

        if let Some(task) = &self.ssh_task {
            match task.try_recv() {
                Ok(result) => {
                    self.ssh_task = None;
                    match result {
                        Ok(SshDone::Generated(path, key)) => {
                            self.ssh_message = Some(format!("Created {}.", path.display()));
                            self.ssh_public_key = Some((path, key));
                            self.diagnostics_pending = None;
                            self.open_settings(SettingsTab::Accounts);
                        }
                        Ok(SshDone::Message(message)) => self.ssh_message = Some(message),
                        Err(error) => self.ssh_message = Some(error),
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.ssh_task = None,
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(100));
                }
            }
        }
        let busy = self.ssh_task.is_some();
        kit::disclosure(ui, "ssh_keygen", "Create a new SSH key", false, |ui| {
            kit::group_label(ui, "1. Create the key");
            if self.ssh_key_path.is_empty() {
                self.ssh_key_path = crate::auth::ssh::suggested_key_path().display().to_string();
            }
            if self.ssh_key_comment.is_empty() {
                self.ssh_key_comment = self.git_email_input.clone();
            }
            ui.add_enabled_ui(!busy, |ui| {
                kit::setting_row(
                    ui,
                    "File name",
                    "Full path for the private key; a .pub file is also created.",
                    |ui| {
                        ui.add(
                            crate::ui::kit::text_field(&mut self.ssh_key_path)
                                .desired_width(ui.available_width()),
                        );
                    },
                );
                kit::setting_row(
                    ui,
                    "Comment",
                    "Usually your email, to help you recognize the key.",
                    |ui| {
                        ui.add(
                            crate::ui::kit::text_field(&mut self.ssh_key_comment)
                                .desired_width(ui.available_width()),
                        );
                    },
                );
                kit::setting_row(
                    ui,
                    "Passphrase (optional)",
                    "Encrypts the private key on disk. Recorde does not store this passphrase.",
                    |ui| {
                        ui.add(
                            crate::ui::kit::text_field(&mut self.ssh_key_passphrase)
                                .password(true)
                                .desired_width(ui.available_width()),
                        );
                    },
                );
            });
            kit::hint(
                ui,
                "Creates an Ed25519 key. Existing files are never replaced. A blank passphrase \
                 leaves the private key unencrypted.",
            );
            if ui
                .add_enabled(!busy, kit::primary_button("Create key"))
                .on_disabled_hover_text("Wait for the current SSH operation to finish.")
                .clicked()
            {
                let path = std::path::PathBuf::from(self.ssh_key_path.trim());
                let comment = self.ssh_key_comment.trim().to_string();
                let passphrase =
                    zeroize::Zeroizing::new(std::mem::take(&mut self.ssh_key_passphrase));
                self.ssh_task = Some(ssh_task(move || {
                    crate::auth::ssh::generate_key(&path, &comment, &passphrase)
                        .map(|key| SshDone::Generated(path, key))
                }));
            }
        });
        let key_id = ui.make_persistent_id("ssh_key_next_steps");
        motion::reveal(ui, key_id, self.ssh_public_key.is_some(), |ui| {
            if let Some((path, key)) = self.ssh_public_key.clone() {
                kit::row_divider(ui);
                kit::group_label(ui, "2. Add the public key to your hosting account");
                kit::hint(
                    ui,
                    "Copy this public key into your host's SSH settings, or upload it directly \
                     to a connected account below. Only the public key is shared.",
                );
                ui.add(
                    egui::Label::new(RichText::new(&key).monospace())
                        .wrap()
                        .selectable(true),
                );
                if ui.add(kit::secondary_button("Copy public key")).clicked() {
                    ui.ctx().copy_text(key.clone());
                }
                let accounts = self.config.accounts.clone();
                if accounts.is_empty() {
                    kit::hint(
                        ui,
                        "Open your hosting website's SSH settings and add the public key. \
                         Connect a hosting account above to see its setup links here.",
                    );
                }
                for account in accounts {
                    ui.vertical(|ui| {
                        kit::hint(
                            ui,
                            &format!(
                                "{} · {}@{}",
                                account.provider.label(),
                                account.username,
                                account.host
                            ),
                        );
                        ui.horizontal_wrapped(|ui| {
                            if crate::auth::ssh::can_upload(&account.provider) {
                                let label =
                                    format!("Upload to {}@{}", account.username, account.host);
                                if ui
                                    .add_enabled(!busy, kit::secondary_button(&label).truncate())
                                    .on_hover_text(
                                        "Upload this public key using the account's saved token. \
                                         The token needs permission to manage SSH keys.",
                                    )
                                    .on_disabled_hover_text(
                                        "Wait for the current SSH operation to finish.",
                                    )
                                    .clicked()
                                {
                                    let key = key.clone();
                                    let title = format!(
                                        "Recorde {}",
                                        path.file_name()
                                            .map(|n| n.to_string_lossy().into_owned())
                                            .unwrap_or_default()
                                    );
                                    let upload_account = account.clone();
                                    self.ssh_task = Some(ssh_task(move || {
                                        crate::auth::ssh::upload_key(&upload_account, &title, &key)
                                            .map(|_| {
                                                SshDone::Message(format!(
                                                    "Key added to {}.",
                                                    upload_account.label()
                                                ))
                                            })
                                    }));
                                }
                            }
                            if let Some(url) =
                                crate::auth::ssh::settings_url(&account.provider, &account.host)
                            {
                                let label =
                                    format!("Open {} SSH settings", account.provider.label());
                                if ui
                                    .add(kit::secondary_button(&label).truncate())
                                    .on_hover_text(
                                        "Open the host's SSH settings in your browser, then paste \
                                         the public key. Check that you are signed into the \
                                         intended account.",
                                    )
                                    .clicked()
                                {
                                    if let Err(error) = super::open_url(&url) {
                                        self.ssh_message = Some(error);
                                    }
                                }
                            }
                        });
                    });
                }
                kit::row_divider(ui);
                kit::group_label(ui, "3. Add to SSH agent");
                kit::hint(
                    ui,
                    "The SSH agent holds your key for Git to use. If you set a passphrase, \
                     add the key from a terminal so you can enter it; Recorde cannot ask for it.",
                );
                ui.add(
                    egui::Label::new(
                        RichText::new(format!("ssh-add \"{}\"", path.display())).monospace(),
                    )
                    .wrap()
                    .selectable(true),
                );
                if ui
                    .add_enabled(!busy, kit::secondary_button("Add to SSH agent"))
                    .on_hover_text(
                        "Load an unencrypted key into the running SSH agent. Use the terminal \
                         command above for a key with a passphrase.",
                    )
                    .on_disabled_hover_text("Wait for the current SSH operation to finish.")
                    .clicked()
                {
                    self.ssh_task = Some(ssh_task(move || {
                        crate::auth::ssh::add_to_agent(&path)
                            .map(|_| SshDone::Message("Key added to the SSH agent.".into()))
                    }));
                }
            }
        });
        let busy_id = ui.make_persistent_id("ssh_task_busy");
        motion::reveal(ui, busy_id, self.ssh_task.is_some(), |ui| {
            ui.horizontal(|ui| {
                ui.spinner();
                kit::hint(ui, "Working…");
            });
        });
        let message_id = ui.make_persistent_id("ssh_task_message");
        motion::reveal(ui, message_id, self.ssh_message.is_some(), |ui| {
            if let Some(message) = &self.ssh_message {
                kit::callout(ui, kit::Status::Info, message);
            }
        });
    }

    pub(super) fn show_terminal_panel(&mut self, root_ui: &mut egui::Ui) {
        let repo = self.state.lock().unwrap().repo_path.clone();
        let mut close = false;
        let mut finished = None;
        // Preserve a usable graph when enlarged text or a shorter window leaves
        // less room for the two optional bottom panels.
        let diff_reserve = if self.config.show_diff_panel {
            120.0
        } else {
            0.0
        };
        let max_height = (root_ui.available_height() - 180.0 - diff_reserve).clamp(140.0, 480.0);
        egui::Panel::bottom("terminal_panel")
            .resizable(true)
            .default_size(200.0_f32.min(max_height))
            .size_range(120.0..=max_height)
            .frame(egui::Frame::NONE.fill(theme::bg_panel()))
            .show(root_ui, |ui| {
                let r = ui.max_rect();
                // Keep overflow in a short window inside this panel.
                ui.shrink_clip_rect(r);
                ui.painter().line_segment(
                    [r.left_top(), r.right_top()],
                    egui::Stroke::new(1.0, theme::border()),
                );
                ui.add_space(4.0);
                (close, finished) = self.terminal.show(ui, repo.as_deref());
            });
        if close {
            self.config.show_terminal = false;
            self.save_config();
        }
        if finished.is_some() && finished == repo {
            crate::git::operations::refresh_repo(self.state.clone());
        }
    }

    pub(super) fn toggle_terminal(&mut self) {
        self.config.show_terminal = !self.config.show_terminal;
        if self.config.show_terminal {
            self.terminal.focus();
        }
        self.save_config();
    }

    pub(super) fn show_file_editor(&mut self, ctx: &Context) {
        // Modal layer state is from the previous frame. Keep the draft dormant
        // immediately while a lifecycle/history decision is pending as well.
        if self.pending_navigation.is_some() || self.quit_waiting || self.history_plan.is_some() {
            return;
        }
        let request = {
            let mut s = self.state.lock().unwrap();
            s.pending_file_edit.take().zip(s.repo_path.clone())
        };
        if let Some((file, repo)) = request {
            if self
                .repo_ui
                .file_editor
                .as_ref()
                .is_some_and(|editor| editor.is_dirty())
            {
                self.state.lock().unwrap().error =
                    Some("Save or close the file you are editing first.".into());
            } else {
                match crate::ui::file_editor::FileEditor::open(&repo, &file) {
                    Ok(editor) => self.repo_ui.file_editor = Some(editor),
                    Err(error) => self.state.lock().unwrap().error = Some(error),
                }
            }
        }
        if let Some(editor) = &mut self.repo_ui.file_editor {
            if editor.show(ctx, self.state.clone()) {
                self.repo_ui.file_editor = None;
            }
        }
    }
}

fn ssh_task(
    work: impl FnOnce() -> Result<SshDone, String> + Send + 'static,
) -> std::sync::mpsc::Receiver<Result<SshDone, String>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(work());
    });
    rx
}

#[cfg(test)]
mod tests {
    #[test]
    fn git_flow_finish_review_refuses_changed_targets_config_and_session() {
        use super::{FlowConfig, FlowFinish, Kind};
        let repo = crate::git::test_support::TestRepo::new();
        let base = repo.commit("file", b"base\n");
        let tip = repo.commit("file", b"tip\n");
        crate::git::gitflow::init(repo.path(), &FlowConfig::default()).unwrap();
        crate::git::gitflow::start(repo.path(), Kind::Release, "1.0").unwrap();
        let state = crate::state::new_shared_state();
        {
            let mut state = state.lock().unwrap();
            state.repo_path = Some(repo.path().into());
            state.repo_epoch = 7;
        }
        let review = FlowFinish::capture(repo.path(), 7, Kind::Release, "1.0").unwrap();
        review.validate(repo.path(), &state).unwrap();
        for reference in [
            "refs/heads/release/1.0",
            "refs/heads/main",
            "refs/heads/develop",
        ] {
            repo.git_ok(&["update-ref", reference, &base.to_string()]);
            assert!(
                review.validate(repo.path(), &state).is_err(),
                "changed {reference}"
            );
            repo.git_ok(&["update-ref", reference, &tip.to_string()]);
        }
        repo.git_ok(&["tag", "1.0", &tip.to_string()]);
        assert!(
            review.validate(repo.path(), &state).is_err(),
            "new tag after confirmation"
        );
        repo.git_ok(&["tag", "-d", "1.0"]);
        repo.git_ok(&["config", "gitflow.prefix.versiontag", "v"]);
        assert!(
            review.validate(repo.path(), &state).is_err(),
            "changed tag prefix"
        );
        repo.git_ok(&["config", "gitflow.prefix.versiontag", ""]);
        review.validate(repo.path(), &state).unwrap();
        repo.git_ok(&["checkout", "main"]);
        assert!(
            review.validate(repo.path(), &state).is_err(),
            "same tip on a different checked-out branch"
        );
        repo.git_ok(&["checkout", "release/1.0"]);
        state.lock().unwrap().repo_epoch += 1;
        assert!(
            review.validate(repo.path(), &state).is_err(),
            "reopened session"
        );
        assert_eq!(repo.head(), tip);
        assert_eq!(repo.index_bytes("file"), b"tip\n");
        assert_eq!(std::fs::read(repo.path().join("file")).unwrap(), b"tip\n");
    }

    #[test]
    fn git_flow_finish_review_keeps_existing_tag_object_identity() {
        use super::{FlowConfig, FlowFinish, Kind};
        let repo = crate::git::test_support::TestRepo::new();
        repo.commit("file", b"base\n");
        crate::git::gitflow::init(repo.path(), &FlowConfig::default()).unwrap();
        crate::git::gitflow::start(repo.path(), Kind::Release, "1.0").unwrap();
        repo.git_ok(&["tag", "-a", "-m", "first tag", "1.0"]);
        let state = crate::state::new_shared_state();
        state.lock().unwrap().repo_path = Some(repo.path().into());
        let review = FlowFinish::capture(repo.path(), 0, Kind::Release, "1.0").unwrap();
        review.validate(repo.path(), &state).unwrap();
        repo.git_ok(&["tag", "-f", "-a", "-m", "replacement tag", "1.0"]);
        assert!(
            review.validate(repo.path(), &state).is_err(),
            "even a tag replacement at the same commit needs fresh review"
        );
    }

    #[test]
    fn unsaved_file_edits_survive_repository_switches_and_block_exit() {
        let repo = crate::git::test_support::TestRepo::new();
        repo.commit("draft.txt", b"original\n");
        let mut active = super::super::RepoUiSession::default();
        let mut editor =
            crate::ui::file_editor::FileEditor::open(repo.path(), "draft.txt").unwrap();
        editor.text = "unsaved draft\n".into();
        active.file_editor = Some(editor);
        let mut inactive = std::collections::HashMap::new();
        assert_eq!(
            super::super::unsaved_file_edit(&active, inactive.values()),
            Some("draft.txt")
        );

        inactive.insert(repo.path().to_path_buf(), std::mem::take(&mut active));
        assert_eq!(
            super::super::unsaved_file_edit(&active, inactive.values()),
            Some("draft.txt")
        );
        let editor = inactive
            .get_mut(repo.path())
            .unwrap()
            .file_editor
            .as_mut()
            .unwrap();
        assert_eq!(editor.text, "unsaved draft\n");
        assert_eq!(
            std::fs::read(repo.path().join("draft.txt")).unwrap(),
            b"original\n"
        );
        editor.save().unwrap();
        assert_eq!(
            super::super::unsaved_file_edit(&active, inactive.values()),
            None
        );
        assert_eq!(
            std::fs::read(repo.path().join("draft.txt")).unwrap(),
            b"unsaved draft\n"
        );
    }
}
