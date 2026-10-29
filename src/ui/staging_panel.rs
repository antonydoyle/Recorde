use crate::ui::button::Button;
use egui::{Color32, RichText, Ui, Vec2};

use crate::git::branches::ResetMode;
use crate::git::{
    conflicts,
    operations::{load_blame, load_file_history, load_workdir_diff},
    staging,
};
use crate::state::{DiffContext, PendingDiscard, SharedState, WorkingDirFile};
use crate::theme;
use crate::ui::{kit, motion};

pub struct StagingPanel {
    state: SharedState,
    drafts: super::commit_draft::Drafts,
    selections:
        std::collections::HashMap<std::path::PathBuf, std::collections::BTreeSet<(bool, String)>>,
    conflicts_expanded: bool,
    staged_expanded: bool,
    unstaged_expanded: bool,
    untracked_expanded: bool,
    /// Natural composer height measured on the previous pass.
    composer_height: Option<f32>,
    composer_footer_height: Option<f32>,
    #[cfg(test)]
    commit_button_rect: Option<egui::Rect>,
    amend_prefill: Option<AmendPrefill>,
    /// Upstream already containing HEAD, checked when Amend is switched on.
    amend_published: Option<String>,
    /// "Undo last commit" waiting for confirmation, with the mode chosen.
    pending_undo: Option<(
        crate::git::branches::UndoCommitPreview,
        crate::git::branches::ResetMode,
    )>,
}

/// The HEAD message copied into an empty draft when Amend is switched on.
#[derive(Debug)]
struct AmendPrefill {
    repo: Option<std::path::PathBuf>,
    previous_body: String,
    subject: String,
    body: String,
}

impl AmendPrefill {
    /// Switching Amend off restores the earlier draft if the copied message is unchanged.
    fn restore(self, repo: &Option<std::path::PathBuf>, subject: &mut String, body: &mut String) {
        if self.repo == *repo && *subject == self.subject && *body == self.body {
            subject.clear();
            *body = self.previous_body;
        }
    }
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
            composer_height: None,
            composer_footer_height: None,
            #[cfg(test)]
            commit_button_rect: None,
            amend_prefill: None,
            amend_published: None,
            pending_undo: None,
        }
    }

    pub fn flush_drafts(&mut self) -> Result<(), String> {
        self.drafts.flush_all()
    }

    /// `editor` and `protected_branches` come from App Settings.
    pub fn show(&mut self, ui: &mut Ui, editor: &str, protected_branches: &[String]) {
        let repo = self.state.lock().unwrap().repo_path.clone();
        self.drafts.switch_to(repo);
        if !self.drafts.current.is_amend {
            self.amend_prefill = None;
            self.amend_published = None;
        }
        self.drafts.poll();
        ui.painter()
            .rect_filled(ui.max_rect(), 0.0, theme::bg_surface());

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
        let mut launch_error: Option<String> = None;
        let mut request_undo: Option<ResetMode> = None;
        let (mut confirm_undo, mut cancel_undo) = (false, false);
        if self
            .pending_undo
            .as_ref()
            .is_some_and(|(preview, _)| !same_repository(&preview.reset.path, &repo_path))
        {
            self.pending_undo = None;
        }
        let mut do_discard_file: Option<String> = None;
        let mut do_ignore_file: Option<String> = None;

        let panel_rect = ui.available_rect_before_wrap();
        let composer_height = self.composer_height.unwrap_or(panel_rect.height());
        let composer_rect = egui::Rect::from_min_max(
            egui::pos2(
                panel_rect.left(),
                panel_rect.bottom() - composer_height.min(panel_rect.height()),
            ),
            panel_rect.max,
        );
        // Measure the composer before assigning the remaining viewport to the file list.
        let mut composer_ui = ui.new_child(
            egui::UiBuilder::new()
                .id_salt(("changes_composer", &repo_path))
                .max_rect(composer_rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        composer_ui.set_clip_rect(ui.clip_rect().intersect(panel_rect));
        let composer = egui::Frame::new()
            .fill(theme::bg_elevated())
            .inner_margin(kit::space::MD as i8)
            .show(&mut composer_ui, |ui| {
                // Load commit template on first open
                if !self.drafts.current.template_loaded
                    && self.drafts.current.subject.is_empty()
                    && self.drafts.current.body.is_empty()
                    && !self.drafts.current.is_amend
                {
                    self.drafts.current.template_loaded = true;
                    if let Some(ref rp) = repo_path {
                        if let Some(tmpl) = load_commit_template(rp) {
                            let mut lines =
                                tmpl.lines().filter(|l| !l.trim_start().starts_with('#'));
                            self.drafts.current.subject =
                                lines.next().unwrap_or("").trim().to_string();
                            let rest = lines
                                .skip_while(|l| l.is_empty())
                                .collect::<Vec<_>>()
                                .join("\n");
                            self.drafts.current.body = rest.trim().to_string();
                        }
                    }
                }

                let narrow_header = ui.available_width() < 300.0;
                if narrow_header {
                    ui.label(
                        RichText::new("Create a commit")
                            .size(15.0)
                            .strong()
                            .color(theme::text_primary()),
                    );
                }
                ui.horizontal(|ui| {
                    if !narrow_header {
                        ui.label(
                            RichText::new("Create a commit")
                                .size(15.0)
                                .strong()
                                .color(theme::text_primary()),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(kit::small_button("Copy"))
                            .on_hover_text(
                                "Copy the summary, description and author credits to the \
                                    clipboard.",
                            )
                            .clicked()
                        {
                            let msg = build_message(
                                &self.drafts.current.subject,
                                &self.drafts.current.body,
                                &self.drafts.current.co_authors,
                                self.drafts.current.signoff_enabled,
                                &repo_path,
                            );
                            ui.ctx().copy_text(msg);
                        }
                        ui.add_space(4.0);
                        if ui
                            .add(
                                kit::small_button("Amend")
                                    .fill(if self.drafts.current.is_amend {
                                        theme::bg_selected()
                                    } else {
                                        Color32::TRANSPARENT
                                    })
                                    .stroke(egui::Stroke::new(
                                        1.0,
                                        if self.drafts.current.is_amend {
                                            theme::accent_yellow()
                                        } else {
                                            theme::border()
                                        },
                                    )),
                            )
                            .on_hover_text(
                                "Replace the last commit with this message and staged changes; \
                                    its commit ID changes.",
                            )
                            .clicked()
                        {
                            self.drafts.current.is_amend = !self.drafts.current.is_amend;
                            if self.drafts.current.is_amend {
                                self.amend_published =
                                    repo_path.as_deref().and_then(head_published);
                            }
                            if self.drafts.current.is_amend
                                && self.drafts.current.subject.is_empty()
                            {
                                if let Some(msg) = staging::load_head_message(self.state.clone()) {
                                    let mut lines = msg.lines();
                                    let draft = &mut self.drafts.current;
                                    draft.subject = lines.next().unwrap_or("").to_string();
                                    let rest = lines
                                        .skip_while(|l| l.is_empty())
                                        .collect::<Vec<_>>()
                                        .join("\n");
                                    let previous_body = std::mem::replace(&mut draft.body, rest);
                                    self.amend_prefill = Some(AmendPrefill {
                                        repo: repo_path.clone(),
                                        previous_body,
                                        subject: draft.subject.clone(),
                                        body: draft.body.clone(),
                                    });
                                }
                            } else if !self.drafts.current.is_amend {
                                if let Some(prefill) = self.amend_prefill.take() {
                                    let draft = &mut self.drafts.current;
                                    prefill.restore(
                                        &repo_path,
                                        &mut draft.subject,
                                        &mut draft.body,
                                    );
                                }
                            }
                        }
                    });
                });

                ui.add_space(kit::space::XS);
                let summary_label = ui
                    .horizontal(|ui| {
                        let label = ui.label(
                            RichText::new("Summary")
                                .size(12.0)
                                .color(theme::text_secondary()),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let count = self.drafts.current.subject.chars().count();
                            ui.label(
                                RichText::new(format!("{count}/72"))
                                    .size(theme::font_size::SMALL)
                                    .color(summary_counter_color(count)),
                            )
                            .on_hover_text(
                                "Aim for 50 characters; keep the summary under 72 so it is easy \
                                    to scan.",
                            );
                        });
                        label
                    })
                    .inner;
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [ui.available_width(), 28.0],
                        egui::TextEdit::singleline(&mut self.drafts.current.subject)
                            .margin(egui::Vec2::new(8.0, 5.0))
                            .hint_text("Short summary, e.g. Fix login redirect")
                            .font(egui::FontId::proportional(13.0)),
                    )
                    .labelled_by(summary_label.id);
                });
                ui.add_space(kit::space::XS);
                let header_height = ui.cursor().top() - composer_rect.top();
                let footer_height = self.composer_footer_height.unwrap_or(
                    38.0 + kit::space::SM + kit::space::MD + ui.spacing().item_spacing.y,
                );
                let details_height = (panel_rect.height() - header_height - footer_height)
                    .max(0.0)
                    .min(panel_rect.height() * 0.38);
                egui::ScrollArea::vertical()
                    .id_salt("commit_composer_details")
                    .max_height(details_height)
                    .min_scrolled_height(0.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        let description_label = ui.label(
                            RichText::new("Description (optional)")
                                .size(12.0)
                                .color(theme::text_secondary()),
                        );
                        ui.horizontal(|ui| {
                            ui.add_sized(
                                [ui.available_width(), 60.0],
                                egui::TextEdit::multiline(&mut self.drafts.current.body)
                                    .desired_width(ui.available_width())
                                    .desired_rows(3)
                                    .hint_text("Explain why, not just what (optional)")
                                    .margin(egui::Vec2::new(8.0, 6.0))
                                    .font(egui::FontId::proportional(13.0)),
                            )
                            .labelled_by(description_label.id);
                        });

                        kit::disclosure(ui, "commit_options", "Commit options", false, |ui| {
                            // ── Co-author trailers ────────────────────────────────────
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                ui.add_sized(
                                    [form_width(ui, 68.0, 190.0), 28.0],
                                    egui::TextEdit::singleline(
                                        &mut self.drafts.current.co_author_input,
                                    )
                                    .margin(egui::Vec2::new(8.0, 5.0))
                                    .hint_text("Co-author: Name <email>")
                                    .font(egui::FontId::proportional(12.0)),
                                );
                                ui.add_space(4.0);
                                if ui.add(small_btn("+ Add")).clicked()
                                    && !self.drafts.current.co_author_input.trim().is_empty()
                                {
                                    self.drafts.current.co_authors.push(
                                        self.drafts.current.co_author_input.trim().to_string(),
                                    );
                                    self.drafts.current.co_author_input.clear();
                                }
                            });
                            let mut remove_co_author: Option<usize> = None;
                            for (i, author) in self.drafts.current.co_authors.iter().enumerate() {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(format!("Co-Authored-By: {}", author))
                                            .color(theme::text_muted())
                                            .size(12.0)
                                            .monospace(),
                                    );
                                    if ui
                                        .small_button(RichText::new("×").color(theme::text_muted()))
                                        .clicked()
                                    {
                                        remove_co_author = Some(i);
                                    }
                                });
                            }
                            if let Some(i) = remove_co_author {
                                self.drafts.current.co_authors.remove(i);
                            }

                            // ── Sign-off + copy ──────────────────────────────────────
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                ui.checkbox(
                                    &mut self.drafts.current.signoff_enabled,
                                    RichText::new("Sign off")
                                        .color(theme::text_muted())
                                        .size(12.0),
                                );
                                if ui.small_button("Identity / signing").clicked() {
                                    self.state.lock().unwrap().pending_commit_settings = true;
                                }
                            });

                            ui.add_space(8.0);
                        });

                        // ── Branch protection warning ─────────────────────────────
                        let amend_notice = self
                            .amend_published
                            .as_ref()
                            .filter(|_| self.drafts.current.is_amend)
                            .map(|upstream| {
                                format!(
                                "This commit is already on {upstream}. After amending, pushing \
                                    requires Force Push with Lease."
                        )
                            });
                        animated_notice(
                            ui,
                            "amend_notice",
                            kit::Status::Warning,
                            amend_notice.as_deref(),
                        );
                        let protected_notice =
                            (is_protected_branch(&head_branch, protected_branches)
                                && !self.drafts.current.is_amend)
                                .then(|| {
                                    format!(
                                        "Committing directly to {head_branch}. Make sure this is \
                                    intentional."
                                    )
                                });
                        animated_notice(
                            ui,
                            "protected_notice",
                            kit::Status::Warning,
                            protected_notice.as_deref(),
                        );
                        animated_notice(
                            ui,
                            "sequence_notice",
                            kit::Status::Ok,
                            (any_seq_in_progress && conflicted.is_empty()).then_some(
                                "All conflicts resolved. Stage remaining changes, then click \
                                    Continue in the banner above.",
                            ),
                        );
                        let draft_errors = self
                            .drafts
                            .save_errors
                            .iter()
                            .map(|(path, error)| {
                                format!("Draft is only in memory ({}): {error}", path.display())
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        animated_notice(
                            ui,
                            "draft_errors",
                            kit::Status::Error,
                            (!draft_errors.is_empty()).then_some(draft_errors.as_str()),
                        );
                        ui.add_space(6.0);
                        kit::disclosure(ui, "reset_last_commit", "Undo last commit", false, |ui| {
                            // ── Reset last commit ─────────────────────────────────────
                            ui.add_space(8.0);
                            if let Some((preview, mode)) = &self.pending_undo {
                                let mode = *mode;
                                commit_notice(
                                    ui,
                                    RichText::new(undo_confirmation_text(preview, mode))
                                        .color(
                                            if mode == ResetMode::Hard
                                                || preview.published_to.is_some()
                                            {
                                                theme::accent_yellow()
                                            } else {
                                                theme::text_secondary()
                                            },
                                        )
                                        .size(12.0),
                                );
                                ui.horizontal(|ui| {
                                    let label = format!("{} reset", mode.label());
                                    let confirm = if mode == ResetMode::Hard {
                                        small_btn_danger(&label)
                                    } else {
                                        small_btn(&label)
                                    };
                                    if ui.add(confirm).clicked() {
                                        confirm_undo = true;
                                    }
                                    if ui.add(small_btn("Cancel")).clicked() {
                                        cancel_undo = true;
                                    }
                                });
                            } else {
                                ui.horizontal_wrapped(|ui| {
                                    kit::hint(ui, "Reset last commit:");
                                    ui.add_space(4.0);
                                    if ui
                                        .add(small_btn("Soft"))
                                        .on_hover_text("Keep changes staged; asks for confirmation")
                                        .clicked()
                                    {
                                        request_undo = Some(ResetMode::Soft);
                                    }
                                    if ui
                                        .add(small_btn("Mixed"))
                                        .on_hover_text(
                                            "Keep changes unstaged; asks for confirmation",
                                        )
                                        .clicked()
                                    {
                                        request_undo = Some(ResetMode::Mixed);
                                    }
                                    if ui
                                        .add(small_btn_danger("Hard"))
                                        .on_hover_text(
                                            "Reset HEAD and discard tracked working changes; \
                                                asks for confirmation",
                                        )
                                        .clicked()
                                    {
                                        request_undo = Some(ResetMode::Hard);
                                    }
                                });
                                kit::hint(ui, "Soft keeps changes staged, ready to commit again.");
                                kit::hint(
                                    ui,
                                    "Mixed keeps changes in your files so you can choose what to \
                                        stage.",
                                );
                                kit::hint(
                                    ui,
                                    "Hard discards tracked changes from the commit and your \
                                        working \
                                        files.",
                                );
                            }
                        });
                    });
                ui.add_space(kit::space::SM);
                let disabled_reason = commit_disabled_reason(
                    &self.drafts.current.subject,
                    staged.len(),
                    self.drafts.current.is_amend,
                    self.drafts.busy(),
                    conflicted.len(),
                );
                let can_commit = disabled_reason.is_empty();
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
                let footer = ui.horizontal(|ui| {
                    let btn = ui.add_enabled(
                        can_commit,
                        kit::primary_button(&btn_label)
                            .truncate()
                            .min_size(Vec2::new(ui.available_width(), 38.0)),
                    );
                    let btn = kit::explain_disabled(btn, disabled_reason)
                        .on_hover_text("Save the staged changes as a commit in this repository.");
                    #[cfg(test)]
                    {
                        self.commit_button_rect = Some(btn.rect);
                    }
                    if btn.clicked() {
                        if self.drafts.current.is_amend {
                            do_amend = true;
                        } else {
                            do_commit = true;
                        }
                    }
                });
                self.composer_footer_height = Some(
                    footer.response.rect.height()
                        + kit::space::SM
                        + kit::space::MD
                        + ui.spacing().item_spacing.y,
                );
            });
        let measured_height = composer.response.rect.height();
        if self
            .composer_height
            .is_none_or(|previous| (previous - measured_height).abs() > 0.5)
        {
            self.composer_height = Some(measured_height);
            ui.ctx()
                .request_discard("Measure the pinned commit composer");
            ui.ctx().request_repaint();
        }
        let list_rect = egui::Rect::from_min_max(
            panel_rect.min,
            egui::pos2(
                panel_rect.right(),
                composer_rect.top().max(panel_rect.top()),
            ),
        );
        let list_body_rect = list_rect.shrink2(egui::vec2(kit::space::MD, 0.0));
        let mut list_ui = ui.new_child(
            egui::UiBuilder::new()
                .id_salt(("changes_list", &repo_path))
                .max_rect(list_body_rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        list_ui.set_clip_rect(ui.clip_rect().intersect(list_body_rect));
        egui::ScrollArea::vertical()
            .id_salt("changes_file_list")
            .max_height(list_rect.height())
            .min_scrolled_height(0.0)
            .auto_shrink([false; 2])
            .show(&mut list_ui, |ui| {
                batch = batch_bar(ui, selection);
                if files.is_empty() && conflicted.is_empty() {
                    kit::empty_state(
                        ui,
                        crate::icons::DONE_ALL,
                        "No local changes",
                        "Edit files in your editor and they'll appear here, ready to commit.",
                        None,
                    );
                    return;
                }
                kit::hint(
                    ui,
                    "Click a file to inspect it. Check boxes to select several files.",
                );
                ui.add_space(8.0);

                // ── CONFLICTS ────────────────────────────────────────────
                if !conflicted.is_empty() {
                    collapsible_section(
                        ui,
                        "Conflicts",
                        conflicted.len(),
                        &mut self.conflicts_expanded,
                        |_| {},
                    );
                    motion::reveal(
                        ui,
                        ui.id().with("conflicts_files"),
                        self.conflicts_expanded,
                        |ui| {
                            ui.add_enabled_ui(self.conflicts_expanded, |ui| {
                                for cf in &conflicted {
                                    let is_sel = selected_file.as_deref() == Some(cf.path.as_str());
                                    let resp =
                                        conflict_file_row(ui, &cf.path, is_sel, |ui| {
                                            if ui
                                            .add(small_btn_danger("Use mine"))
                                            .on_hover_text(if cf.ours_deleted {
                                                "Keep the deletion from your current branch (ours)."
                                            } else {
                                                "Keep the version from your current branch (ours)."
                                            })
                                            .clicked()
                                        {
                                            do_choose_ours = Some(cf.path.clone());
                                        }
                                            if ui
                                            .add(small_btn_danger("Use theirs"))
                                            .on_hover_text(if cf.theirs_deleted {
                                                "Keep the deletion from the incoming side (theirs)."
                                            } else {
                                                "Keep the incoming version (theirs)."
                                            })
                                            .clicked()
                                        {
                                            do_choose_theirs = Some(cf.path.clone());
                                        }
                                            if ui.add(small_btn("Mark as resolved"))
                                            .on_hover_text(
                                                "Tell Git this file is fixed; it will be staged.")
                                            .clicked() {
                                            do_mark_resolved = Some(cf.path.clone());
                                        }
                                        });
                                    resp.context_menu(|ui| {
                                        if ui.button("Resolve in Recorde…").clicked() {
                                            self.state.lock().unwrap().pending_conflict_editor =
                                                Some(cf.path.clone());
                                            ui.close();
                                        }
                                        if ui.button("Open merge tool").clicked() {
                                            do_open_mergetool = Some(cf.path.clone());
                                            ui.close();
                                        }
                                        if ui.button("Reveal in folder").clicked() {
                                            launch_error =
                                                reveal_in_folder(&cf.path, &repo_path).err();
                                            ui.close();
                                        }
                                    });
                                    if resp.clicked() {
                                        view_file =
                                            Some((cf.path.clone(), DiffContext::UnstagedFile));
                                    }
                                }
                            });
                        },
                    );
                    ui.add_space(6.0);
                }

                // ── UNSTAGED CHANGES ─────────────────────────────────────
                let unstaged_expanded = &mut self.unstaged_expanded;
                collapsible_section(ui, "Unstaged", unstaged.len(), unstaged_expanded, |ui| {
                    if ui
                        .add_enabled(!unstaged.is_empty(), small_btn("Stage all"))
                        .on_hover_text("Include all changed and new files in your next commit.")
                        .clicked()
                    {
                        do_stage_all = true;
                    }
                });

                motion::reveal(
                    ui,
                    ui.id().with("unstaged_files"),
                    self.unstaged_expanded,
                    |ui| {
                        ui.add_enabled_ui(self.unstaged_expanded, |ui| {
                            for f in &unstaged {
                                let is_sel = selected_file.as_deref() == Some(f.path.as_str());
                                let (resp, inline_clicked) = file_row(
                                    ui,
                                    &f.path,
                                    f.unstaged_status.unwrap_or('M'),
                                    is_sel,
                                    selection,
                                    false,
                                    |ui| {
                                        if ui.button("Stage").clicked() {
                                            do_stage_file = Some(f.path.clone());
                                            ui.close();
                                        }
                                        if ui
                                            .button(
                                                RichText::new("Discard").color(theme::accent_red()),
                                            )
                                            .clicked()
                                        {
                                            do_discard_file = Some(f.path.clone());
                                            ui.close();
                                        }
                                        if ui.button("Show file history").clicked() {
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
                                        if ui.button("Stash this file").clicked() {
                                            do_stash_file = Some(f.path.clone());
                                            ui.close();
                                        }
                                        ui.separator();
                                        if ui.button("Edit in Recorde").clicked() {
                                            self.state.lock().unwrap().pending_file_edit =
                                                Some(f.path.clone());
                                            ui.close();
                                        }
                                        if ui.button("Open in editor").clicked() {
                                            launch_error =
                                                open_file_in_editor(&f.path, &repo_path, editor)
                                                    .err();
                                            ui.close();
                                        }
                                        if ui.button("Reveal in folder").clicked() {
                                            launch_error =
                                                reveal_in_folder(&f.path, &repo_path).err();
                                            ui.close();
                                        }
                                        if ui.button("Open in external diff tool").clicked() {
                                            launch_error =
                                                open_diff_tool(&f.path, &repo_path, false).err();
                                            ui.close();
                                        }
                                    },
                                );
                                if inline_clicked {
                                    do_stage_file = Some(f.path.clone());
                                }
                                if resp.clicked() {
                                    view_file = Some((f.path.clone(), DiffContext::UnstagedFile));
                                }
                            }
                        });
                    },
                );

                ui.add_space(6.0);

                // ── STAGED CHANGES ───────────────────────────────────────
                let staged_expanded = &mut self.staged_expanded;
                collapsible_section(ui, "Staged", staged.len(), staged_expanded, |ui| {
                    if ui
                        .add_enabled(!staged.is_empty(), small_btn("Unstage all"))
                        .on_hover_text(
                            "Leave all files out of your next commit; \
                             keep their changes in your working files.",
                        )
                        .clicked()
                    {
                        do_unstage_all = true;
                    }
                });

                motion::reveal(
                    ui,
                    ui.id().with("staged_files"),
                    self.staged_expanded,
                    |ui| {
                        ui.add_enabled_ui(self.staged_expanded, |ui| {
                            for f in &staged {
                                let is_sel = selected_file.as_deref() == Some(f.path.as_str());
                                let (resp, inline_clicked) = file_row(
                                    ui,
                                    &f.path,
                                    f.staged_status.unwrap_or('M'),
                                    is_sel,
                                    selection,
                                    true,
                                    |ui| {
                                        if ui.button("Unstage").clicked() {
                                            do_unstage_file = Some(f.path.clone());
                                            ui.close();
                                        }
                                        if ui.button("Edit in Recorde").clicked() {
                                            self.state.lock().unwrap().pending_file_edit =
                                                Some(f.path.clone());
                                            ui.close();
                                        }
                                        if ui.button("Open in editor").clicked() {
                                            launch_error =
                                                open_file_in_editor(&f.path, &repo_path, editor)
                                                    .err();
                                            ui.close();
                                        }
                                        if ui.button("Reveal in folder").clicked() {
                                            launch_error =
                                                reveal_in_folder(&f.path, &repo_path).err();
                                            ui.close();
                                        }
                                        if ui.button("Open in external diff tool").clicked() {
                                            launch_error =
                                                open_diff_tool(&f.path, &repo_path, true).err();
                                            ui.close();
                                        }
                                    },
                                );
                                if inline_clicked {
                                    do_unstage_file = Some(f.path.clone());
                                }
                                if resp.clicked() {
                                    view_file = Some((f.path.clone(), DiffContext::StagedFile));
                                }
                            }
                        });
                    },
                );

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

                motion::reveal(
                    ui,
                    ui.id().with("untracked_files"),
                    self.untracked_expanded,
                    |ui| {
                        ui.add_enabled_ui(self.untracked_expanded, |ui| {
                            for f in &untracked {
                                let is_sel = selected_file.as_deref() == Some(f.path.as_str());
                                let (resp, inline_clicked) =
                                    file_row(ui, &f.path, '?', is_sel, selection, false, |ui| {
                                        if ui.button("Stage").clicked() {
                                            do_stage_file = Some(f.path.clone());
                                            ui.close();
                                        }
                                        if ui.button("Ignore").clicked() {
                                            do_ignore_file = Some(f.path.clone());
                                            ui.close();
                                        }
                                        if ui.button("Edit in Recorde").clicked() {
                                            self.state.lock().unwrap().pending_file_edit =
                                                Some(f.path.clone());
                                            ui.close();
                                        }
                                        if ui.button("Reveal in folder").clicked() {
                                            launch_error =
                                                reveal_in_folder(&f.path, &repo_path).err();
                                            ui.close();
                                        }
                                        if ui.button("Add to .gitignore").clicked() {
                                            do_ignore_file = Some(f.path.clone());
                                            ui.close();
                                        }
                                    });
                                if inline_clicked {
                                    do_stage_file = Some(f.path.clone());
                                }
                                if resp.clicked() {
                                    view_file = Some((f.path.clone(), DiffContext::UnstagedFile));
                                }
                            }
                        });
                    },
                );
            });
        ui.advance_cursor_after_rect(panel_rect);

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

        if let Some(error) = launch_error {
            self.state.lock().unwrap().error = Some(error);
        }
        if let (Some(mode), Some(path)) = (request_undo, repo_path.as_deref()) {
            let epoch = self.state.lock().unwrap().repo_epoch;
            match crate::git::branches::UndoCommitPreview::capture(path, epoch) {
                Ok(preview) => self.pending_undo = Some((preview, mode)),
                Err(error) => self.state.lock().unwrap().error = Some(error),
            }
        }
        if cancel_undo {
            self.pending_undo = None;
        }
        if confirm_undo {
            if let Some((preview, mode)) = self.pending_undo.take() {
                preview.reset.submit(mode, self.state.clone());
            }
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
        if let Some((name, email)) = git_identity_for_signoff(repo_path) {
            trailers.push(format!("Signed-off-by: {} <{}>", name, email));
        }
    }
    if !trailers.is_empty() {
        msg.push_str("\n\n");
        msg.push_str(&trailers.join("\n"));
    }
    msg
}

/// The repository's effective identity, else the global one; never the process directory's.
fn git_identity_for_signoff(repo_path: &Option<std::path::PathBuf>) -> Option<(String, String)> {
    let config = repo_path
        .as_ref()
        .and_then(|path| git2::Repository::open(path).ok())
        .and_then(|repo| repo.config().ok())
        .or_else(|| git2::Config::open_default().ok())?;
    let name = config.get_string("user.name").ok()?;
    let email = config.get_string("user.email").ok()?;
    Some((name, email))
}

/// Read `commit.template` like Git: `~/` expands to the home directory and relative
/// paths resolve from the worktree root. Oversized or non-UTF-8 templates are ignored.
fn load_commit_template(repo_path: &std::path::Path) -> Option<String> {
    use std::io::Read;
    const LIMIT: u64 = 64 * 1024;
    let repo = git2::Repository::open(repo_path).ok()?;
    let configured = repo.config().ok()?.get_path("commit.template").ok()?;
    let path = template_path(configured, repo.workdir().unwrap_or(repo_path));
    let mut text = String::new();
    std::fs::File::open(path)
        .ok()?
        .take(LIMIT + 1)
        .read_to_string(&mut text)
        .ok()?;
    (text.len() as u64 <= LIMIT).then_some(text)
}

fn template_path(configured: std::path::PathBuf, workdir: &std::path::Path) -> std::path::PathBuf {
    if configured.is_absolute() {
        configured
    } else {
        workdir.join(configured)
    }
}

// ── Shell helpers ─────────────────────────────────────────────────────────────

/// Open a worktree file through the hardened launcher; never through a shell.
fn open_file_in_editor(
    file_path: &str,
    repo_path: &Option<std::path::PathBuf>,
    editor: &str,
) -> Result<(), String> {
    let repo = repo_path
        .as_ref()
        .ok_or("Open a repository before launching an editor.")?;
    let file = crate::git::safe_path::relative(file_path)?;
    crate::external::open_editor(&repo.join(file), &crate::external::editor_command(editor)?)
}

/// Compare the index with the worktree (unstaged) or HEAD with the index (staged).
fn open_diff_tool(
    file_path: &str,
    repo_path: &Option<std::path::PathBuf>,
    staged: bool,
) -> Result<(), String> {
    let repo = repo_path
        .as_ref()
        .ok_or("Open a repository before launching a diff tool.")?;
    let request = if staged {
        crate::external::DiffToolArgs::Staged { path: file_path }
    } else {
        crate::external::DiffToolArgs::Unstaged { path: file_path }
    };
    crate::external::open_diff_tool(repo, request)
}

fn reveal_in_folder(file_path: &str, repo_path: &Option<std::path::PathBuf>) -> Result<(), String> {
    let repo = repo_path
        .as_ref()
        .ok_or("Open a repository before revealing a file.")?;
    let file = crate::git::safe_path::relative(file_path)?;
    crate::external::reveal_path(&repo.join(file))
}

/// The confirmation shown before "Undo last commit" resets to the captured parent.
fn undo_confirmation_text(
    preview: &crate::git::branches::UndoCommitPreview,
    mode: ResetMode,
) -> String {
    let mut text = format!(
        "{} reset: undo \"{}\" ({})? {}",
        mode.label(),
        preview.summary,
        preview.short_oid,
        mode.description()
    );
    if preview.merge {
        text.push_str(" This is a merge commit; the branch returns to its first parent.");
    }
    if let Some(upstream) = &preview.published_to {
        text.push_str(&format!(
            " The commit is already on {upstream}; pushing afterwards \
                    requires Force Push with Lease."
        ));
    }
    text.push_str(" A recovery snapshot is saved first.");
    text
}

fn same_repository(captured: &std::path::Path, current: &Option<std::path::PathBuf>) -> bool {
    current
        .as_ref()
        .and_then(|path| path.canonicalize().ok())
        .is_some_and(|path| path == captured)
}

/// The upstream already containing HEAD, if any (amending it rewrites published history).
fn head_published(repo_path: &std::path::Path) -> Option<String> {
    let repo = git2::Repository::open(repo_path).ok()?;
    let head = repo.head().ok()?.peel_to_commit().ok()?.id();
    crate::git::branches::published_upstream(&repo, head)
}

/// Matches the graph-action rule: protected branches are exact local branch names.
fn is_protected_branch(branch: &str, protected: &[String]) -> bool {
    !branch.is_empty() && protected.iter().any(|name| name == branch)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn selection_summary(count: usize) -> String {
    format!(
        "{count} {} selected",
        if count == 1 { "file" } else { "files" }
    )
}

fn batch_bar(
    ui: &mut Ui,
    selection: &mut std::collections::BTreeSet<(bool, String)>,
) -> Option<(Vec<String>, bool)> {
    let open = !selection.is_empty();
    let mut batch = None;
    motion::reveal(ui, ui.id().with("batch_actions"), open, |ui| {
        ui.add_enabled_ui(open, |ui| {
            egui::Frame::new()
                .fill(theme::bg_selected())
                .corner_radius(6)
                .inner_margin(kit::space::SM as i8)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.x = kit::space::XS;
                    let summary = selection_summary(selection.len());
                    let actions: Vec<_> = [(false, "Stage"), (true, "Unstage")]
                        .into_iter()
                        .filter_map(|(staged, label)| {
                            let paths: Vec<_> = selection
                                .iter()
                                .filter(|(side, _)| *side == staged)
                                .map(|(_, path)| path.clone())
                                .collect();
                            (!paths.is_empty()).then(|| {
                                let visible = format!("{label} ({})", paths.len());
                                let accessible = format!("{label} selected ({})", paths.len());
                                (staged, paths, visible, accessible)
                            })
                        })
                        .collect();
                    let text_width = |text: &str, size| {
                        ui.painter()
                            .layout_no_wrap(
                                text.to_owned(),
                                egui::FontId::proportional(size),
                                theme::text_secondary(),
                            )
                            .size()
                            .x
                    };
                    let controls_width = 32.0
                        + actions
                            .iter()
                            .map(|(_, _, label, _)| {
                                text_width(label, 12.0)
                                    + 2.0 * ui.spacing().button_padding.x
                                    + kit::space::XS
                            })
                            .sum::<f32>();
                    let summary_width = text_width(&summary, theme::font_size::SMALL);
                    let stacked =
                        summary_width + kit::space::SM + controls_width > ui.available_width();
                    if stacked {
                        kit::hint(ui, &summary);
                    }
                    let mut clear = false;
                    let mut controls = |ui: &mut Ui| {
                        clear =
                            kit::icon_button(ui, crate::icons::CLOSE, "Clear selection").clicked();
                        for (staged, paths, label, accessible) in actions.iter().rev() {
                            let response = ui.add(kit::small_button(label));
                            response.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::WidgetType::Button,
                                    ui.is_enabled(),
                                    accessible,
                                )
                            });
                            if response.clicked() {
                                batch = Some((paths.clone(), *staged));
                            }
                        }
                    };
                    ui.horizontal(|ui| {
                        if !stacked {
                            kit::hint(ui, &summary);
                        }
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center).with_main_wrap(true),
                            &mut controls,
                        );
                    });
                    if clear {
                        selection.clear();
                    }
                });
        });
    });
    batch
}

fn collapsible_section(
    ui: &mut Ui,
    title: &str,
    count: usize,
    expanded: &mut bool,
    add_right: impl FnOnce(&mut Ui),
) {
    let help = match title {
        "Staged" => "Files that will be included in your next commit.",
        "Untracked" => "New files Git isn't tracking yet. Stage them to include them.",
        "Conflicts" => "Choose which changes to keep, then mark each file as resolved.",
        _ => "Changed files not yet included in your next commit.",
    };
    let id = ui.id().with(("file_section", title));
    let openness = motion::progress(ui.ctx(), id, *expanded, motion::NORMAL);
    ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), 32.0),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            // Lay out the real action first; the heading gets exactly the remaining width.
            add_right(ui);
            let (rect, toggle) = ui.allocate_exact_size(
                Vec2::new(ui.available_width().max(0.0), 32.0),
                egui::Sense::click(),
            );
            let hover = motion::hover(ui, &toggle);
            ui.painter().rect_filled(
                rect,
                6.0,
                motion::mix(Color32::TRANSPARENT, theme::bg_hover(), hover),
            );
            kit::paint_chevron(
                ui,
                egui::pos2(rect.left() + kit::space::SM, rect.center().y),
                openness,
                theme::text_muted(),
            );
            let count_galley = ui.painter().layout_no_wrap(
                count.to_string(),
                egui::FontId::proportional(theme::font_size::SMALL),
                theme::text_muted(),
            );
            let count_width = (count_galley.size().x + kit::space::MD).min(rect.width());
            let count_rect = egui::Rect::from_center_size(
                egui::pos2(rect.right() - count_width / 2.0, rect.center().y),
                Vec2::new(count_width, 20.0),
            );
            ui.painter()
                .rect_filled(count_rect, 10.0, theme::bg_elevated());
            ui.painter().with_clip_rect(count_rect).galley(
                count_rect.center() - count_galley.size() / 2.0,
                count_galley,
                theme::text_muted(),
            );
            let text_left = (rect.left() + 20.0).min(count_rect.left());
            let text_width = (count_rect.left() - text_left - kit::space::XS).max(0.0);
            let color = if title == "Conflicts" {
                theme::accent_red()
            } else {
                theme::text_secondary()
            };
            let mut job = egui::text::LayoutJob::simple(
                title.to_owned(),
                egui::FontId::proportional(theme::font_size::BODY),
                color,
                text_width,
            );
            job.wrap.max_rows = 1;
            job.wrap.break_anywhere = true;
            let galley = ui.painter().layout_job(job);
            let text_rect = egui::Rect::from_min_size(
                egui::pos2(text_left, rect.top()),
                Vec2::new(text_width, rect.height()),
            );
            ui.painter().with_clip_rect(text_rect).galley(
                egui::pos2(text_left, rect.center().y - galley.size().y / 2.0),
                galley,
                color,
            );
            if toggle.clicked() {
                *expanded = !*expanded;
            }
            toggle.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::CollapsingHeader,
                    ui.is_enabled(),
                    *expanded,
                    title,
                )
            });
            ui.ctx().accesskit_node_builder(toggle.id, |node| {
                node.set_expanded(*expanded);
            });
            toggle.on_hover_text(help);
        },
    );
}

fn file_row(
    ui: &mut Ui,
    path: &str,
    status: char,
    is_selected: bool,
    selection: &mut std::collections::BTreeSet<(bool, String)>,
    staged: bool,
    mut add_menu: impl FnMut(&mut Ui),
) -> (egui::Response, bool) {
    ui.push_id((staged, path), |ui| {
        let background = ui.painter().add(egui::Shape::Noop);
        let (row_rect, row) = ui.allocate_exact_size(
            Vec2::new(ui.available_width().max(0.0), 32.0),
            egui::Sense::hover(),
        );
        let menu_width = crate::ui::widgets::ROW_ACTIONS_WIDTH.min(row_rect.width());
        let action_width = 64.0_f32.min((row_rect.width() - menu_width).max(0.0));
        let action_right =
            (row_rect.right() - menu_width - kit::space::XS).max(row_rect.left() + action_width);
        let slot = egui::Rect::from_min_max(
            egui::pos2(action_right - action_width, row_rect.top()),
            egui::pos2(action_right, row_rect.bottom()),
        );
        let content_rect = egui::Rect::from_min_max(
            row_rect.min,
            egui::pos2(
                (slot.left() - kit::space::XS).max(row_rect.left()),
                row_rect.bottom(),
            ),
        );
        let mut content_ui = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("file_content")
                .max_rect(content_rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        content_ui.set_clip_rect(ui.clip_rect().intersect(content_rect));
        content_ui.spacing_mut().item_spacing.x = kit::space::XS;
        let key = (staged, path.to_owned());
        let mut checked = selection.contains(&key);
        let checkbox = content_ui.checkbox(&mut checked, "");
        checkbox.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::Checkbox,
                ui.is_enabled(),
                checked,
                format!("Select {path} for batch staging"),
            )
        });
        let checkbox = checkbox
            .on_hover_text("Select for batch actions. This checkbox does not stage the file.");
        if checkbox.changed() {
            if checked {
                selection.insert(key);
            } else {
                selection.remove(&key);
            }
        }
        let (badge, color) = status_icon(status);
        content_ui
            .add(
                egui::Label::new(
                    RichText::new(badge)
                        .size(theme::font_size::SMALL)
                        .color(color),
                )
                .truncate(),
            )
            .on_hover_text(status_tooltip(status));
        let width = content_ui.available_width().max(0.0);
        let (rect, label) = content_ui
            .allocate_exact_size(Vec2::new(width, row_rect.height()), egui::Sense::click());
        let label = label.on_hover_text(path);
        if label.has_focus() {
            ui.painter().rect_stroke(
                rect.shrink(1.0),
                6.0,
                egui::Stroke::new(1.5, theme::accent_blue()),
                egui::StrokeKind::Inside,
            );
        }
        let name = path.rsplit('/').next().unwrap_or(path);
        let mut job = egui::text::LayoutJob::simple(
            name.into(),
            egui::FontId::proportional(theme::font_size::BODY),
            theme::text_primary(),
            (width - kit::space::XS).max(0.0),
        );
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        let galley = ui.painter().layout_job(job);
        ui.painter().with_clip_rect(rect).galley(
            egui::pos2(rect.left(), rect.center().y - galley.size().y / 2.0),
            galley,
            theme::text_primary(),
        );
        label.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), path)
        });
        let action_id = ui.id().with("inline_action");
        let focused_id = ui.data(|data| data.get_temp::<egui::Id>(action_id.with("focus")));
        let action_focused = focused_id.is_some_and(|id| ui.memory(|memory| memory.has_focus(id)));
        let show = is_selected
            || ui.rect_contains_pointer(row_rect)
            || label.has_focus()
            || checkbox.has_focus()
            || action_focused;
        let progress = motion::progress(ui.ctx(), action_id, show, motion::FAST);
        let mut action_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(slot)
                .id_salt("inline_action_ui"),
        );
        action_ui.multiply_opacity(progress);
        let visible = egui::Rect::from_min_size(
            slot.min + egui::vec2(action_width * (1.0 - progress), kit::space::XS),
            Vec2::new(action_width * progress, 24.0),
        );
        action_ui.set_clip_rect(ui.clip_rect().intersect(visible));
        let action = action_ui
            .put(
                slot.shrink2(egui::vec2(0.0, kit::space::XS)),
                kit::small_button(if staged { "Unstage" } else { "Stage" }),
            )
            .on_hover_text(if staged {
                "Leave this file out of the next commit; keep its changes in your working files."
            } else {
                "Include this file's changes in your next commit."
            });
        action.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                ui.is_enabled(),
                format!("{} {path}", if staged { "Unstage" } else { "Stage" }),
            )
        });
        ui.data_mut(|data| data.insert_temp(action_id.with("focus"), action.id));
        if action.gained_focus() || action.lost_focus() {
            ui.ctx().request_repaint();
        }
        let mut menu_row = label.clone();
        menu_row.rect = row_rect;
        // The shared overflow control has a bounded slot and keeps the existing menu labels.
        crate::ui::widgets::row_actions_menu(ui, &menu_row, path, true, |ui| add_menu(ui));
        let hover = motion::hover(ui, &row);
        let fill = if is_selected {
            theme::bg_selected()
        } else {
            motion::mix(Color32::TRANSPARENT, theme::bg_hover(), hover)
        };
        ui.painter()
            .set(background, egui::Shape::rect_filled(row_rect, 6.0, fill));
        (label, action.clicked())
    })
    .inner
}

fn small_btn(label: &str) -> Button<'_> {
    kit::small_button(label)
}

fn small_btn_danger(label: &str) -> Button<'_> {
    kit::danger_outline_button(label).min_size(Vec2::new(0.0, 24.0))
}

fn status_icon(status: char) -> (&'static str, Color32) {
    match status {
        'A' => ("Added", theme::accent_green()),
        '?' => ("Untracked", theme::accent_green()),
        'D' => ("Deleted", theme::accent_red()),
        'R' => ("Renamed", theme::accent_yellow()),
        'C' | 'U' => ("Conflicted", theme::accent_red()),
        _ => ("Modified", theme::accent_yellow()),
    }
}

fn status_tooltip(status: char) -> &'static str {
    match status {
        'A' => "Added: this new file will be included in your next commit.",
        '?' => "Untracked: stage this new file to include it in your next commit.",
        'D' => "Deleted: this file has been removed.",
        'R' => "Renamed: this file has moved or its name has changed.",
        'C' | 'U' => "Conflicted: choose which changes to keep, then mark the file as resolved.",
        _ => "Modified: this file has changes since the last commit.",
    }
}

fn conflict_file_row(
    ui: &mut Ui,
    path: &str,
    is_selected: bool,
    mut add_btns: impl FnMut(&mut Ui),
) -> egui::Response {
    ui.push_id(("conflict", path), |ui| {
        let background = ui.painter().add(egui::Shape::Noop);
        let wide = ui.available_width() >= 560.0;
        let row = ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), 32.0),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.x = kit::space::XS;
                ui.menu_button(crate::icons::rich(crate::icons::MORE_VERT, 18.0), |ui| {
                    add_btns(ui);
                })
                .response
                .widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Button,
                        ui.is_enabled(),
                        format!("Actions for {path}"),
                    )
                });
                if wide {
                    add_btns(ui);
                }
                ui.allocate_ui_with_layout(
                    Vec2::new(ui.available_width().max(0.0), 32.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.label(
                            RichText::new("Conflicted")
                                .color(theme::accent_red())
                                .size(12.0),
                        )
                        .on_hover_text(status_tooltip('C'));
                        ui.add_sized(
                            [ui.available_width().max(0.0), 32.0],
                            Button::new(path.rsplit('/').next().unwrap_or(path))
                                .fill(Color32::TRANSPARENT)
                                .stroke(egui::Stroke::NONE)
                                .truncate(),
                        )
                        .on_hover_text(path)
                    },
                )
                .inner
            },
        );
        let hover = motion::hover(ui, &row.response);
        let fill = if is_selected {
            theme::bg_selected()
        } else {
            motion::mix(Color32::TRANSPARENT, theme::bg_hover(), hover)
        };
        ui.painter().set(
            background,
            egui::Shape::rect_filled(row.response.rect, 6.0, fill),
        );
        row.inner
    })
    .inner
}

fn available_width_after(ui: &Ui, reserved: f32) -> f32 {
    (ui.available_width() - reserved).max(0.0)
}

fn form_width(ui: &Ui, reserved: f32, max_width: f32) -> f32 {
    available_width_after(ui, reserved).min(max_width)
}

fn commit_notice(ui: &mut Ui, text: RichText) -> egui::Response {
    ui.horizontal(|ui| {
        ui.add_space(kit::space::MD);
        let width = (ui.available_width() - kit::space::MD).max(1.0);
        ui.allocate_ui_with_layout(
            Vec2::new(width, 0.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| kit::callout(ui, kit::Status::Warning, text.text()),
        )
        .inner
    })
    .inner
}

fn animated_notice(ui: &mut Ui, salt: &str, status: kit::Status, text: Option<&str>) {
    let id = ui.id().with(salt);
    if let Some(text) = text {
        ui.data_mut(|data| data.insert_temp(id.with("text"), text.to_owned()));
    }
    let message = ui
        .data(|data| data.get_temp::<String>(id.with("text")))
        .unwrap_or_default();
    motion::reveal(ui, id, text.is_some(), |ui| {
        kit::callout(ui, status, &message);
        ui.add_space(kit::space::XS);
    });
}

fn summary_counter_color(count: usize) -> Color32 {
    if count > 72 {
        theme::accent_red()
    } else if count > 50 {
        theme::accent_yellow()
    } else {
        theme::text_muted()
    }
}

fn commit_disabled_reason(
    subject: &str,
    staged: usize,
    amend: bool,
    busy: bool,
    conflicts: usize,
) -> &'static str {
    if busy {
        "Wait for the current commit to finish"
    } else if conflicts > 0 {
        "Resolve the conflicted files first"
    } else if subject.trim().is_empty() {
        "Write a summary first"
    } else if staged == 0 && !amend {
        "Stage at least one file"
    } else {
        ""
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_summary_uses_singular_for_one_file() {
        assert_eq!(selection_summary(0), "0 files selected");
        assert_eq!(selection_summary(1), "1 file selected");
        assert_eq!(selection_summary(2), "2 files selected");
    }

    #[test]
    fn changes_controls_fit_inside_the_panel_margin() {
        for width in [300.0, 340.0] {
            let ctx = egui::Context::default();
            ctx.set_fonts(crate::icons::font_definitions());
            ctx.enable_accesskit();
            let panel = egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(width, 600.0));
            let body = panel.shrink2(egui::vec2(kit::space::MD, 0.0));
            ctx.begin_pass(egui::RawInput {
                screen_rect: Some(panel),
                ..Default::default()
            });
            let mut ui = Ui::new(
                ctx.clone(),
                egui::Id::new("changes_width_test"),
                egui::UiBuilder::new().max_rect(body),
            );
            let mut expanded = true;
            for (title, label) in [("Unstaged", "Stage all"), ("Staged", "Unstage all")] {
                collapsible_section(&mut ui, title, 123, &mut expanded, |ui| {
                    ui.add(small_btn(label));
                });
            }
            let mut selection = std::collections::BTreeSet::new();
            file_row(
                &mut ui,
                "hello.txt",
                '?',
                true,
                &mut selection,
                false,
                |_| {},
            );
            file_row(
                &mut ui,
                "staged.txt",
                'M',
                true,
                &mut selection,
                true,
                |_| {},
            );
            conflict_file_row(&mut ui, "conflict.txt", false, |_| {});
            selection.insert((false, "hello.txt".into()));
            let single = ui.push_id("single", |ui| batch_bar(ui, &mut selection));
            assert!(
                single.response.rect.height() <= 49.0,
                "Single selection should use one row"
            );
            assert!((single.response.rect.width() - body.width()).abs() <= 0.5);
            selection.insert((true, "staged.txt".into()));
            ui.push_id("mixed", |ui| batch_bar(ui, &mut selection));
            assert!(ui.min_rect().right() <= body.right() + 0.5);
            let mut output = ctx.end_pass();
            output.textures_delta.clear();
            let tree = output
                .platform_output
                .accesskit_update
                .expect("Accessible controls");
            for label in [
                "Stage all",
                "Unstage all",
                "Stage hello.txt",
                "Unstage staged.txt",
                "Actions for hello.txt",
                "Actions for staged.txt",
                "Actions for conflict.txt",
                "Stage selected (1)",
                "Unstage selected (1)",
                "Clear selection",
            ] {
                let nodes: Vec<_> = tree
                    .nodes
                    .iter()
                    .filter(|(_, node)| node.label() == Some(label))
                    .collect();
                assert!(!nodes.is_empty(), "Missing accessible label: {label}");
                for (_, node) in nodes {
                    let bounds = node.bounds().expect("Control bounds");
                    assert!(
                        bounds.x0 >= f64::from(body.left()) - 0.5
                            && bounds.x1 <= f64::from(body.right()) + 0.5,
                        "{label}: {bounds:?} outside {body:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn commit_button_stays_visible_with_one_unstaged_file() {
        for (width, height) in [
            (344.0, 600.0),
            (300.0, 600.0),
            (240.0, 600.0),
            (344.0, 300.0),
        ] {
            let state = crate::state::new_shared_state();
            state
                .lock()
                .unwrap()
                .working_dir_files
                .push(WorkingDirFile {
                    path: "hello.txt".into(),
                    staged_status: None,
                    unstaged_status: Some('M'),
                });
            let mut panel = StagingPanel::new(state);
            let ctx = egui::Context::default();
            ctx.set_fonts(crate::icons::font_definitions());
            let visible = egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(width, height));
            // The first pass measures the natural composer; later passes pin that height.
            for _ in 0..4 {
                ctx.begin_pass(egui::RawInput {
                    screen_rect: Some(visible),
                    ..Default::default()
                });
                let mut ui = Ui::new(
                    ctx.clone(),
                    egui::Id::new("pinned_composer_test"),
                    egui::UiBuilder::new().max_rect(visible),
                );
                panel.show(&mut ui, "", &[]);
                ctx.end_pass().textures_delta.clear();
            }
            let button = panel.commit_button_rect.expect("Commit button rendered");
            assert!(
                visible.contains_rect(button),
                "{button:?} outside {visible:?}"
            );
            assert!(button.bottom() >= visible.bottom() - kit::space::LG);
            assert!(button.width() >= width - 2.0 * kit::space::LG);
        }
    }

    #[test]
    fn commit_disabled_reasons_match_the_existing_prerequisites() {
        assert_eq!(
            commit_disabled_reason("", 1, false, false, 0),
            "Write a summary first"
        );
        assert_eq!(
            commit_disabled_reason("Summary", 0, false, false, 0),
            "Stage at least one file"
        );
        assert_eq!(commit_disabled_reason("Summary", 0, true, false, 0), "");
        assert_eq!(commit_disabled_reason("Summary", 1, false, false, 0), "");
        assert_eq!(
            commit_disabled_reason("Summary", 1, false, true, 0),
            "Wait for the current commit to finish"
        );
        assert_eq!(
            commit_disabled_reason("Summary", 1, false, false, 1),
            "Resolve the conflicted files first"
        );
    }

    #[test]
    fn summary_counter_marks_only_overlong_summaries() {
        assert_eq!(summary_counter_color(50), theme::text_muted());
        assert_eq!(summary_counter_color(51), theme::accent_yellow());
        assert_eq!(summary_counter_color(72), theme::accent_yellow());
        assert_eq!(summary_counter_color(73), theme::accent_red());
        assert_eq!(status_icon('?').0, "Untracked");
        assert_ne!(status_icon('?').0, status_icon('A').0);
    }

    #[test]
    fn commit_notices_wrap_without_expanding_narrow_composer() {
        for message in [
            "⚠ Committing directly to \"production\" — make sure this is intentional.",
            "✓ All conflicts resolved — stage remaining changes, then click \
                Continue in the banner above.",
        ] {
            let mut heights = Vec::new();
            for width in [240.0, 344.0, 600.0] {
                let ctx = egui::Context::default();
                ctx.begin_pass(egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(width, 400.0),
                    )),
                    ..Default::default()
                });
                let mut ui = Ui::new(
                    ctx.clone(),
                    egui::Id::new("responsive_commit_composer"),
                    egui::UiBuilder::new().max_rect(ctx.content_rect()),
                );
                let notice = commit_notice(&mut ui, RichText::new(message).size(12.0));
                assert!(notice.rect.right() <= width - 11.0);
                assert!(ui.min_rect().right() <= width);
                // Later controls must still receive the original panel width.
                assert!(ui.available_width() <= width);
                heights.push(notice.rect.height());
                ctx.end_pass().textures_delta.clear();
            }
            assert!(
                heights[0] > heights[2],
                "Narrow notices must wrap onto additional lines"
            );
        }
    }

    #[test]
    fn switching_amend_off_restores_an_unchanged_draft() {
        let repo = Some(std::path::PathBuf::from("repo"));
        let prefill = || AmendPrefill {
            repo: repo.clone(),
            previous_body: "my notes".into(),
            subject: "HEAD subject".into(),
            body: "HEAD body".into(),
        };
        let (mut subject, mut body) = ("HEAD subject".to_string(), "HEAD body".to_string());
        prefill().restore(&repo, &mut subject, &mut body);
        assert_eq!((subject.as_str(), body.as_str()), ("", "my notes"));

        let (mut subject, mut body) = ("Edited subject".to_string(), "HEAD body".to_string());
        prefill().restore(&repo, &mut subject, &mut body);
        assert_eq!(
            (subject.as_str(), body.as_str()),
            ("Edited subject", "HEAD body")
        );

        let (mut subject, mut body) = ("HEAD subject".to_string(), "HEAD body".to_string());
        prefill().restore(&Some("other".into()), &mut subject, &mut body);
        assert_eq!(subject, "HEAD subject");
    }

    #[test]
    fn commit_template_resolves_relative_paths_from_the_worktree() {
        let repo = crate::git::test_support::TestRepo::new();
        repo.write(".gitmessage", b"Subject\n\n# hint\nBody\n");
        repo.git_ok(&["config", "commit.template", ".gitmessage"]);
        assert_eq!(
            load_commit_template(repo.path()).as_deref(),
            Some("Subject\n\n# hint\nBody\n")
        );
        repo.write(".gitmessage", &vec![b'x'; 64 * 1024 + 1]);
        assert_eq!(load_commit_template(repo.path()), None);
        let absolute = repo.path().join("abs");
        assert_eq!(template_path(absolute.clone(), repo.path()), absolute);
        assert_eq!(
            template_path("rel".into(), repo.path()),
            repo.path().join("rel")
        );
    }

    #[test]
    fn undo_last_commit_captures_parent_and_reports_publication() {
        let remote = tempfile::tempdir().unwrap();
        git2::Repository::init_bare(remote.path()).unwrap();
        let repo = crate::git::test_support::TestRepo::new();
        let parent = repo.commit("file", b"one\n").to_string();
        repo.commit("file", b"two\n");
        let remote_path = remote.path().to_string_lossy().to_string();
        repo.git_ok(&["remote", "add", "origin", &remote_path]);
        repo.git_ok(&["push", "-u", "origin", "main"]);

        let preview = crate::git::branches::UndoCommitPreview::capture(repo.path(), 0).unwrap();
        assert_eq!(preview.reset.oid, parent);
        assert_eq!(preview.published_to.as_deref(), Some("origin/main"));
        assert!(!preview.merge);
        let text = undo_confirmation_text(&preview, ResetMode::Soft);
        assert!(text.contains("Force Push with Lease"), "{text}");
        assert_eq!(head_published(repo.path()).as_deref(), Some("origin/main"));

        repo.commit("file", b"three\n");
        let local = crate::git::branches::UndoCommitPreview::capture(repo.path(), 0).unwrap();
        assert_eq!(local.published_to, None);
        assert!(!undo_confirmation_text(&local, ResetMode::Mixed).contains("Force Push"));
        assert_eq!(head_published(repo.path()), None);

        // A commit made after confirmation makes the captured undo stale.
        repo.commit("file", b"four\n");
        assert!(local.reset.validate(repo.path()).is_err());
    }

    #[test]
    fn undo_last_commit_refuses_the_root_commit() {
        let repo = crate::git::test_support::TestRepo::new();
        repo.commit("file", b"one\n");
        assert!(crate::git::branches::UndoCommitPreview::capture(repo.path(), 0).is_err());
    }

    #[test]
    fn protected_branch_warning_uses_configured_names() {
        let protected = vec!["develop".to_string(), "release/1.0".to_string()];
        assert!(is_protected_branch("develop", &protected));
        assert!(is_protected_branch("release/1.0", &protected));
        assert!(!is_protected_branch("main", &protected));
        assert!(!is_protected_branch("develop-old", &protected));
        assert!(!is_protected_branch("", &[String::new()]));
    }

    #[test]
    fn file_launchers_reject_paths_outside_the_worktree() {
        let repo = Some(std::path::PathBuf::from("repo"));
        for file in ["../outside", ".git/config", "", "/etc/passwd"] {
            assert!(
                open_file_in_editor(file, &repo, "editor").is_err(),
                "{file}"
            );
            assert!(reveal_in_folder(file, &repo).is_err(), "{file}");
        }
        assert!(open_file_in_editor("file", &None, "editor").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn open_in_editor_passes_metacharacter_file_names_as_one_argument() {
        let root = tempfile::tempdir().unwrap();
        let script = root.path().join("editor.sh");
        let output = root.path().join("received");
        std::fs::write(&script, "printf '%s' \"$2\" > \"$1\"\n").unwrap();
        let name = "x&calc %PATH% ^ $(touch marker).txt";
        open_file_in_editor(
            name,
            &Some(root.path().to_path_buf()),
            &format!("/bin/sh \"{}\" \"{}\"", script.display(), output.display()),
        )
        .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while !output.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(
            std::fs::read_to_string(output).unwrap(),
            root.path().join(name).to_string_lossy()
        );
        assert!(!root.path().join("marker").exists());
    }
}
