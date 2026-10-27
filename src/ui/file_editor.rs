//! Built-in editor for one working-tree text file. Saving refuses to
//! overwrite changes another program made after the file was opened.
use std::path::{Path, PathBuf};

use egui::{FontId, Key, RichText};

use crate::git::safe_path::ScopedFile;
use crate::state::SharedState;
use crate::theme;

const LIMIT: u64 = 2 * 1024 * 1024;

pub struct FileEditor {
    repo: PathBuf,
    pub file: String,
    /// Bytes on disk when opened or last saved.
    original: Vec<u8>,
    crlf: bool,
    pub text: String,
    saved_text: String,
    error: Option<String>,
    confirm_close: bool,
    confirm_reload: bool,
}

fn decode(bytes: &[u8]) -> Result<(String, bool), String> {
    if bytes.contains(&0) {
        return Err("Binary files cannot be edited here.".into());
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "Only UTF-8 text files can be edited here.".to_string())?;
    // Edit with LF; write CRLF back only when the file used it throughout.
    let lines = text.matches('\n').count();
    let crlf = lines > 0 && text.matches("\r\n").count() == lines;
    Ok((
        if crlf {
            text.replace("\r\n", "\n")
        } else {
            text.to_string()
        },
        crlf,
    ))
}

impl FileEditor {
    pub fn open(repo: &Path, file: &str) -> Result<Self, String> {
        let original = ScopedFile::new(repo, file)?
            .read(LIMIT)?
            .ok_or_else(|| format!("{file} no longer exists."))?;
        let (text, crlf) = decode(&original)?;
        Ok(Self {
            repo: repo.to_path_buf(),
            file: file.to_string(),
            original,
            crlf,
            saved_text: text.clone(),
            text,
            error: None,
            confirm_close: false,
            confirm_reload: false,
        })
    }

    pub fn is_dirty(&self) -> bool {
        self.text != self.saved_text
    }

    fn text_edit_id(&self) -> egui::Id {
        egui::Id::new(("file_editor_text", &self.repo, &self.file))
    }

    fn encoded(&self) -> Vec<u8> {
        if self.crlf {
            self.text.replace('\n', "\r\n").into_bytes()
        } else {
            self.text.clone().into_bytes()
        }
    }

    pub fn save(&mut self) -> Result<(), String> {
        let bytes = self.encoded();
        ScopedFile::new(&self.repo, &self.file)?.replace(Some(&self.original), &bytes)?;
        self.original = bytes;
        self.saved_text = self.text.clone();
        Ok(())
    }

    /// Reload from disk, dropping unsaved edits.
    pub fn reload(&mut self) -> Result<(), String> {
        let fresh = Self::open(&self.repo, &self.file)?;
        *self = fresh;
        Ok(())
    }

    fn request_reload(&mut self) {
        self.confirm_close = false;
        if self.is_dirty() {
            self.confirm_reload = true;
        } else {
            self.reload_from_disk();
        }
    }

    fn reload_from_disk(&mut self) {
        // reload constructs a complete replacement before modifying this editor.
        // Keep both the unsaved buffer and retry confirmation if reading fails.
        if let Err(error) = self.reload() {
            self.error = Some(error);
        }
    }

    /// Returns true when the editor should close.
    pub fn show(&mut self, ctx: &egui::Context, state: SharedState) -> bool {
        let mut close = false;
        let mut open = true;
        let title = format!(
            "Edit {}{}",
            self.file,
            if self.is_dirty() { " •" } else { "" }
        );
        egui::Window::new(title)
            .id(egui::Id::new(("file_editor", &self.repo, &self.file)))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([780.0, 560.0])
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                let save_key = ui.is_enabled()
                    && ui.memory(|memory| memory.allows_interaction(ui.layer_id()))
                    && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, Key::S));
                ui.horizontal(|ui| {
                    let save = ui
                        .add_enabled(self.is_dirty(), egui::Button::new("Save"))
                        .on_hover_text("Save  (Ctrl+S)")
                        .clicked();
                    if (save || save_key) && self.is_dirty() {
                        match self.save() {
                            Ok(()) => {
                                self.error = None;
                                crate::git::operations::refresh_worktree(state.clone());
                            }
                            Err(error) => self.error = Some(error),
                        }
                    }
                    if ui
                        .button("Reload")
                        .on_hover_text(
                            "Read the latest file from disk; unsaved edits require confirmation",
                        )
                        .clicked()
                    {
                        self.request_reload();
                    }
                    if ui.button("Close").clicked() {
                        self.confirm_reload = false;
                        if self.is_dirty() {
                            self.confirm_close = true;
                        } else {
                            close = true;
                        }
                    }
                    ui.label(
                        RichText::new(if self.crlf { "CRLF" } else { "LF" })
                            .size(12.0)
                            .color(theme::text_muted()),
                    );
                });
                if self.confirm_close {
                    ui.horizontal(|ui| {
                        ui.colored_label(theme::accent_orange(), "Discard unsaved changes?");
                        if ui.button("Discard").clicked() {
                            close = true;
                        }
                        if ui.button("Keep editing").clicked() {
                            self.confirm_close = false;
                        }
                    });
                }
                if self.confirm_reload {
                    ui.horizontal_wrapped(|ui| {
                        ui.colored_label(
                            theme::accent_orange(),
                            "Discard unsaved changes and reload from disk?",
                        );
                        if ui.button("Discard and reload").clicked() {
                            self.reload_from_disk();
                        }
                        if ui.button("Keep editing").clicked() {
                            self.confirm_reload = false;
                        }
                    });
                }
                if let Some(error) = &self.error {
                    ui.colored_label(theme::accent_red(), error);
                }
                ui.separator();
                egui::ScrollArea::both()
                    .id_salt(("file_editor_scroll", &self.repo, &self.file))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let id = self.text_edit_id();
                        ui.add(
                            egui::TextEdit::multiline(&mut self.text)
                                .id(id)
                                .code_editor()
                                .font(FontId::monospace(13.0))
                                .desired_width(f32::INFINITY)
                                .desired_rows(28)
                                .lock_focus(true),
                        );
                    });
            });
        if !open {
            self.confirm_reload = false;
            if self.is_dirty() {
                self.confirm_close = true;
            } else {
                close = true;
            }
        }
        close
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_history_does_not_cross_repositories_with_the_same_relative_file() {
        let editor = |repo: &str, text: &str| FileEditor {
            repo: repo.into(),
            file: "notes.txt".into(),
            original: text.as_bytes().to_vec(),
            crlf: false,
            text: text.into(),
            saved_text: text.into(),
            error: None,
            confirm_close: false,
            confirm_reload: false,
        };
        let mut first = editor("first-repository", "first original");
        let mut second = editor("second-repository", "second original");
        let ctx = egui::Context::default();
        let state = crate::state::new_shared_state();
        let frame = |editor: &mut FileEditor, time, events| {
            ctx.begin_pass(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 800.0),
                )),
                time: Some(time),
                events,
                ..Default::default()
            });
            ctx.memory_mut(|memory| memory.request_focus(editor.text_edit_id()));
            assert!(!editor.show(&ctx, state.clone()));
            ctx.end_pass().textures_delta.clear();
        };
        let command = |key| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        };
        let release = |key| egui::Event::Key {
            key,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        };
        frame(&mut first, 0.0, vec![]);
        frame(&mut first, 0.1, vec![]);
        frame(
            &mut first,
            0.2,
            vec![command(Key::A), egui::Event::Text("first edited".into())],
        );
        frame(&mut first, 1.5, vec![release(Key::A)]);
        assert_eq!(first.text, "first edited");
        frame(&mut second, 2.0, vec![]);
        frame(&mut second, 3.0, vec![command(Key::Z)]);
        assert_eq!(
            second.text, "second original",
            "Undo must not insert content from another repository"
        );
        frame(&mut first, 4.0, vec![release(Key::Z)]);
        frame(&mut first, 5.0, vec![command(Key::Z)]);
        assert_eq!(
            first.text, "first original",
            "Returning to the first repository retains its own undo history"
        );
    }

    #[test]
    fn edits_round_trip_and_keep_crlf() {
        let repo = crate::git::test_support::TestRepo::new();
        repo.write("notes.txt", b"one\r\ntwo\r\n");
        let mut editor = FileEditor::open(repo.path(), "notes.txt").unwrap();
        assert_eq!(editor.text, "one\ntwo\n");
        assert!(!editor.is_dirty());
        editor.text.push_str("three\n");
        assert!(editor.is_dirty());
        editor.save().unwrap();
        assert!(!editor.is_dirty());
        assert_eq!(
            std::fs::read(repo.path().join("notes.txt")).unwrap(),
            b"one\r\ntwo\r\nthree\r\n"
        );
    }

    #[test]
    fn saving_refuses_to_overwrite_outside_changes() {
        let repo = crate::git::test_support::TestRepo::new();
        repo.write("file.txt", b"original\n");
        let mut editor = FileEditor::open(repo.path(), "file.txt").unwrap();
        editor.text = "mine\n".into();
        repo.write("file.txt", b"theirs\n");
        assert!(editor.save().is_err());
        assert_eq!(
            std::fs::read(repo.path().join("file.txt")).unwrap(),
            b"theirs\n"
        );
        editor.reload().unwrap();
        assert_eq!(editor.text, "theirs\n");
    }

    #[test]
    fn reload_picks_up_external_changes_and_requires_confirmation_for_unsaved_text() {
        let repo = crate::git::test_support::TestRepo::new();
        repo.write("file.txt", b"original\n");
        let mut editor = FileEditor::open(repo.path(), "file.txt").unwrap();
        repo.write("file.txt", b"external update\n");
        editor.request_reload();
        assert_eq!(editor.text, "external update\n");
        assert!(!editor.is_dirty());
        assert!(!editor.confirm_reload);

        editor.text = "unsaved draft\n".into();
        editor.confirm_close = true;
        repo.write("file.txt", b"new disk contents\n");
        editor.request_reload();
        assert!(editor.confirm_reload);
        assert!(
            !editor.confirm_close,
            "Close and reload have distinct confirmations"
        );
        assert_eq!(editor.text, "unsaved draft\n");
        assert_eq!(editor.saved_text, "external update\n");
        assert!(editor.is_dirty());

        // The Keep editing action changes only the pending decision.
        editor.confirm_reload = false;
        assert_eq!(editor.text, "unsaved draft\n");
        editor.request_reload();
        editor.reload_from_disk();
        assert_eq!(editor.text, "new disk contents\n");
        assert!(!editor.is_dirty());
        assert!(!editor.confirm_reload);
        assert_eq!(
            std::fs::read(repo.path().join("file.txt")).unwrap(),
            b"new disk contents\n"
        );
    }

    #[test]
    fn failed_confirmed_reload_keeps_unsaved_data_and_error_until_a_successful_retry() {
        let repo = crate::git::test_support::TestRepo::new();
        repo.write("file.txt", b"original\r\n");
        let mut editor = FileEditor::open(repo.path(), "file.txt").unwrap();
        editor.text = "irreplaceable draft\n".into();
        editor.request_reload();
        repo.write("file.txt", b"\0binary replacement");
        editor.reload_from_disk();
        assert_eq!(editor.text, "irreplaceable draft\n");
        assert_eq!(editor.saved_text, "original\n");
        assert_eq!(editor.original, b"original\r\n");
        assert!(editor.crlf && editor.is_dirty() && editor.confirm_reload);
        assert!(editor.error.as_deref().unwrap().contains("Binary"));
        assert_eq!(
            std::fs::read(repo.path().join("file.txt")).unwrap(),
            b"\0binary replacement"
        );

        repo.write("file.txt", b"readable again\n");
        editor.reload_from_disk();
        assert_eq!(editor.text, "readable again\n");
        assert!(!editor.is_dirty());
        assert!(!editor.crlf && !editor.confirm_reload);
        assert!(editor.error.is_none());
    }

    #[test]
    fn binary_non_utf8_and_unsafe_paths_are_refused() {
        let repo = crate::git::test_support::TestRepo::new();
        repo.write("image.bin", b"\x89PNG\0\0");
        repo.write("latin1.txt", b"caf\xe9\n");
        assert!(FileEditor::open(repo.path(), "image.bin").is_err());
        assert!(FileEditor::open(repo.path(), "latin1.txt").is_err());
        assert!(FileEditor::open(repo.path(), "../outside").is_err());
        assert!(FileEditor::open(repo.path(), ".git/config").is_err());
        assert!(FileEditor::open(repo.path(), "missing.txt").is_err());
    }
}
