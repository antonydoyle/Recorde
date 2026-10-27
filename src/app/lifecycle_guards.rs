use super::*;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub(super) enum Navigation {
    Close(PathBuf),
    Workspace(Vec<PathBuf>),
}

impl Navigation {
    fn removes(&self, path: &Path) -> bool {
        match self {
            Self::Close(closed) => path == closed,
            Self::Workspace(retained) => !retained.iter().any(|p| p == path),
        }
    }
}

fn repository_work_pending(state: &crate::state::AppState) -> bool {
    !state.local_jobs.is_empty()
        || !state.active_ops.is_empty()
        || state.remote_op.is_some()
        || state.repo_load_path.is_some()
}

fn dirty_file(session: &RepoUiSession) -> Option<&str> {
    session
        .file_editor
        .as_ref()
        .filter(|editor| editor.is_dirty())
        .map(|editor| editor.file.as_str())
}

impl RecordeApp {
    pub(super) fn background_work_pending(&self, state: &crate::state::AppState) -> bool {
        repository_work_pending(state)
            || self.runner.has_any_jobs()
            || self.terminal.is_running()
            || self.account_task.is_some()
            || self.ssh_task.is_some()
            || self.repository_browser.is_busy()
            || self.history_plan.is_some()
    }

    fn dirty_editors(&self) -> Vec<(PathBuf, String)> {
        let mut editors: Vec<_> = self
            .repo_sessions
            .iter()
            .filter_map(|(path, session)| {
                dirty_file(session).map(|file| (path.clone(), file.to_owned()))
            })
            .collect();
        if let (Some(path), Some(file)) = (&self.observed_repo, dirty_file(&self.repo_ui)) {
            editors.push((path.clone(), file.to_owned()));
        }
        editors.sort();
        editors.dedup();
        editors
    }

    /// Shared by native window close and File > Quit. Never tear down workers
    /// in the middle of a repository write or silently kill a command tab.
    pub(super) fn request_quit(&mut self, ctx: &Context) {
        let busy = self.background_work_pending(&self.state.lock().unwrap());
        if busy {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.quit_waiting = true;
        } else if let Some((path, file)) = self.dirty_editors().first() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.state.lock().unwrap().error = Some(format!(
                "Save or close the editor for {file} in {} before quitting. Reopen that repository if its tab is closed.", path.display()
            ));
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    pub(super) fn show_lifecycle_guards(&mut self, ctx: &Context) {
        if self.quit_waiting {
            let busy = self.background_work_pending(&self.state.lock().unwrap());
            let mut cancel = false;
            egui::Modal::new(egui::Id::new("wait_before_quit")).show(ctx, |ui| {
                ui.set_max_width(440.0);
                ui.heading("Waiting to quit");
                ui.label("Recorde will close after its running and queued operations finish. Use Keep working to stop commands or cancel operations from their controls.");
                ui.spinner();
                cancel = ui.button("Keep working").clicked() || ui.input(|i| i.key_pressed(Key::Escape));
            });
            if cancel {
                self.quit_waiting = false;
            } else if !busy {
                self.quit_waiting = false;
                self.request_quit(ctx);
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        let Some(navigation) = self.pending_navigation.clone() else {
            return;
        };
        let affected: Vec<_> = self
            .dirty_editors()
            .into_iter()
            .filter(|(path, _)| navigation.removes(path))
            .collect();
        let mut discard = false;
        let mut cancel = false;
        egui::Modal::new(egui::Id::new("repository_draft_navigation")).show(ctx, |ui| {
            ui.set_max_width(540.0);
            ui.heading("Unsaved file edits");
            ui.label("These repository tabs contain unsaved edits. Keep editing, or discard the listed drafts before continuing.");
            for (path, file) in &affected { ui.label(format!("{file} — {}", path.display())); }
            ui.horizontal(|ui| {
                cancel = ui.button("Keep editing").clicked() || ui.input(|i| i.key_pressed(Key::Escape));
                discard = ui.add(danger_btn("Discard drafts and continue")).clicked();
            });
        });
        if cancel {
            self.pending_navigation = None;
        }
        if discard {
            for (path, _) in affected {
                if self.observed_repo.as_ref() == Some(&path) {
                    self.repo_ui.file_editor = None;
                }
                if let Some(session) = self.repo_sessions.get_mut(&path) {
                    session.file_editor = None;
                }
            }
            self.pending_navigation = None;
            self.apply_navigation(navigation);
        }
    }

    pub(super) fn request_navigation(&mut self, navigation: Navigation) {
        if self
            .dirty_editors()
            .iter()
            .any(|(path, _)| navigation.removes(path))
        {
            self.pending_navigation = Some(navigation);
        } else {
            self.apply_navigation(navigation);
        }
    }

    fn apply_navigation(&mut self, navigation: Navigation) {
        let active = self.state.lock().unwrap().repo_path.clone();
        let open = match navigation {
            Navigation::Close(path) => {
                self.config.open_repos.retain(|p| p != &path);
                (active.as_ref() == Some(&path))
                    .then(|| self.config.open_repos.last().cloned())
                    .flatten()
            }
            Navigation::Workspace(paths) => {
                let open = paths.first().cloned();
                self.config.open_repos = paths;
                open
            }
        };
        self.save_config();
        if let Some(path) = open {
            self.config.add_recent(path.clone());
            load_repo(path, self.state.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draft_navigation_only_affects_dirty_editors_in_removed_repositories() {
        let active = crate::git::test_support::TestRepo::new();
        let hidden = crate::git::test_support::TestRepo::new();
        let clean = crate::git::test_support::TestRepo::new();
        let make_session = |repo: &crate::git::test_support::TestRepo, draft: Option<&str>| {
            repo.write("draft.txt", b"saved content\n");
            let mut editor =
                crate::ui::file_editor::FileEditor::open(repo.path(), "draft.txt").unwrap();
            if let Some(draft) = draft {
                editor.text = draft.into();
            }
            let mut session = RepoUiSession::default();
            session.file_editor = Some(editor);
            session
        };
        let active_session = make_session(&active, Some("active unsaved\n"));
        let hidden_session = make_session(&hidden, Some("hidden unsaved\n"));
        let clean_session = make_session(&clean, None);
        let sessions = [
            (active.path(), &active_session),
            (hidden.path(), &hidden_session),
            (clean.path(), &clean_session),
        ];
        let affected = |navigation: Navigation| -> Vec<PathBuf> {
            sessions
                .iter()
                .filter_map(|(path, session)| {
                    (dirty_file(session).is_some() && navigation.removes(path))
                        .then(|| path.to_path_buf())
                })
                .collect()
        };

        assert_eq!(dirty_file(&active_session), Some("draft.txt"));
        assert_eq!(dirty_file(&hidden_session), Some("draft.txt"));
        assert_eq!(dirty_file(&clean_session), None);
        assert_eq!(
            affected(Navigation::Close(active.path().into())),
            vec![active.path().to_path_buf()]
        );
        assert_eq!(
            affected(Navigation::Close(hidden.path().into())),
            vec![hidden.path().to_path_buf()]
        );
        assert!(affected(Navigation::Close(clean.path().into())).is_empty());
        assert_eq!(
            affected(Navigation::Workspace(vec![
                active.path().into(),
                clean.path().into()
            ])),
            vec![hidden.path().to_path_buf()],
            "keeping the active tab must still guard the removed hidden draft",
        );
        assert_eq!(
            affected(Navigation::Workspace(vec![hidden.path().into()])),
            vec![active.path().to_path_buf()],
        );
        assert!(affected(Navigation::Workspace(vec![
            active.path().into(),
            hidden.path().into()
        ]))
        .is_empty());
        assert_eq!(
            affected(Navigation::Workspace(Vec::new())),
            vec![active.path().to_path_buf(), hidden.path().to_path_buf()],
        );
        // Merely reviewing/cancelling navigation must not mutate either draft
        // or the identically named files belonging to separate repositories.
        assert_eq!(
            active_session.file_editor.as_ref().unwrap().text,
            "active unsaved\n"
        );
        assert_eq!(
            hidden_session.file_editor.as_ref().unwrap().text,
            "hidden unsaved\n"
        );
        for (path, _) in sessions {
            assert_eq!(
                std::fs::read(path.join("draft.txt")).unwrap(),
                b"saved content\n"
            );
        }
    }

    #[test]
    fn lifecycle_modal_does_not_let_editor_save_shortcut_modify_a_draft() {
        let repo = crate::git::test_support::TestRepo::new();
        repo.write("draft.txt", b"saved content\n");
        let mut editor =
            crate::ui::file_editor::FileEditor::open(repo.path(), "draft.txt").unwrap();
        editor.text = "unsaved content\n".into();
        let state = crate::state::new_shared_state();
        let ctx = egui::Context::default();
        for frame in 0..3 {
            let events = if frame == 2 {
                vec![egui::Event::Key {
                    key: Key::S,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::COMMAND,
                }]
            } else {
                Vec::new()
            };
            ctx.begin_pass(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100.0, 800.0),
                )),
                events,
                ..Default::default()
            });
            // Match application order: lifecycle Modal is rendered before the
            // existing editor window; modal state must block its raw shortcut.
            egui::Modal::new(egui::Id::new("repository_draft_navigation")).show(&ctx, |ui| {
                ui.heading("Unsaved file edits");
                let _ = ui.button("Keep editing");
            });
            let closed = editor.show(&ctx, state.clone());
            ctx.end_pass().textures_delta.clear();
            assert!(!closed);
        }
        assert!(editor.is_dirty(), "the modal must retain the unsaved draft");
        assert_eq!(editor.text, "unsaved content\n");
        assert_eq!(
            std::fs::read(repo.path().join("draft.txt")).unwrap(),
            b"saved content\n"
        );
    }

    #[test]
    fn exit_guard_covers_queued_jobs_and_repository_replacement() {
        let mut state = crate::state::AppState::default();
        assert!(!repository_work_pending(&state));
        state.repo_load_path = Some("other-repository".into());
        assert!(repository_work_pending(&state));
        state.repo_load_path = None;
        state.local_jobs.push(crate::git::mutation::LocalJob {
            id: 1,
            path: "repository".into(),
            label: "Restore".into(),
            running: false,
            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        });
        assert!(
            repository_work_pending(&state),
            "queued work must also prevent exit"
        );
        state.local_jobs[0].running = true;
        assert!(repository_work_pending(&state));
        state.local_jobs.clear();
        assert!(!repository_work_pending(&state));
    }
}
