use crate::ui::button::Button;
use egui::{Color32, RichText, Ui, Vec2};

use crate::git::{
    conflicts,
    operations::{load_blame, load_file_history, load_workdir_diff},
    staging,
};
use crate::state::{DiffContext, PendingDiscard, SharedState, WorkingDirFile};
use crate::theme;

pub struct StagingPanel {
    state: SharedState,
    drafts: super::commit_draft::Drafts,
    selections:
        std::collections::HashMap<std::path::PathBuf, std::collections::BTreeSet<(bool, String)>>,
    conflicts_expanded: bool,
    staged_expanded: bool,
    unstaged_expanded: bool,
    untracked_expanded: bool,
    commit_options_open: bool,
}

impl StagingPanel {
    pub fn new(state: SharedState) -> Self {
        Self {
            state,
            drafts: Default::default(),
            selections: Default::default(),
            conflicts_expanded: true,
            staged_expanded: true,
            unstaged_expanded: true,
            untracked_expanded: true,
            commit_options_open: false,
        }
    }

    pub fn show(&mut self, ui: &mut Ui) {
        let repo = self.state.lock().unwrap().repo_path.clone();
        self.drafts.switch_to(repo);
        self.drafts.poll();
        ui.painter()
            .rect_filled(ui.max_rect(), 0.0, theme::bg_surface());
        for (path, error) in &self.drafts.save_errors {
            ui.colored_label(
                theme::accent_red(),
                format!("Draft is only in memory ({}): {error}", path.display()),
            );
        }

        let (files, conflicted, head_branch, selected_file, repo_path, any_seq_in_progress) = {
            let s = self.state.lock().unwrap();
            let in_progress = s.capabilities.as_ref().map_or(false, |c| {
                c.merge_in_progress || c.cherry_pick_in_progress || c.revert_in_progress
            });
            (
                s.working_dir_files.clone(),
                s.conflicted_files.clone(),
                s.head_branch.clone().unwrap_or_default(),
                s.selected_file.clone(),
                s.repo_path.clone(),
                in_progress,
            )
        };

        // Partition files into three groups
        let staged: Vec<&WorkingDirFile> =
            files.iter().filter(|f| f.staged_status.is_some()).collect();
        let unstaged: Vec<&WorkingDirFile> = files
            .iter()
            .filter(|f| f.unstaged_status.is_some() && f.unstaged_status != Some('?'))
            .collect();
        let untracked: Vec<&WorkingDirFile> = files
            .iter()
            .filter(|f| f.unstaged_status == Some('?'))
            .collect();

        let selection = self
            .selections
            .entry(repo_path.clone().unwrap_or_default())
            .or_default();
        selection.retain(|(staged, path)| {
            files.iter().any(|file| {
                &file.path == path
                    && if *staged {
                        file.staged_status.is_some()
                    } else {
                        file.unstaged_status.is_some()
                    }
            })
        });
        let mut batch = None;
        if !selection.is_empty() {
            ui.horizontal_wrapped(|ui| {
                for (staged, label) in [(false, "Stage selected"), (true, "Unstage selected")] {
                    let paths: Vec<_> = selection
                        .iter()
                        .filter(|(side, _)| *side == staged)
                        .map(|(_, path)| path.clone())
                        .collect();
                    if ui
                        .add_enabled(
                            !paths.is_empty(),
                            Button::new(format!("{label} ({})", paths.len())),
                        )
                        .clicked()
                    {
                        batch = Some((paths, staged));
                    }
                }
                if ui.button("Clear selection").clicked() {
                    selection.clear();
                }
            });
        }

        // Actions collected during rendering, applied after
        let mut do_choose_ours: Option<String> = None;
        let mut do_choose_theirs: Option<String> = None;
        let mut do_mark_resolved: Option<String> = None;
        let mut do_open_mergetool: Option<String> = None;
        let mut do_stage_file: Option<String> = None;
        let mut do_stash_file: Option<String> = None;
        let mut do_unstage_file: Option<String> = None;
        let mut do_stage_all = false;
        let mut do_unstage_all = false;
        let mut do_commit = false;
        let mut do_amend = false;
        let mut view_file: Option<(String, DiffContext)> = None;
        let mut do_discard_file: Option<String> = None;
        let mut do_ignore_file: Option<String> = None;

        let list_height = (ui.available_height()
            - if self.commit_options_open {
                400.0
            } else {
                300.0
            })
        .max(120.0);
        egui::ScrollArea::vertical()
            .id_salt("changes_file_list")
            .max_height(list_height)
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                ui.add_space(8.0);

                // ── CONFLICTS ────────────────────────────────────────────
                if !conflicted.is_empty() {
                    let conflicts_expanded = &mut self.conflicts_expanded;
                    ui.horizontal(|ui| {
                        ui.add_space(8.0);
                        let arrow = if *conflicts_expanded { "▼" } else { "▶" };
                        let toggle = ui.add(
                            Button::new(
                                RichText::new(format!(
                                    "{} Conflicts ({})",
                                    arrow,
                                    conflicted.len()
                                ))
                                .color(theme::accent_red())
                                .size(12.0)
                                .strong(),
                            )
                            .fill(Color32::TRANSPARENT)
                            .stroke(egui::Stroke::NONE),
                        );
                        if toggle.clicked() {
                            *conflicts_expanded = !*conflicts_expanded;
                        }
                    });
                    ui.add_space(2.0);

                    if self.conflicts_expanded {
                        for cf in &conflicted {
                            let is_sel = selected_file.as_deref() == Some(cf.path.as_str());
                            let resp = conflict_file_row(ui, &cf.path, is_sel, |ui| {
                                if ui
                                    .add(small_btn_danger(if cf.ours_deleted {
                                        "Delete (ours)"
                                    } else {
                                        "Ours"
                                    }))
                                    .clicked()
                                {
                                    do_choose_ours = Some(cf.path.clone());
                                }
                                if ui
                                    .add(small_btn_danger(if cf.theirs_deleted {
                                        "Delete (theirs)"
                                    } else {
                                        "Theirs"
                                    }))
                                    .clicked()
                                {
                                    do_choose_theirs = Some(cf.path.clone());
                                }
                                if ui.add(small_btn("Resolved")).clicked() {
                                    do_mark_resolved = Some(cf.path.clone());
                                }
                            });
                            resp.context_menu(|ui| {
                                if ui.button("Resolve in Recorde...").clicked() {
                                    self.state.lock().unwrap().pending_conflict_editor =
                                        Some(cf.path.clone());
                                    ui.close();
                                }
                                if ui.button("Open Merge Tool").clicked() {
                                    do_open_mergetool = Some(cf.path.clone());
                                    ui.close();
                                }
                                if ui.button("Reveal in Folder").clicked() {
                                    reveal_in_folder(&cf.path, &repo_path);
                                    ui.close();
                                }
                            });
                            if resp.clicked() {
                                view_file = Some((cf.path.clone(), DiffContext::UnstagedFile));
                            }
                        }
                    }
                    ui.add_space(6.0);
                }

                // ── UNSTAGED CHANGES ─────────────────────────────────────
                let unstaged_expanded = &mut self.unstaged_expanded;
                collapsible_section(ui, "Unstaged", unstaged.len(), unstaged_expanded, |ui| {
                    if ui
                        .add_enabled(!unstaged.is_empty(), small_btn("Stage All"))
                        .clicked()
                    {
                        do_stage_all = true;
                    }
                });

                if self.unstaged_expanded {
                    if unstaged.is_empty() {
                        empty_label(ui, "Nothing to stage.");
                    } else {
                        for f in &unstaged {
                            let is_sel = selected_file.as_deref() == Some(f.path.as_str());
                            let resp = file_row(
                                ui,
                                &f.path,
                                f.unstaged_status.unwrap_or('M'),
                                is_sel,
                                selection,
                                false,
                                |ui| {
                                    let btn = if is_sel {
                                        small_btn_accent("Stage")
                                    } else {
                                        small_btn("Stage")
                                    };
                                    if ui.add(btn).clicked() {
                                        do_stage_file = Some(f.path.clone());
                                    }
                                    if ui.add(small_btn_danger("Discard")).clicked() {
                                        do_discard_file = Some(f.path.clone());
                                    }
                                },
                            );
                            resp.context_menu(|ui| {
                                if ui.button("Show File History").clicked() {
                                    load_file_history(&f.path, self.state.clone());
                                    ui.close();
                                }
                                if ui.button("Blame").clicked() {
                                    {
                                        let mut s = self.state.lock().unwrap();
                                        s.selected_file = Some(f.path.clone());
                                        s.show_blame = true;
                                    }
                                    load_blame(&f.path, self.state.clone());
                                    ui.close();
                                }
                                ui.separator();
                                if ui.button("Stash This File").clicked() {
                                    do_stash_file = Some(f.path.clone());
                                    ui.close();
                                }
                                ui.separator();
                                if ui.button("Open in Editor").clicked() {
                                    open_file_in_editor(&f.path, &repo_path);
                                    ui.close();
                                }
                                if ui.button("Reveal in Folder").clicked() {
                                    reveal_in_folder(&f.path, &repo_path);
                                    ui.close();
                                }
                            });
                            if resp.clicked() {
                                view_file = Some((f.path.clone(), DiffContext::UnstagedFile));
                            }
                        }
                    }
                }

                ui.add_space(6.0);

                // ── STAGED CHANGES ───────────────────────────────────────
                let staged_expanded = &mut self.staged_expanded;
                collapsible_section(ui, "Staged", staged.len(), staged_expanded, |ui| {
                    if ui
                        .add_enabled(!staged.is_empty(), small_btn("Unstage All"))
                        .clicked()
                    {
                        do_unstage_all = true;
                    }
                });

                if self.staged_expanded {
                    if staged.is_empty() {
                        empty_label(ui, "Nothing staged for commit.");
                    } else {
                        for f in &staged {
                            let is_sel = selected_file.as_deref() == Some(f.path.as_str());
                            let resp = file_row(
                                ui,
                                &f.path,
                                f.staged_status.unwrap_or('M'),
                                is_sel,
                                selection,
                                true,
                                |ui| {
                                    let btn = if is_sel {
                                        small_btn_accent("Unstage")
                                    } else {
                                        small_btn("Unstage")
                                    };
                                    if ui.add(btn).clicked() {
                                        do_unstage_file = Some(f.path.clone());
                                    }
                                },
                            );
                            resp.context_menu(|ui| {
                                if ui.button("Open in Editor").clicked() {
                                    open_file_in_editor(&f.path, &repo_path);
                                    ui.close();
                                }
                                if ui.button("Reveal in Folder").clicked() {
                                    reveal_in_folder(&f.path, &repo_path);
                                    ui.close();
                                }
                            });
                            if resp.clicked() {
                                view_file = Some((f.path.clone(), DiffContext::StagedFile));
                            }
                        }
                    }
                }

                ui.add_space(6.0);

                // ── UNTRACKED FILES ──────────────────────────────────────
                let untracked_expanded = &mut self.untracked_expanded;
                collapsible_section(
                    ui,
                    "Untracked",
                    untracked.len(),
                    untracked_expanded,
                    |_ui| {},
                );

                if self.untracked_expanded {
                    if untracked.is_empty() {
                        empty_label(ui, "No untracked files.");
                    } else {
                        for f in &untracked {
                            let is_sel = selected_file.as_deref() == Some(f.path.as_str());
                            let resp = file_row(ui, &f.path, '?', is_sel, selection, false, |ui| {
                                if ui.add(small_btn("Stage")).clicked() {
                                    do_stage_file = Some(f.path.clone());
                                }
                                if ui.add(small_btn("Ignore")).clicked() {
                                    do_ignore_file = Some(f.path.clone());
                                }
                            });
                            resp.context_menu(|ui| {
                                if ui.button("Reveal in Folder").clicked() {
                                    reveal_in_folder(&f.path, &repo_path);
                                    ui.close();
                                }
                                if ui.button("Add to .gitignore").clicked() {
                                    do_ignore_file = Some(f.path.clone());
                                    ui.close();
                                }
                            });
                            if resp.clicked() {
                                view_file = Some((f.path.clone(), DiffContext::UnstagedFile));
                            }
                        }
                    }
                }

                ui.add_space(10.0);
                ui.add(egui::Separator::default().spacing(0.0));
                ui.add_space(8.0);
            });
        egui::ScrollArea::vertical().id_salt("commit_composer").auto_shrink([false; 2]).show(ui, |ui| {
                // Hint when a sequencing op is in progress but all conflicts resolved
                if any_seq_in_progress && conflicted.is_empty() {
                    ui.horizontal(|ui| {
                        ui.add_space(12.0);
                        ui.label(RichText::new("✓ All conflicts resolved — stage remaining changes, then click Continue in the banner above.")
                            .color(theme::accent_green()).size(12.0));
                    });
                    ui.add_space(6.0);
                }

                // ── COMMIT FORM ──────────────────────────────────────────

                // Load commit template on first open
                if !self.drafts.current.template_loaded && self.drafts.current.subject.is_empty() && self.drafts.current.body.is_empty() && !self.drafts.current.is_amend {
                    self.drafts.current.template_loaded = true;
                    if let Some(ref rp) = repo_path {
                        if let Some(tmpl) = load_commit_template(rp) {
                            let mut lines = tmpl.lines().filter(|l| !l.trim_start().starts_with('#'));
                            self.drafts.current.subject = lines.next().unwrap_or("").trim().to_string();
                            let rest: String = lines.skip_while(|l| l.is_empty()).collect::<Vec<_>>().join("\n");
                            self.drafts.current.body = rest.trim().to_string();
                        }
                    }
                }

                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.label(RichText::new("Create a commit").color(theme::text_primary()).size(14.0).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(12.0);
                        if ui.add(small_btn("Copy")).on_hover_text("Copy assembled commit message").clicked() {
                            let msg = build_message(&self.drafts.current.subject, &self.drafts.current.body, &self.drafts.current.co_authors,
                                self.drafts.current.signoff_enabled, &repo_path);
                            ui.ctx().copy_text(msg);
                        }
                        ui.add_space(4.0);
                        let amend_color = if self.drafts.current.is_amend { theme::accent_yellow() } else { theme::text_muted() };
                        if ui.add(
                            Button::new(RichText::new("Amend").color(amend_color).size(12.0))
                                .fill(if self.drafts.current.is_amend { theme::bg_selected() } else { Color32::TRANSPARENT })
                                .stroke(egui::Stroke::new(1.0, if self.drafts.current.is_amend { theme::accent_yellow() } else { theme::border() })),
                        ).clicked() {
                            self.drafts.current.is_amend = !self.drafts.current.is_amend;
                            if self.drafts.current.is_amend && self.drafts.current.subject.is_empty() {
                                if let Some(msg) = staging::load_head_message(self.state.clone()) {
                                    let mut lines = msg.lines();
                                    self.drafts.current.subject = lines.next().unwrap_or("").to_string();
                                    let rest: String = lines.skip_while(|l| l.is_empty()).collect::<Vec<_>>().join("\n");
                                    self.drafts.current.body = rest;
                                }
                            }
                        }
                    });
                });

                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.add_sized(
                        [form_width(ui, 24.0, 480.0), 28.0],
                        egui::TextEdit::singleline(&mut self.drafts.current.subject)
                            .margin(egui::Vec2::new(8.0, 5.0))
                            .hint_text("What changed? (required)")
                            .font(egui::FontId::proportional(13.0)),
                    );
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.add_sized([form_width(ui, 24.0, 480.0), 60.0],
                        egui::TextEdit::multiline(&mut self.drafts.current.body)
                            .desired_width(form_width(ui, 24.0, 480.0))
                            .desired_rows(3)
                            .hint_text("Description (optional)")
                            .margin(egui::Vec2::new(8.0, 6.0))
                            .font(egui::FontId::proportional(13.0)),
                    );
                    ui.add_space(12.0);
                });

                let options = egui::CollapsingHeader::new("Commit options").id_salt("commit_options").show(ui, |ui| {
                // ── Co-author trailers ────────────────────────────────────
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.add_sized(
                        [form_width(ui, 68.0, 190.0), 28.0],
                        egui::TextEdit::singleline(&mut self.drafts.current.co_author_input)
                            .margin(egui::Vec2::new(8.0, 5.0))
                            .hint_text("Co-author: Name <email>")
                            .font(egui::FontId::proportional(12.0)),
                    );
                    ui.add_space(4.0);
                    if ui.add(small_btn("+ Add")).clicked() && !self.drafts.current.co_author_input.trim().is_empty() {
                        self.drafts.current.co_authors.push(self.drafts.current.co_author_input.trim().to_string());
                        self.drafts.current.co_author_input.clear();
                    }
                });
                let mut remove_co_author: Option<usize> = None;
                for (i, author) in self.drafts.current.co_authors.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.add_space(12.0);
                        ui.label(RichText::new(format!("Co-Authored-By: {}", author))
                            .color(theme::text_muted()).size(12.0).monospace());
                        if ui.small_button(RichText::new("×").color(theme::text_muted())).clicked() {
                            remove_co_author = Some(i);
                        }
                    });
                }
                if let Some(i) = remove_co_author { self.drafts.current.co_authors.remove(i); }

                // ── Sign-off + copy ──────────────────────────────────────
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.checkbox(&mut self.drafts.current.signoff_enabled,
                        RichText::new("Sign off").color(theme::text_muted()).size(12.0));
                    if ui.small_button("Identity / signing").clicked() {
                        self.state.lock().unwrap().pending_commit_settings = true;
                    }
                });

                ui.add_space(8.0);

                });
                self.commit_options_open = options.body_returned.is_some();
                // ── Branch protection warning ─────────────────────────────
                let is_protected = ["main", "master", "production", "release"]
                    .iter().any(|p| *p == head_branch.as_str());
                if is_protected && !self.drafts.current.is_amend {
                    ui.horizontal(|ui| {
                        ui.add_space(12.0);
                        ui.label(RichText::new(format!(
                            "⚠ Committing directly to \"{}\" — make sure this is intentional.",
                            head_branch
                        )).color(theme::accent_yellow()).size(12.0));
                    });
                    ui.add_space(4.0);
                }

                let can_commit = !self.drafts.current.subject.trim().is_empty()
                    && (!staged.is_empty() || self.drafts.current.is_amend)
                    && !self.drafts.busy() && conflicted.is_empty();
                let compact_commit_label = ui.available_width() < 280.0;
                let btn_label = if compact_commit_label && self.drafts.current.is_amend {
                    "Amend commit".to_string()
                } else if compact_commit_label {
                    "Commit".to_string()
                } else if self.drafts.current.is_amend {
                    format!("Amend commit on {}", head_branch)
                } else {
                    format!("Commit to {}", head_branch)
                };
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    let btn = ui.add_enabled(
                        can_commit,
                        Button::new(RichText::new(&btn_label).color(if can_commit { theme::on_primary() } else { theme::text_muted() }).size(14.0).strong())
                            .fill(if can_commit { theme::primary_fill() } else { theme::bg_elevated() })
                            .min_size(Vec2::new(form_width(ui, 24.0, 480.0), 38.0)),
                    );
                    if btn.clicked() {
                        if self.drafts.current.is_amend { do_amend = true; } else { do_commit = true; }
                    }
                });

                ui.add_space(6.0);
                egui::CollapsingHeader::new("Undo last commit").id_salt("reset_last_commit").show(ui, |ui| {
                // ── Reset last commit ─────────────────────────────────────
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.label(RichText::new("Reset last commit:").color(theme::text_muted()).size(12.0));
                    ui.add_space(4.0);
                    if ui.add(small_btn("Soft")).on_hover_text("Keep changes staged").clicked() {
                        staging::reset_soft(self.state.clone());
                    }
                    if ui.add(small_btn("Mixed")).on_hover_text("Keep changes unstaged").clicked() {
                        staging::reset_mixed(self.state.clone());
                    }
                    if ui.add(small_btn_danger("Hard")).on_hover_text("Reset HEAD and discard tracked working changes; confirmation required").clicked() {
                        self.state.lock().unwrap().pending_discard = Some(PendingDiscard::ResetHard);
                    }
                });
                });
                ui.add_space(16.0);
            });

        // ── Apply actions ────────────────────────────────────────────────
        if let Some(path) = do_stash_file {
            crate::git::branches::stash_file(&path, self.state.clone());
        }
        if let Some(path) = do_choose_ours {
            conflicts::choose_ours(&path, self.state.clone());
        }
        if let Some(path) = do_choose_theirs {
            conflicts::choose_theirs(&path, self.state.clone());
        }
        if let Some(path) = do_mark_resolved {
            conflicts::mark_resolved(&path, self.state.clone());
        }
        if let Some(path) = do_open_mergetool {
            if let Some(ref rp) = repo_path {
                conflicts::open_mergetool(&path, rp, self.state.clone());
            }
        }
        if let Some(path) = do_stage_file {
            staging::stage_file(&path, self.state.clone());
        }
        if let Some(path) = do_unstage_file {
            staging::unstage_file(&path, self.state.clone());
        }
        if let Some((paths, reverse)) = batch {
            staging::stage_files(paths, reverse, self.state.clone());
        }
        if do_stage_all {
            staging::stage_all(self.state.clone());
        }
        if do_unstage_all {
            staging::unstage_all(self.state.clone());
        }

        if let Some(path) = do_discard_file {
            self.state.lock().unwrap().pending_discard = Some(PendingDiscard::File { path });
        }

        if let Some(path) = do_ignore_file {
            staging::ignore_file(&path, self.state.clone());
        }

        if do_commit || do_amend {
            let full_msg = build_message(
                &self.drafts.current.subject,
                &self.drafts.current.body,
                &self.drafts.current.co_authors,
                self.drafts.current.signoff_enabled,
                &repo_path,
            );
            let completion = if do_commit {
                staging::create_commit(
                    &full_msg,
                    self.drafts.current.signoff_enabled,
                    self.state.clone(),
                )
            } else {
                staging::amend_commit(
                    &full_msg,
                    self.drafts.current.signoff_enabled,
                    self.state.clone(),
                )
            };
            if let Some(completion) = completion {
                self.drafts.submitted(completion);
            }
        }

        if let Some((path, context)) = view_file {
            {
                let mut s = self.state.lock().unwrap();
                s.selected_file = Some(path.clone());
                s.current_diff = None;
                s.diff_context = context;
            }
            let staged = context == DiffContext::StagedFile;
            load_workdir_diff(&path, staged, self.state.clone());
        }
    }
}

// ── Commit message helpers ────────────────────────────────────────────────────

/// Assemble the full commit message with trailers.
fn build_message(
    subject: &str,
    body: &str,
    co_authors: &[String],
    signoff: bool,
    repo_path: &Option<std::path::PathBuf>,
) -> String {
    let mut msg = subject.trim().to_string();
    if !body.trim().is_empty() {
        msg.push_str("\n\n");
        msg.push_str(body.trim());
    }
    // Trailers
    let mut trailers: Vec<String> = co_authors
        .iter()
        .map(|a| format!("Co-Authored-By: {}", a))
        .collect();
    if signoff {
        // Read full name + email from git config for the sign-off line
        let identity = repo_path
            .as_ref()
            .and_then(|p| git_identity_for_signoff(p))
            .or_else(|| git_identity_for_signoff(std::path::Path::new(".")));
        if let Some((name, email)) = identity {
            trailers.push(format!("Signed-off-by: {} <{}>", name, email));
        }
    }
    if !trailers.is_empty() {
        msg.push_str("\n\n");
        msg.push_str(&trailers.join("\n"));
    }
    msg
}

fn git_identity_for_signoff(repo_path: &std::path::Path) -> Option<(String, String)> {
    let config = git2::Repository::open(repo_path)
        .ok()?
        .config()
        .ok()
        .or_else(|| git2::Config::open_default().ok())?;
    let name = config.get_string("user.name").ok()?;
    let email = config.get_string("user.email").ok()?;
    Some((name, email))
}

fn load_commit_template(repo_path: &std::path::Path) -> Option<String> {
    let config = git2::Repository::open(repo_path).ok()?.config().ok()?;
    let tmpl_path = config.get_string("commit.template").ok()?;
    let path = if tmpl_path.starts_with('~') {
        let home = std::env::var("HOME").ok()?;
        std::path::PathBuf::from(tmpl_path.replacen('~', &home, 1))
    } else {
        std::path::PathBuf::from(&tmpl_path)
    };
    std::fs::read_to_string(path).ok()
}

// ── Shell helpers ─────────────────────────────────────────────────────────────

fn open_file_in_editor(file_path: &str, repo_path: &Option<std::path::PathBuf>) {
    let Some(rp) = repo_path else { return };
    let full = rp.join(file_path);
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| {
            #[cfg(target_os = "windows")]
            {
                "notepad".into()
            }
            #[cfg(not(target_os = "windows"))]
            {
                "xdg-open".into()
            }
        });
    let mut parts = editor.split_whitespace();
    let Some(prog) = parts.next() else { return };
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd")
        .args(["/C", prog])
        .args(parts)
        .arg(full)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    #[cfg(not(target_os = "windows"))]
    let _ = std::process::Command::new(prog)
        .args(parts)
        .arg(full)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

fn reveal_in_folder(file_path: &str, repo_path: &Option<std::path::PathBuf>) {
    let Some(rp) = repo_path else { return };
    let dir = rp.join(file_path);
    let dir = dir.parent().unwrap_or(rp.as_path());
    let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn collapsible_section(
    ui: &mut Ui,
    title: &str,
    count: usize,
    expanded: &mut bool,
    add_right: impl FnOnce(&mut Ui),
) {
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        let arrow = if *expanded { "▼" } else { "▶" };
        let toggle = ui.add(
            Button::new(
                RichText::new(format!("{} {}  ({})", arrow, title, count))
                    .color(theme::text_secondary())
                    .size(13.0)
                    .strong(),
            )
            .fill(Color32::TRANSPARENT)
            .stroke(egui::Stroke::NONE),
        );
        if toggle.clicked() {
            *expanded = !*expanded;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(12.0);
            add_right(ui);
        });
    });
    ui.add_space(2.0);
}

fn file_row(
    ui: &mut Ui,
    path: &str,
    status: char,
    is_selected: bool,
    selection: &mut std::collections::BTreeSet<(bool, String)>,
    staged: bool,
    add_btns: impl FnOnce(&mut Ui),
) -> egui::Response {
    let row_resp = ui
        .horizontal(|ui| {
            ui.add_space(4.0);
            let key = (staged, path.to_owned());
            let mut checked = selection.contains(&key);
            if ui
                .checkbox(&mut checked, "")
                .on_hover_text(format!("Select {path} for batch staging"))
                .changed()
            {
                if checked {
                    selection.insert(key);
                } else {
                    selection.remove(&key);
                }
            }
            let (icon, color) = status_icon(status);
            ui.label(
                RichText::new(icon)
                    .font(egui::FontId::monospace(12.0))
                    .color(color),
            )
            .on_hover_text(status_tooltip(status));
            ui.add_space(4.0);
            let name = path.rsplit('/').next().unwrap_or(path);
            let width = (ui.available_width() - 64.0).max(60.0);
            let label = ui
                .add_sized(
                    [width, 32.0],
                    Button::new("")
                        .fill(if is_selected {
                            theme::bg_selected()
                        } else {
                            Color32::TRANSPARENT
                        })
                        .stroke(egui::Stroke::NONE),
                )
                .on_hover_text(path);
            let mut job = egui::text::LayoutJob::simple(
                name.into(),
                egui::FontId::proportional(13.0),
                theme::text_primary(),
                (width - 16.0).max(20.0),
            );
            job.wrap.max_rows = 1;
            job.wrap.break_anywhere = true;
            let galley = ui.painter().layout_job(job);
            ui.painter().with_clip_rect(label.rect).galley(
                egui::pos2(
                    label.rect.left() + 8.0,
                    label.rect.center().y - galley.size().y / 2.0,
                ),
                galley,
                theme::text_primary(),
            );
            label.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), path)
            });
            ui.menu_button(crate::icons::rich(crate::icons::MORE_VERT, 18.0), |ui| {
                add_btns(ui);
            });
            label
        })
        .inner;

    row_resp
}

fn small_btn(label: &str) -> Button<'_> {
    Button::new(RichText::new(label).color(theme::text_muted()).size(12.0))
        .fill(Color32::TRANSPARENT)
        .stroke(egui::Stroke::new(1.0, theme::border()))
        .min_size(egui::Vec2::new(0.0, 22.0))
}

fn small_btn_accent(label: &str) -> Button<'_> {
    Button::new(
        RichText::new(label)
            .color(theme::on_primary())
            .size(12.0)
            .strong(),
    )
    .fill(theme::primary_fill())
    .stroke(egui::Stroke::new(1.0, theme::accent_teal()))
    .min_size(egui::Vec2::new(0.0, 22.0))
}

fn small_btn_danger(label: &str) -> Button<'_> {
    Button::new(RichText::new(label).color(theme::accent_red()).size(12.0))
        .fill(Color32::TRANSPARENT)
        .stroke(egui::Stroke::new(
            1.0,
            theme::accent_red().gamma_multiply(0.5),
        ))
        .min_size(egui::Vec2::new(0.0, 22.0))
}

fn status_icon(status: char) -> (&'static str, Color32) {
    match status {
        'A' | '?' => ("A", theme::accent_green()),
        'D' => ("D", theme::accent_red()),
        'R' => ("R", theme::accent_yellow()),
        _ => ("M", theme::accent_yellow()),
    }
}

fn status_tooltip(status: char) -> &'static str {
    match status {
        'A' => "Added",
        '?' => "Untracked",
        'D' => "Deleted",
        'R' => "Renamed",
        _ => "Modified",
    }
}

fn conflict_file_row(
    ui: &mut Ui,
    path: &str,
    is_selected: bool,
    add_btns: impl FnOnce(&mut Ui),
) -> egui::Response {
    let row_resp = ui
        .horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(
                RichText::new("C")
                    .font(egui::FontId::monospace(12.0))
                    .color(theme::accent_red())
                    .strong(),
            )
            .on_hover_text("Conflicted");
            ui.add_space(4.0);
            let name = path.rsplit('/').next().unwrap_or(path);
            let label = ui.selectable_label(
                is_selected,
                RichText::new(name)
                    .color(if is_selected {
                        theme::text_primary()
                    } else {
                        theme::accent_red().gamma_multiply(0.85)
                    })
                    .size(13.0),
            );
            let label = label.on_hover_text(path);
            if ui.available_width() > 170.0 {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(8.0);
                    add_btns(ui);
                });
            }
            label
        })
        .inner;
    row_resp
}

fn empty_label(ui: &mut Ui, text: &str) {
    ui.horizontal(|ui| {
        ui.add_space(20.0);
        ui.label(RichText::new(text).color(theme::text_muted()).size(12.0));
    });
    ui.add_space(2.0);
}

fn available_width_after(ui: &Ui, reserved: f32) -> f32 {
    (ui.available_width() - reserved).max(80.0)
}

fn form_width(ui: &Ui, reserved: f32, max_width: f32) -> f32 {
    available_width_after(ui, reserved).min(max_width)
}
