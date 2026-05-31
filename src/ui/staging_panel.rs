use egui::{Color32, RichText, Ui, Vec2};

use crate::git::{conflicts, operations::{load_blame, load_file_history, load_workdir_diff}, staging};
use crate::state::{ConflictedFile, DiffContext, PendingDiscard, SharedState, WorkingDirFile};
use crate::theme;

pub struct StagingPanel {
    state: SharedState,
    subject: String,
    body: String,
    is_amend: bool,
    conflicts_expanded: bool,
    staged_expanded: bool,
    unstaged_expanded: bool,
    untracked_expanded: bool,
    // Phase 3: commit workflow
    co_author_input: String,
    co_authors: Vec<String>,
    signoff_enabled: bool,
    template_loaded: bool,
}

impl StagingPanel {
    pub fn new(state: SharedState) -> Self {
        Self {
            state,
            subject: String::new(),
            body: String::new(),
            is_amend: false,
            conflicts_expanded: true,
            staged_expanded: true,
            unstaged_expanded: true,
            untracked_expanded: true,
            co_author_input: String::new(),
            co_authors: Vec::new(),
            signoff_enabled: false,
            template_loaded: false,
        }
    }

    pub fn show(&mut self, ui: &mut Ui) {
        ui.painter().rect_filled(ui.max_rect(), 0.0, theme::bg_surface());

        let (files, conflicted, head_branch, selected_file, repo_path, any_seq_in_progress, git_user) = {
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
                s.git_user_initials.clone(),
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

        egui::ScrollArea::vertical()
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
                            egui::Button::new(
                                RichText::new(format!("{} ⚠ CONFLICTS  ({})", arrow, conflicted.len()))
                                    .color(theme::ACCENT_RED).size(10.5).strong()
                            )
                            .fill(Color32::TRANSPARENT)
                            .stroke(egui::Stroke::NONE),
                        );
                        if toggle.clicked() { *conflicts_expanded = !*conflicts_expanded; }
                    });
                    ui.add_space(2.0);

                    if self.conflicts_expanded {
                        for cf in &conflicted {
                            let is_sel = selected_file.as_deref() == Some(cf.path.as_str());
                            let resp = conflict_file_row(ui, &cf.path, is_sel, |ui| {
                                if ui.add(small_btn_danger("Ours")).clicked()     { do_choose_ours   = Some(cf.path.clone()); }
                                if ui.add(small_btn_danger("Theirs")).clicked()   { do_choose_theirs = Some(cf.path.clone()); }
                                if ui.add(small_btn("Resolved")).clicked()        { do_mark_resolved = Some(cf.path.clone()); }
                            });
                            resp.context_menu(|ui| {
                                if ui.button("Open Merge Tool").clicked() {
                                    do_open_mergetool = Some(cf.path.clone());
                                    ui.close_menu();
                                }
                                if ui.button("Reveal in Folder").clicked() {
                                    reveal_in_folder(&cf.path, &repo_path);
                                    ui.close_menu();
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
                collapsible_section(ui, "UNSTAGED CHANGES", unstaged.len(), unstaged_expanded, |ui| {
                    if ui.add_enabled(!unstaged.is_empty(), small_btn("Stage All")).clicked() {
                        do_stage_all = true;
                    }
                });

                if self.unstaged_expanded {
                    if unstaged.is_empty() {
                        empty_label(ui, "Nothing to stage.");
                    } else {
                        for f in &unstaged {
                            let is_sel = selected_file.as_deref() == Some(f.path.as_str());
                            let resp = file_row(ui, &f.path, f.unstaged_status.unwrap_or('M'), is_sel, |ui| {
                                let btn = if is_sel { small_btn_accent("Stage") } else { small_btn("Stage") };
                                if ui.add(btn).clicked() { do_stage_file = Some(f.path.clone()); }
                                if ui.add(small_btn_danger("Discard")).clicked() {
                                    do_discard_file = Some(f.path.clone());
                                }
                            });
                            resp.context_menu(|ui| {
                                if ui.button("Show File History").clicked() {
                                    load_file_history(&f.path, self.state.clone());
                                    ui.close_menu();
                                }
                                if ui.button("Blame").clicked() {
                                    {
                                        let mut s = self.state.lock().unwrap();
                                        s.selected_file = Some(f.path.clone());
                                        s.show_blame = true;
                                    }
                                    load_blame(&f.path, self.state.clone());
                                    ui.close_menu();
                                }
                                ui.separator();
                                if ui.button("Stash This File").clicked() {
                                    do_stash_file = Some(f.path.clone());
                                    ui.close_menu();
                                }
                                ui.separator();
                                if ui.button("Open in Editor").clicked() {
                                    open_file_in_editor(&f.path, &repo_path);
                                    ui.close_menu();
                                }
                                if ui.button("Reveal in Folder").clicked() {
                                    reveal_in_folder(&f.path, &repo_path);
                                    ui.close_menu();
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
                collapsible_section(ui, "STAGED CHANGES", staged.len(), staged_expanded, |ui| {
                    if ui.add_enabled(!staged.is_empty(), small_btn("Unstage All")).clicked() {
                        do_unstage_all = true;
                    }
                });

                if self.staged_expanded {
                    if staged.is_empty() {
                        empty_label(ui, "Nothing staged for commit.");
                    } else {
                        for f in &staged {
                            let is_sel = selected_file.as_deref() == Some(f.path.as_str());
                            let resp = file_row(ui, &f.path, f.staged_status.unwrap_or('M'), is_sel, |ui| {
                                let btn = if is_sel { small_btn_accent("Unstage") } else { small_btn("Unstage") };
                                if ui.add(btn).clicked() { do_unstage_file = Some(f.path.clone()); }
                            });
                            resp.context_menu(|ui| {
                                if ui.button("Open in Editor").clicked() {
                                    open_file_in_editor(&f.path, &repo_path);
                                    ui.close_menu();
                                }
                                if ui.button("Reveal in Folder").clicked() {
                                    reveal_in_folder(&f.path, &repo_path);
                                    ui.close_menu();
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
                collapsible_section(ui, "UNTRACKED FILES", untracked.len(), untracked_expanded, |_ui| {});

                if self.untracked_expanded {
                    if untracked.is_empty() {
                        empty_label(ui, "No untracked files.");
                    } else {
                        for f in &untracked {
                            let is_sel = selected_file.as_deref() == Some(f.path.as_str());
                            let resp = file_row(ui, &f.path, '?', is_sel, |ui| {
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
                                    ui.close_menu();
                                }
                                if ui.button("Add to .gitignore").clicked() {
                                    do_ignore_file = Some(f.path.clone());
                                    ui.close_menu();
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

                // Hint when a sequencing op is in progress but all conflicts resolved
                if any_seq_in_progress && conflicted.is_empty() {
                    ui.horizontal(|ui| {
                        ui.add_space(12.0);
                        ui.label(RichText::new("✓ All conflicts resolved — stage remaining changes, then click Continue in the banner above.")
                            .color(theme::ACCENT_GREEN).size(11.0));
                    });
                    ui.add_space(6.0);
                }

                // ── COMMIT FORM ──────────────────────────────────────────

                // Load commit template on first open
                if !self.template_loaded && self.subject.is_empty() && self.body.is_empty() && !self.is_amend {
                    self.template_loaded = true;
                    if let Some(ref rp) = repo_path {
                        if let Some(tmpl) = load_commit_template(rp) {
                            let mut lines = tmpl.lines().filter(|l| !l.trim_start().starts_with('#'));
                            self.subject = lines.next().unwrap_or("").trim().to_string();
                            let rest: String = lines.skip_while(|l| l.is_empty()).collect::<Vec<_>>().join("\n");
                            self.body = rest.trim().to_string();
                        }
                    }
                }

                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.label(RichText::new("Commit message").color(theme::text_muted()).size(10.5).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(12.0);
                        let amend_color = if self.is_amend { theme::ACCENT_YELLOW } else { theme::text_muted() };
                        if ui.add(
                            egui::Button::new(RichText::new("Amend").color(amend_color).size(11.0))
                                .fill(if self.is_amend { Color32::from_rgb(0x2A, 0x24, 0x10) } else { Color32::TRANSPARENT })
                                .stroke(egui::Stroke::new(1.0, if self.is_amend { theme::ACCENT_YELLOW } else { theme::border() })),
                        ).clicked() {
                            self.is_amend = !self.is_amend;
                            if self.is_amend && self.subject.is_empty() {
                                if let Some(msg) = staging::load_head_message(self.state.clone()) {
                                    let mut lines = msg.lines();
                                    self.subject = lines.next().unwrap_or("").to_string();
                                    let rest: String = lines.skip_while(|l| l.is_empty()).collect::<Vec<_>>().join("\n");
                                    self.body = rest;
                                }
                            }
                        }
                    });
                });

                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.subject)
                            .desired_width(ui.available_width() - 12.0)
                            .hint_text("Summary (required)")
                            .font(egui::FontId::proportional(12.5)),
                    );
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.add(
                        egui::TextEdit::multiline(&mut self.body)
                            .desired_width(ui.available_width() - 24.0)
                            .desired_rows(3)
                            .hint_text("Description (optional)")
                            .margin(egui::Vec2::new(8.0, 6.0))
                            .font(egui::FontId::proportional(12.0)),
                    );
                    ui.add_space(12.0);
                });

                // ── Co-author trailers ────────────────────────────────────
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.co_author_input)
                            .desired_width(ui.available_width() - 68.0)
                            .hint_text("Co-author: Name <email>")
                            .font(egui::FontId::proportional(11.5)),
                    );
                    ui.add_space(4.0);
                    if ui.add(small_btn("+ Add")).clicked() && !self.co_author_input.trim().is_empty() {
                        self.co_authors.push(self.co_author_input.trim().to_string());
                        self.co_author_input.clear();
                    }
                });
                let mut remove_co_author: Option<usize> = None;
                for (i, author) in self.co_authors.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.add_space(12.0);
                        ui.label(RichText::new(format!("Co-Authored-By: {}", author))
                            .color(theme::text_muted()).size(10.5).monospace());
                        if ui.small_button(RichText::new("×").color(theme::text_muted())).clicked() {
                            remove_co_author = Some(i);
                        }
                    });
                }
                if let Some(i) = remove_co_author { self.co_authors.remove(i); }

                // ── Sign-off + copy ──────────────────────────────────────
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.checkbox(&mut self.signoff_enabled,
                        RichText::new("Sign off").color(theme::text_muted()).size(11.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(12.0);
                        if ui.add(small_btn("📋 Copy")).on_hover_text("Copy assembled commit message").clicked() {
                            let msg = build_message(&self.subject, &self.body, &self.co_authors,
                                self.signoff_enabled, &repo_path, &git_user);
                            ui.output_mut(|o| o.copied_text = msg);
                        }
                    });
                });

                ui.add_space(8.0);

                // ── Branch protection warning ─────────────────────────────
                let is_protected = ["main", "master", "production", "release"]
                    .iter().any(|p| *p == head_branch.as_str());
                if is_protected && !self.is_amend {
                    ui.horizontal(|ui| {
                        ui.add_space(12.0);
                        ui.label(RichText::new(format!(
                            "⚠ Committing directly to \"{}\" — make sure this is intentional.",
                            head_branch
                        )).color(theme::ACCENT_YELLOW).size(11.0));
                    });
                    ui.add_space(4.0);
                }

                let can_commit = !self.subject.trim().is_empty() && !staged.is_empty();
                let btn_label = if self.is_amend {
                    format!("Amend commit on {}", head_branch)
                } else {
                    format!("Commit to {}", head_branch)
                };
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    let btn = ui.add_enabled(
                        can_commit,
                        egui::Button::new(RichText::new(&btn_label).color(Color32::WHITE).size(12.5))
                            .fill(if can_commit { theme::ACCENT_BLUE } else { theme::bg_elevated() })
                            .min_size(Vec2::new(ui.available_width() - 12.0, 28.0)),
                    );
                    if btn.clicked() {
                        if self.is_amend { do_amend = true; } else { do_commit = true; }
                    }
                });

                // ── Reset last commit ─────────────────────────────────────
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.label(RichText::new("Reset last commit:").color(theme::text_muted()).size(10.5));
                    ui.add_space(4.0);
                    if ui.add(small_btn("Soft")).on_hover_text("Keep changes staged").clicked() {
                        staging::reset_soft(self.state.clone());
                    }
                    if ui.add(small_btn("Mixed")).on_hover_text("Keep changes unstaged").clicked() {
                        staging::reset_mixed(self.state.clone());
                    }
                    if ui.add(small_btn_danger("Hard")).on_hover_text("Discard ALL changes — cannot be undone").clicked() {
                        self.state.lock().unwrap().pending_discard = Some(PendingDiscard::ResetHard);
                    }
                });
                ui.add_space(16.0);
            });

        // ── Apply actions ────────────────────────────────────────────────
        if let Some(path) = do_stash_file    { crate::git::branches::stash_file(&path, self.state.clone()); }
        if let Some(path) = do_choose_ours   { conflicts::choose_ours(&path, self.state.clone()); }
        if let Some(path) = do_choose_theirs { conflicts::choose_theirs(&path, self.state.clone()); }
        if let Some(path) = do_mark_resolved { conflicts::mark_resolved(&path, self.state.clone()); }
        if let Some(path) = do_open_mergetool {
            if let Some(ref rp) = repo_path {
                conflicts::open_mergetool(&path, rp);
            }
        }
        if let Some(path) = do_stage_file {
            staging::stage_file(&path, self.state.clone());
        }
        if let Some(path) = do_unstage_file {
            staging::unstage_file(&path, self.state.clone());
        }
        if do_stage_all { staging::stage_all(self.state.clone()); }
        if do_unstage_all { staging::unstage_all(self.state.clone()); }

        if let Some(path) = do_discard_file {
            self.state.lock().unwrap().pending_discard = Some(PendingDiscard::File { path });
        }

        if let Some(path) = do_ignore_file {
            add_to_gitignore(&path, &repo_path);
            // Trigger a reload so the file disappears from untracked
            if let Some(rp) = repo_path.clone() {
                crate::git::operations::load_repo(rp, self.state.clone());
            }
        }

        if do_commit || do_amend {
            let full_msg = build_message(
                &self.subject, &self.body,
                &self.co_authors, self.signoff_enabled,
                &repo_path, &git_user,
            );
            if do_commit {
                staging::create_commit(&full_msg, self.signoff_enabled, self.state.clone());
            } else {
                staging::amend_commit(&full_msg, self.signoff_enabled, self.state.clone());
            }
            self.subject.clear();
            self.body.clear();
            self.co_authors.clear();
            self.is_amend = false;
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
    git_user_initials: &str,
) -> String {
    let mut msg = subject.trim().to_string();
    if !body.trim().is_empty() {
        msg.push_str("\n\n");
        msg.push_str(body.trim());
    }
    // Trailers
    let mut trailers: Vec<String> = co_authors.iter()
        .map(|a| format!("Co-Authored-By: {}", a))
        .collect();
    if signoff {
        // Read full name + email from git config for the sign-off line
        let identity = repo_path.as_ref()
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
    let config = git2::Repository::open(repo_path).ok()?.config().ok()
        .or_else(|| git2::Config::open_default().ok())?;
    let name  = config.get_string("user.name").ok()?;
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
    let editor = std::env::var("VISUAL").or_else(|_| std::env::var("EDITOR")).unwrap_or_else(|_| "xdg-open".into());
    let mut parts = editor.split_whitespace();
    if let Some(prog) = parts.next() {
        let _ = std::process::Command::new(prog).args(parts).arg(full).spawn();
    }
}

fn reveal_in_folder(file_path: &str, repo_path: &Option<std::path::PathBuf>) {
    let Some(rp) = repo_path else { return };
    let dir = rp.join(file_path);
    let dir = dir.parent().unwrap_or(rp.as_path());
    let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
}

fn add_to_gitignore(file_path: &str, repo_path: &Option<std::path::PathBuf>) {
    let Some(rp) = repo_path else { return };
    let gitignore = rp.join(".gitignore");
    let entry = format!("{}\n", file_path);
    let _ = std::fs::OpenOptions::new()
        .create(true).append(true)
        .open(gitignore)
        .map(|mut f| { use std::io::Write; let _ = f.write_all(entry.as_bytes()); });
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
            egui::Button::new(
                RichText::new(format!("{} {}  ({})", arrow, title, count))
                    .color(theme::text_muted()).size(10.5).strong()
            )
            .fill(Color32::TRANSPARENT)
            .stroke(egui::Stroke::NONE),
        );
        if toggle.clicked() { *expanded = !*expanded; }
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
    add_btns: impl FnOnce(&mut Ui),
) -> egui::Response {
    let row_resp = ui.horizontal(|ui| {
        ui.add_space(12.0);
        let (icon, color) = status_icon(status);
        ui.label(RichText::new(icon).font(egui::FontId::monospace(11.0)).color(color));
        ui.add_space(4.0);
        let name = path.rsplit('/').next().unwrap_or(path);
        let label = ui.selectable_label(
            is_selected,
            RichText::new(name)
                .color(if is_selected { theme::bg_app() } else { theme::text_secondary() })
                .size(12.0),
        );
        if path != name { label.on_hover_text(path); }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(8.0);
            add_btns(ui);
        });
    }).response;

    if row_resp.hovered() {
        ui.painter().rect_filled(row_resp.rect, 0.0, Color32::from_white_alpha(5));
    }
    row_resp
}

fn small_btn(label: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(label).color(theme::text_muted()).size(10.5))
        .fill(Color32::TRANSPARENT)
        .stroke(egui::Stroke::new(1.0, theme::border()))
        .min_size(egui::Vec2::new(0.0, 22.0))
}

fn small_btn_accent(label: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(label).color(theme::bg_app()).size(10.5).strong())
        .fill(theme::ACCENT_TEAL)
        .stroke(egui::Stroke::new(1.0, theme::ACCENT_TEAL))
        .min_size(egui::Vec2::new(0.0, 22.0))
}

fn small_btn_danger(label: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(label).color(theme::ACCENT_RED).size(10.5))
        .fill(Color32::TRANSPARENT)
        .stroke(egui::Stroke::new(1.0, theme::ACCENT_RED.gamma_multiply(0.5)))
        .min_size(egui::Vec2::new(0.0, 22.0))
}

fn status_icon(status: char) -> (&'static str, Color32) {
    match status {
        'A' | '?' => ("A", theme::ACCENT_GREEN),
        'D'       => ("D", theme::ACCENT_RED),
        'R'       => ("R", theme::ACCENT_YELLOW),
        _         => ("M", theme::ACCENT_YELLOW),
    }
}

fn conflict_file_row(
    ui: &mut Ui,
    path: &str,
    is_selected: bool,
    add_btns: impl FnOnce(&mut Ui),
) -> egui::Response {
    let row_resp = ui.horizontal(|ui| {
        ui.add_space(12.0);
        ui.label(RichText::new("C").font(egui::FontId::monospace(11.0)).color(theme::ACCENT_RED).strong());
        ui.add_space(4.0);
        let name = path.rsplit('/').next().unwrap_or(path);
        let label = ui.selectable_label(
            is_selected,
            RichText::new(name)
                .color(if is_selected { theme::bg_app() } else { theme::ACCENT_RED.gamma_multiply(0.85) })
                .size(12.0),
        );
        if path != name { label.on_hover_text(path); }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(8.0);
            add_btns(ui);
        });
    }).response;
    if row_resp.hovered() {
        ui.painter().rect_filled(row_resp.rect, 0.0, Color32::from_rgba_unmultiplied(0x4A, 0x18, 0x10, 20));
    }
    row_resp
}

fn empty_label(ui: &mut Ui, text: &str) {
    ui.horizontal(|ui| {
        ui.add_space(20.0);
        ui.label(RichText::new(text).color(theme::text_muted()).size(11.0));
    });
    ui.add_space(2.0);
}
