//! Integrated terminal: runs one shell command at a time in the repository
//! folder and streams its output. Commands get no console window and no
//! keyboard input, so programs that prompt or draw full-screen are not
//! supported; Git's own prompts are disabled and its pager is turned off.
use std::collections::VecDeque;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use command_group::{CommandGroup, GroupChild};
use egui::{FontId, Key, RichText};

use crate::theme;

const MAX_LINES: usize = 5000;
const MAX_LINE_CHARS: usize = 4000;
const MAX_LINE_BYTES: usize = MAX_LINE_CHARS * 4;
const MAX_PENDING_LINES: usize = 256;
const MAX_OUTPUT_BYTES: usize = 2 * 1024 * 1024;
const MAX_TABS: usize = 8;
const MAX_HISTORY: usize = 200;
const OUTPUT_IDLE: std::time::Duration = std::time::Duration::from_secs(2);
const OUTPUT_DEADLINE: std::time::Duration = std::time::Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Command,
    Output,
    Error,
    Info,
}

enum Message {
    Line(LineKind, String),
    Closed,
}

/// Bound both partial lines and queued output, even when a command writes
/// without newlines or produces output faster than the UI can display it.
fn stream_output(mut reader: impl Read, sender: mpsc::SyncSender<Message>, kind: LineKind) {
    let mut chunk = [0; 8192];
    let mut line = Vec::with_capacity(MAX_LINE_BYTES);
    loop {
        let count = match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        for byte in &chunk[..count] {
            if *byte == b'\n' {
                if sender
                    .send(Message::Line(kind, clean(&String::from_utf8_lossy(&line))))
                    .is_err()
                {
                    return;
                }
                line.clear();
            } else if line.len() < MAX_LINE_BYTES {
                line.push(*byte);
            }
        }
    }
    if !line.is_empty() {
        let _ = sender.send(Message::Line(kind, clean(&String::from_utf8_lossy(&line))));
    }
    let _ = sender.send(Message::Closed);
}

struct Running {
    child: GroupChild,
    output: mpsc::Receiver<Message>,
    open_streams: usize,
    repo: PathBuf,
    exit: Option<(std::process::ExitStatus, std::time::Instant)>,
    drain: OutputDrain,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DrainAction {
    Continue,
    StopDescendants,
    Finished,
    TimedOut,
}

struct OutputDrain {
    last_progress: std::time::Instant,
    cleanup_started: Option<std::time::Instant>,
}

impl OutputDrain {
    fn new(now: std::time::Instant) -> Self {
        Self {
            last_progress: now,
            cleanup_started: None,
        }
    }

    fn action(
        &mut self,
        now: std::time::Instant,
        exited_at: Option<std::time::Instant>,
        open_streams: usize,
        received: bool,
    ) -> DrainAction {
        if received {
            self.last_progress = now;
        }
        let Some(exited_at) = exited_at else {
            return DrainAction::Continue;
        };
        if open_streams == 0 {
            return DrainAction::Finished;
        }
        if let Some(cleanup_started) = self.cleanup_started {
            if now.duration_since(self.last_progress) >= OUTPUT_IDLE
                || now.duration_since(cleanup_started) >= OUTPUT_DEADLINE
            {
                return DrainAction::TimedOut;
            }
        } else if now.duration_since(self.last_progress.max(exited_at)) >= OUTPUT_IDLE
            || now.duration_since(exited_at) >= OUTPUT_DEADLINE
        {
            // Stop a descendant that holds inherited pipes open. Keep readers
            // alive afterwards so their buffered output still reaches the UI.
            self.cleanup_started = Some(now);
            self.last_progress = now;
            return DrainAction::StopDescendants;
        }
        DrainAction::Continue
    }
}

fn collect_output(
    output: &mpsc::Receiver<Message>,
    open_streams: &mut usize,
) -> (Vec<(LineKind, String)>, bool) {
    let mut lines = Vec::new();
    let mut received = false;
    for message in output.try_iter().take(MAX_PENDING_LINES) {
        received = true;
        match message {
            Message::Line(kind, text) => lines.push((kind, text)),
            Message::Closed => *open_streams = open_streams.saturating_sub(1),
        }
    }
    (lines, received)
}

/// Process side of the terminal, independent of the UI.
#[derive(Default)]
pub struct Session {
    pub lines: VecDeque<(LineKind, String)>,
    running: Option<Running>,
    output_bytes: usize,
}

fn clean(text: &str) -> String {
    // Drop terminal colour and cursor sequences; keep tabs.
    static ANSI: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let ansi = ANSI.get_or_init(|| {
        regex::Regex::new(r"\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(\x07|\x1b\\)|\x1b.")
            .unwrap()
    });
    let text = ansi.replace_all(text, "");
    let text: String = text
        .trim_end_matches(['\r', '\n'])
        .chars()
        .filter(|c| *c == '\t' || !c.is_control())
        .take(MAX_LINE_CHARS)
        .collect();
    text
}

fn shell(line: &str) -> std::process::Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut command = crate::git::process::background_command("cmd.exe");
        // /S strips only the outer quotes, leaving the line exactly as typed.
        command.args(["/D", "/S", "/C"]);
        command.raw_arg(format!("\"{line}\""));
        command
    }
    #[cfg(not(windows))]
    {
        let mut command = crate::git::process::background_command("sh");
        command.args(["-c", line]);
        command
    }
}

/// Let `git` typed here use Recorde's saved accounts for HTTPS remotes,
/// like Git commands started by the app itself.
fn credential_environment(command: &mut std::process::Command, repo: &Path) {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    if cfg!(test) {
        return;
    }
    let exe = exe.to_string_lossy().replace('\\', "/");
    let quoted = format!("'{}'", exe.replace('\'', "'\\''"));
    command
        .env("GIT_CONFIG_COUNT", "3")
        .env("GIT_CONFIG_KEY_0", "credential.helper")
        .env("GIT_CONFIG_VALUE_0", "")
        .env("GIT_CONFIG_KEY_1", "credential.helper")
        .env(
            "GIT_CONFIG_VALUE_1",
            format!("!{quoted} --credential-helper"),
        )
        .env("GIT_CONFIG_KEY_2", "credential.useHttpPath")
        .env("GIT_CONFIG_VALUE_2", "true")
        .env("RECORDE_CREDENTIAL_REPO", repo);
}

impl Session {
    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    pub fn running_repo(&self) -> Option<&Path> {
        self.running.as_ref().map(|r| r.repo.as_path())
    }

    pub fn push(&mut self, kind: LineKind, text: impl Into<String>) {
        let text: String = text.into().chars().take(MAX_LINE_CHARS).collect();
        self.output_bytes += text.len();
        self.lines.push_back((kind, text));
        while self.lines.len() > MAX_LINES || self.output_bytes > MAX_OUTPUT_BYTES {
            if let Some((_, text)) = self.lines.pop_front() {
                self.output_bytes = self.output_bytes.saturating_sub(text.len());
            }
        }
    }

    pub fn clear(&mut self) {
        self.lines.clear();
        self.output_bytes = 0;
    }

    /// Start `line` in `repo`. Returns false when the line was handled
    /// without starting a process.
    pub fn run(&mut self, repo: &Path, line: &str) -> bool {
        let line = line.trim();
        if line.is_empty() {
            return false;
        }
        if self.running.is_some() {
            self.push(
                LineKind::Info,
                "A command is still running. Stop it before starting another.",
            );
            return false;
        }
        self.push(LineKind::Command, format!("$ {line}"));
        match line.split_whitespace().next().unwrap_or_default() {
            "clear" | "cls" => {
                self.clear();
                return false;
            }
            "cd" | "pushd" => {
                self.push(
                    LineKind::Info,
                    "Commands always run in the repository folder. Use a path in the command instead.",
                );
                return false;
            }
            _ => {}
        }
        let mut command = shell(line);
        command
            .current_dir(repo)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_PAGER", "cat")
            .env("PAGER", "cat")
            .env("GIT_EDITOR", "true")
            .env("TERM", "dumb")
            .env("NO_COLOR", "1");
        credential_environment(&mut command, repo);
        let mut group = command.group();
        // command-group replaces the creation flags when it creates the job.
        #[cfg(windows)]
        group.creation_flags(0x08000000).kill_on_drop(true); // CREATE_NO_WINDOW
        let mut child = match group.spawn() {
            Ok(child) => child,
            Err(error) => {
                self.push(
                    LineKind::Error,
                    format!("Could not start the shell: {error}"),
                );
                return false;
            }
        };
        log::debug!(
            "Terminal command started: pid={}, ports=none requested",
            child.id()
        );
        let (sender, output) = mpsc::sync_channel(MAX_PENDING_LINES);
        let mut open_streams = 0;
        let stdout = child.inner().stdout.take();
        let stderr = child.inner().stderr.take();
        for (kind, stream) in [
            (
                LineKind::Output,
                stdout.map(|s| Box::new(s) as Box<dyn Read + Send>),
            ),
            (
                LineKind::Error,
                stderr.map(|s| Box::new(s) as Box<dyn Read + Send>),
            ),
        ] {
            let Some(stream) = stream else { continue };
            open_streams += 1;
            let sender = sender.clone();
            std::thread::spawn(move || stream_output(stream, sender, kind));
        }
        self.running = Some(Running {
            child,
            output,
            open_streams,
            repo: repo.to_path_buf(),
            exit: None,
            drain: OutputDrain::new(std::time::Instant::now()),
        });
        true
    }

    /// Collect output. Returns the repository of a command that just finished.
    pub fn poll(&mut self) -> Option<PathBuf> {
        let (lines, action) = {
            let running = self.running.as_mut()?;
            let (lines, received) = collect_output(&running.output, &mut running.open_streams);
            if running.exit.is_none() {
                running.exit = running
                    .child
                    .inner()
                    .try_wait()
                    .ok()
                    .flatten()
                    .map(|status| (status, std::time::Instant::now()));
            }
            let action = running.drain.action(
                std::time::Instant::now(),
                running.exit.map(|(_, at)| at),
                running.open_streams,
                received,
            );
            if action == DrainAction::StopDescendants {
                let _ = running.child.kill();
                let _ = running.child.wait();
            }
            (lines, action)
        };
        for (kind, text) in lines {
            self.push(kind, text);
        }
        if !matches!(action, DrainAction::Finished | DrainAction::TimedOut) {
            return None;
        }
        let mut running = self.running.take()?;
        // Background programs the command left behind are stopped with it.
        if running.drain.cleanup_started.is_none() {
            let _ = running.child.kill();
            let _ = running.child.wait();
        }
        for message in running.output.try_iter().take(MAX_PENDING_LINES) {
            if let Message::Line(kind, text) = message {
                self.push(kind, text);
            }
        }
        if action == DrainAction::TimedOut {
            self.push(
                LineKind::Info,
                "Stopped collecting output after the command cleanup deadline.",
            );
        }
        let status = running.exit.and_then(|(status, _)| status.code());
        match status {
            Some(0) => {}
            Some(code) => self.push(LineKind::Info, format!("Exited with code {code}")),
            None => self.push(LineKind::Info, "Stopped"),
        }
        Some(running.repo)
    }

    /// Stop the running command and everything it started.
    pub fn stop(&mut self) -> Option<PathBuf> {
        if let Some(mut running) = self.running.take() {
            if running.drain.cleanup_started.is_none() {
                let _ = running.child.kill();
                let _ = running.child.wait();
            }
            for message in running.output.try_iter().take(MAX_PENDING_LINES) {
                if let Message::Line(kind, text) = message {
                    self.push(kind, text);
                }
            }
            self.push(LineKind::Info, "Stopped");
            return Some(running.repo);
        }
        None
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(mut running) = self.running.take() {
            if running.drain.cleanup_started.is_none() {
                let _ = running.child.kill();
                let _ = running.child.wait();
            }
        }
    }
}

struct CommandTab {
    id: u64,
    repo: PathBuf,
    session: Session,
    input: String,
    history: Vec<String>,
    history_position: Option<usize>,
}

#[derive(Default)]
pub struct TerminalPanel {
    tabs: Vec<CommandTab>,
    selected: Option<u64>,
    next_id: u64,
    focus: bool,
    close_pending: Option<u64>,
    completed: VecDeque<PathBuf>,
}

impl CommandTab {
    fn remember(&mut self, line: &str) {
        if !line.trim().is_empty() && self.history.last().is_none_or(|last| last != line) {
            self.history.push(line.to_string());
            if self.history.len() > MAX_HISTORY {
                self.history.remove(0);
            }
        }
        self.history_position = None;
    }
}

impl TerminalPanel {
    /// Poll every tab, including tabs in repositories that are no longer shown.
    /// Call while the panel is hidden to preserve backpressure and cleanup.
    pub fn poll(&mut self) -> Option<PathBuf> {
        for tab in &mut self.tabs {
            if let Some(path) = tab.session.poll() {
                if !self.completed.contains(&path) {
                    self.completed.push_back(path);
                }
            }
        }
        self.completed.pop_front()
    }

    pub fn is_running(&self) -> bool {
        self.tabs.iter().any(|tab| tab.session.is_running())
    }

    pub fn needs_poll(&self) -> bool {
        self.is_running() || !self.completed.is_empty()
    }

    fn add_tab(&mut self, repo: &Path) -> bool {
        if self.tabs.len() >= MAX_TABS {
            return false;
        }
        self.next_id += 1;
        self.tabs.push(CommandTab {
            id: self.next_id,
            repo: repo.to_path_buf(),
            session: Session::default(),
            input: String::new(),
            history: Vec::new(),
            history_position: None,
        });
        self.selected = Some(self.next_id);
        self.focus = true;
        true
    }

    fn select_repository(&mut self, repo: Option<&Path>) {
        let Some(repo) = repo else {
            self.selected = None;
            return;
        };
        if self
            .tabs
            .iter()
            .any(|tab| Some(tab.id) == self.selected && tab.repo == repo)
        {
            return;
        }
        if let Some(tab) = self.tabs.iter().rev().find(|tab| tab.repo == repo) {
            self.selected = Some(tab.id);
        } else {
            self.selected = None;
            self.add_tab(repo);
        }
    }

    /// Return true when closing the selected tab leaves its repository without
    /// a tab. Hiding the panel then avoids auto-creating a replacement next frame;
    /// an explicit reopen will select/create a tab through `select_repository`.
    fn close_tab(&mut self, id: u64) -> bool {
        let mut hide_panel = false;
        if let Some(index) = self.tabs.iter().position(|tab| tab.id == id) {
            let mut tab = self.tabs.remove(index);
            if let Some(path) = tab.session.stop() {
                if !self.completed.contains(&path) {
                    self.completed.push_back(path);
                }
            }
            if self.selected == Some(id) {
                self.selected = self
                    .tabs
                    .iter()
                    .rev()
                    .find(|other| other.repo == tab.repo)
                    .map(|other| other.id);
                hide_panel = self.selected.is_none();
                self.focus = self.selected.is_some();
            }
        }
        self.close_pending = None;
        hide_panel
    }

    /// Move keyboard focus to the command line on the next frame.
    pub fn focus(&mut self) {
        self.focus = true;
    }

    /// Returns true when the panel should close. `finished` receives the
    /// repository of a command that completed, so the app can refresh it.
    pub fn show(&mut self, ui: &mut egui::Ui, repo: Option<&Path>) -> (bool, Option<PathBuf>) {
        let mut finished = self.poll();
        self.select_repository(repo);
        if self.needs_poll() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
        let mut close = self.show_header(ui, repo);
        ui.horizontal_wrapped(|ui| {
            for tab in &self.tabs {
                if Some(tab.repo.as_path()) != repo {
                    continue;
                }
                let label = format!(
                    "Command {}{}",
                    tab.id,
                    if tab.session.is_running() {
                        " • running"
                    } else {
                        ""
                    }
                );
                if ui
                    .selectable_label(self.selected == Some(tab.id), label)
                    .clicked()
                {
                    self.selected = Some(tab.id);
                    self.focus = true;
                }
            }
            ui.menu_button("All tabs", |ui| {
                let mut remove = None;
                for tab in &self.tabs {
                    ui.horizontal(|ui| {
                        ui.label(format!(
                            "{} · {}{}",
                            tab.id,
                            tab.repo.display(),
                            if tab.session.is_running() {
                                " · running"
                            } else {
                                ""
                            }
                        ));
                        if ui.small_button("Close tab").clicked() {
                            remove = Some((tab.id, tab.session.is_running()));
                        }
                    });
                }
                if let Some((id, running)) = remove {
                    if running {
                        self.close_pending = Some(id);
                    } else {
                        close |= self.close_tab(id);
                    }
                    ui.close();
                }
            });
            ui.label(
                RichText::new("Noninteractive · each command starts in the repository folder")
                    .small()
                    .color(theme::text_muted()),
            );
        });
        if let Some(id) = self.close_pending {
            let description = self.tabs.iter().find(|tab| tab.id == id).map(|tab| {
                format!(
                    "Stop command tab {id} in {} and close it? Completed file changes remain.",
                    tab.repo.display()
                )
            });
            if let Some(description) = description {
                ui.horizontal_wrapped(|ui| {
                    ui.label(description);
                    if ui.button("Stop and close tab").clicked() {
                        close |= self.close_tab(id);
                    }
                    if ui.button("Keep running").clicked() {
                        self.close_pending = None;
                    }
                });
            } else {
                self.close_pending = None;
            }
        }
        let Some(index) = self
            .tabs
            .iter()
            .position(|tab| Some(tab.id) == self.selected && Some(tab.repo.as_path()) == repo)
        else {
            if !close {
                ui.label(if repo.is_none() {
                    "Open a repository to run commands."
                } else {
                    "All 8 command tabs are in use. Close one under All tabs, then choose New tab."
                });
            }
            return (close, finished);
        };
        let mut remove = None;
        let tab = &mut self.tabs[index];
        ui.horizontal_wrapped(|ui| {
            if ui.button("Clear output").clicked() {
                tab.session.clear();
            }
            if ui.button("Copy output").clicked() {
                ui.ctx().copy_text(
                    tab.session
                        .lines
                        .iter()
                        .map(|(_, text)| text.as_str())
                        .collect::<Vec<_>>()
                        .join("\n"),
                );
            }
            if ui.button("Close tab").clicked() {
                remove = Some((tab.id, tab.session.is_running()));
            }
            if tab.session.is_running() {
                if ui.button("Stop").clicked() {
                    if let Some(path) = tab.session.stop() {
                        if finished.is_none() {
                            finished = Some(path);
                        } else if !self.completed.contains(&path) {
                            self.completed.push_back(path);
                        }
                    }
                }
                ui.spinner();
            }
        });
        ui.separator();
        // Lay out bottom-up so the input row is placed first and the output
        // fills exactly the remaining height; the panel never grows itself.
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            Self::show_input(ui, tab, &mut self.focus);
            ui.add_space(2.0);
            egui::ScrollArea::vertical()
                .id_salt(("terminal_output", tab.id))
                .stick_to_bottom(true)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        if tab.session.lines.is_empty() {
                            ui.label(
                                RichText::new(
                                    "Run Git or shell commands in the repository folder. \
                             Commands that ask for input are not supported.",
                                )
                                .color(theme::text_muted())
                                .size(12.0),
                            );
                        }
                        for (kind, text) in &tab.session.lines {
                            let color = match kind {
                                LineKind::Command => theme::accent_teal(),
                                LineKind::Output => theme::text_primary(),
                                LineKind::Error => theme::accent_orange(),
                                LineKind::Info => theme::text_muted(),
                            };
                            ui.add(
                                egui::Label::new(
                                    RichText::new(text)
                                        .font(FontId::monospace(12.5))
                                        .color(color),
                                )
                                .wrap()
                                .selectable(true),
                            );
                        }
                    });
                });
        });
        if let Some((id, running)) = remove {
            if running {
                self.close_pending = Some(id);
            } else {
                close |= self.close_tab(id);
            }
        }
        (close, finished)
    }

    fn show_header(&mut self, ui: &mut egui::Ui, repo: Option<&Path>) -> bool {
        let folder = repo
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "No repository open".into());
        let compact = ui.available_width() < 600.0;
        let title = if ui.available_width() < 360.0 {
            "Terminal"
        } else {
            "Terminal commands"
        };
        let mut hide = false;
        ui.horizontal(|ui| {
            // Reserve the controls first. Long paths can only consume the
            // remaining region, never push buttons offscreen or underlap them.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(8.0);
                hide = ui
                    .button("Hide")
                    .on_hover_text("Hide this panel; running commands continue")
                    .clicked();
                if ui
                    .add_enabled(
                        repo.is_some() && self.tabs.len() < MAX_TABS,
                        egui::Button::new("New tab"),
                    )
                    .on_hover_text("Create a command tab for this repository (up to 8 total)")
                    .clicked()
                {
                    if let Some(repo) = repo {
                        self.add_tab(repo);
                    }
                }
                let available = ui.available_width().max(0.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(available, ui.spacing().interact_size.y),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.add_space(8.0);
                        ui.add(
                            egui::Label::new(
                                RichText::new(title).strong().color(theme::text_primary()),
                            )
                            .truncate(),
                        );
                        if !compact {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&folder).size(12.0).color(theme::text_muted()),
                                )
                                .truncate(),
                            )
                            .on_hover_text(&folder);
                        }
                    },
                );
            });
        });
        if compact {
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                ui.add(
                    egui::Label::new(RichText::new(&folder).size(12.0).color(theme::text_muted()))
                        .truncate(),
                )
                .on_hover_text(&folder);
            });
        }
        hide
    }

    fn show_input(ui: &mut egui::Ui, tab: &mut CommandTab, focus: &mut bool) {
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.label(RichText::new("$").font(FontId::monospace(13.0)));
            // Typing stays possible while a command runs; Enter waits for it.
            let response = ui
                .add(
                    crate::ui::kit::text_field(&mut tab.input)
                        .id(egui::Id::new(("terminal_input", tab.id)))
                        .font(FontId::monospace(13.0))
                        .char_limit(16000)
                        .desired_width(f32::INFINITY)
                        .hint_text("Press Enter to run (for example, git status)"),
                )
                .on_hover_text("Press Enter to run. Up and Down recall command history.");
            if std::mem::take(focus) {
                response.request_focus();
            }
            if response.has_focus() {
                let (up, down) = ui.input_mut(|i| {
                    (
                        i.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
                        i.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
                    )
                });
                if (up || down) && !tab.history.is_empty() {
                    let last = tab.history.len() - 1;
                    let position = match (tab.history_position, up) {
                        (None, true) => Some(last),
                        (None, false) => None,
                        (Some(p), true) => Some(p.saturating_sub(1)),
                        (Some(p), false) if p < last => Some(p + 1),
                        (Some(_), false) => None,
                    };
                    tab.history_position = position;
                    tab.input = position.map(|p| tab.history[p].clone()).unwrap_or_default();
                }
            }
            let submitted = response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            if submitted && tab.session.is_running() {
                *focus = true;
            } else if submitted {
                let line = std::mem::take(&mut tab.input);
                tab.remember(&line);
                tab.session.run(&tab.repo, &line);
                *focus = true;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_repository_header_keeps_controls_visible_without_overlap_at_narrow_widths() {
        let path = PathBuf::from(format!(
            "C:/workspace/{}/repository",
            "very-long-folder/".repeat(24)
        ));
        for width in [280.0, 360.0, 480.0, 960.0] {
            let context = egui::Context::default();
            let mut panel = TerminalPanel::default();
            context.begin_pass(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 180.0),
                )),
                ..Default::default()
            });
            egui::CentralPanel::default().show(
                &mut egui::Ui::new(
                    context.clone(),
                    egui::Id::new("terminal_header_test"),
                    egui::UiBuilder::new().max_rect(context.content_rect()),
                ),
                |ui| {
                    assert!(!panel.show_header(ui, Some(&path)));
                },
            );
            let mut output = context.end_pass();
            output.textures_delta.clear();
            let rect = |label: &str| {
                output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.text() == label => {
                            Some(text.galley.rect.translate(text.pos.to_vec2()))
                        }
                        _ => None,
                    })
                    .unwrap_or_else(|| panic!("Missing {label} at width {width}"))
            };
            let hide = rect("Hide");
            let new_tab = rect("New tab");
            let title = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text)
                        if matches!(text.galley.text(), "Terminal" | "Terminal commands") =>
                    {
                        Some(text.galley.rect.translate(text.pos.to_vec2()))
                    }
                    _ => None,
                })
                .expect("terminal title remains readable");
            assert!(new_tab.right() < hide.left(), "controls overlap at {width}");
            assert!(
                title.right() < new_tab.left(),
                "title overlaps controls at {width}"
            );
            assert!(
                title.left() >= 0.0 && hide.right() <= width,
                "header escapes at {width}"
            );
            let folder = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text().starts_with("C:/workspace/") => {
                        Some(text.galley.rect.translate(text.pos.to_vec2()))
                    }
                    _ => None,
                })
                .expect("repository path is rendered even when truncated");
            assert!(
                folder.left() >= 0.0 && folder.right() <= width,
                "path is not bounded at {width}"
            );
            assert!(
                !folder.intersects(new_tab) && !folder.intersects(hide),
                "path overlaps controls at {width}"
            );
        }
    }

    fn wait(session: &mut Session) -> PathBuf {
        let start = std::time::Instant::now();
        loop {
            if let Some(repo) = session.poll() {
                return repo;
            }
            assert!(start.elapsed().as_secs() < 30, "command did not finish");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    fn output(session: &Session) -> Vec<String> {
        session.lines.iter().map(|(_, text)| text.clone()).collect()
    }

    #[test]
    fn runs_git_in_the_repository_and_reports_failures() {
        let repo = crate::git::test_support::TestRepo::new();
        repo.commit("file.txt", b"content\n");
        let mut session = Session::default();
        assert!(session.run(repo.path(), "git rev-parse --abbrev-ref HEAD"));
        assert_eq!(wait(&mut session), repo.path());
        let lines = output(&session);
        assert_eq!(lines[0], "$ git rev-parse --abbrev-ref HEAD");
        assert!(lines.contains(&"main".to_string()), "{lines:?}");

        assert!(session.run(repo.path(), "git definitely-not-a-command"));
        wait(&mut session);
        assert!(session
            .lines
            .iter()
            .any(|(kind, _)| *kind == LineKind::Error));
        assert!(session
            .lines
            .back()
            .is_some_and(|(_, text)| text.starts_with("Exited with code")));

        assert!(!session.run(repo.path(), "cd .."));
        assert!(!session.run(repo.path(), "clear"));
        assert!(session.lines.is_empty());
    }

    #[test]
    fn stop_ends_a_long_running_command() {
        let repo = crate::git::test_support::TestRepo::new();
        let mut session = Session::default();
        let long = if cfg!(windows) {
            "ping -n 60 127.0.0.1"
        } else {
            "sleep 60"
        };
        assert!(session.run(repo.path(), long));
        assert!(!session.run(repo.path(), "git status"), "one at a time");
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(session.poll().is_none());
        let start = std::time::Instant::now();
        assert_eq!(session.stop(), Some(repo.path().to_path_buf()));
        assert!(!session.is_running());
        assert!(start.elapsed().as_secs() < 10);
        assert_eq!(session.lines.back().unwrap().1, "Stopped");
    }

    #[test]
    fn exited_command_drains_short_line_backlog_and_keeps_the_final_summary() {
        let now = std::time::Instant::now();
        let mut drain = OutputDrain::new(now);
        let (sender, output) = mpsc::sync_channel(MAX_PENDING_LINES);
        let mut streams = 1;
        let mut session = Session::default();
        // Model the bounded reader channel filling once per UI frame after
        // the shell exits. Over 25,000 short lines take longer than the old
        // fixed two-second deadline to drain at the normal per-frame limit.
        for frame in 0..100 {
            for line in 0..MAX_PENDING_LINES {
                sender
                    .send(Message::Line(
                        LineKind::Output,
                        format!("line {}", frame * MAX_PENDING_LINES + line),
                    ))
                    .unwrap();
            }
            let (lines, received) = collect_output(&output, &mut streams);
            assert_eq!(lines.len(), MAX_PENDING_LINES);
            for (kind, text) in lines {
                session.push(kind, text);
            }
            let at = now + std::time::Duration::from_millis(3000 + frame as u64 * 100);
            assert_eq!(
                drain.action(at, Some(now), streams, received),
                DrainAction::Continue
            );
        }
        sender
            .send(Message::Line(LineKind::Error, "final error summary".into()))
            .unwrap();
        sender.send(Message::Closed).unwrap();
        let (lines, received) = collect_output(&output, &mut streams);
        for (kind, text) in lines {
            session.push(kind, text);
        }
        assert_eq!(
            drain.action(
                now + std::time::Duration::from_secs(14),
                Some(now),
                streams,
                received
            ),
            DrainAction::Finished
        );
        assert_eq!(session.lines.len(), MAX_LINES);
        assert_eq!(
            session.lines.back().unwrap(),
            &(LineKind::Error, "final error summary".into())
        );
        assert!(session.output_bytes <= MAX_OUTPUT_BYTES);
        assert!(
            drain.cleanup_started.is_none(),
            "A progressing finite output stream must not be killed"
        );
    }

    #[test]
    fn inherited_output_streams_get_idle_and_absolute_cleanup_deadlines() {
        let now = std::time::Instant::now();
        let mut idle = OutputDrain::new(now);
        let cleanup = now + OUTPUT_IDLE;
        assert_eq!(
            idle.action(cleanup, Some(now), 1, false),
            DrainAction::StopDescendants
        );
        // Tail data delivered by readers after group termination is retained.
        assert_eq!(
            idle.action(cleanup + OUTPUT_IDLE, Some(now), 1, true),
            DrainAction::Continue
        );
        assert_eq!(
            idle.action(cleanup + OUTPUT_IDLE, Some(now), 0, true),
            DrainAction::Finished
        );

        let mut noisy = OutputDrain::new(now);
        let cleanup = now + OUTPUT_DEADLINE;
        assert_eq!(
            noisy.action(cleanup, Some(now), 1, true),
            DrainAction::StopDescendants
        );
        assert_eq!(
            noisy.action(cleanup + OUTPUT_DEADLINE, Some(now), 1, true),
            DrainAction::TimedOut
        );
        let mut stalled = OutputDrain::new(now);
        assert_eq!(
            stalled.action(now + OUTPUT_IDLE, Some(now), 1, false),
            DrainAction::StopDescendants
        );
        assert_eq!(
            stalled.action(now + OUTPUT_IDLE * 2, Some(now), 1, false),
            DrainAction::TimedOut
        );
    }

    #[test]
    fn output_handles_long_lines_and_a_final_line_without_newline() {
        let mut bytes = vec![b'x'; MAX_LINE_BYTES * 100];
        bytes.extend_from_slice(b"\nnext\nlast");
        let (sender, receiver) = mpsc::sync_channel(MAX_PENDING_LINES);
        stream_output(std::io::Cursor::new(bytes), sender, LineKind::Output);
        let lines: Vec<_> = receiver
            .try_iter()
            .filter_map(|message| match message {
                Message::Line(_, line) => Some(line),
                Message::Closed => None,
            })
            .collect();
        assert_eq!(
            lines,
            vec!["x".repeat(MAX_LINE_CHARS), "next".into(), "last".into()]
        );
    }

    #[test]
    fn escape_sequences_and_control_characters_are_removed() {
        assert_eq!(clean("\x1b[31mred\x1b[0m text\r\n"), "red text");
        assert_eq!(clean("a\x07b\tc"), "ab\tc");
        assert_eq!(
            clean(&"x".repeat(MAX_LINE_CHARS + 10)).len(),
            MAX_LINE_CHARS
        );
    }

    #[test]
    fn tabs_keep_repository_input_history_and_output_separate() {
        let mut panel = TerminalPanel::default();
        let first = Path::new("repository-one");
        let second = Path::new("repository-two-worktree");
        panel.select_repository(Some(first));
        let first_id = panel.selected.unwrap();
        panel.tabs[0].input = "git diff --cached".into();
        panel.tabs[0].remember("git status");
        panel.tabs[0].session.push(LineKind::Output, "first output");
        panel.select_repository(Some(second));
        assert_ne!(panel.selected, Some(first_id));
        assert!(panel.tabs[1].input.is_empty());
        assert!(panel.tabs[1].history.is_empty());
        assert!(panel.tabs[1].session.lines.is_empty());
        panel.select_repository(Some(first));
        assert_eq!(panel.selected, Some(first_id));
        assert_eq!(panel.tabs[0].input, "git diff --cached");
        assert_eq!(panel.tabs[0].history, ["git status"]);
        assert_eq!(output(&panel.tabs[0].session), ["first output"]);
        assert!(panel.add_tab(first));
        let new_id = panel.selected.unwrap();
        assert!(
            !panel.close_tab(new_id),
            "a remaining tab keeps the panel visible"
        );
        assert_eq!(panel.selected, Some(first_id));
        panel.select_repository(None);
        assert_eq!(panel.selected, None);
        assert_eq!(panel.tabs.len(), 2);
    }

    #[test]
    fn closing_last_selected_tab_hides_until_reopened_and_preserves_other_repositories() {
        let mut panel = TerminalPanel::default();
        let first = Path::new("first-repository");
        let second = Path::new("second-repository");
        panel.select_repository(Some(first));
        let first_id = panel.selected.unwrap();
        panel.select_repository(Some(second));
        let second_id = panel.selected.unwrap();
        panel.tabs[1].input = "git status".into();
        panel.tabs[1]
            .session
            .push(LineKind::Output, "retained output");
        panel.select_repository(Some(first));
        assert!(
            panel.close_tab(first_id),
            "the app must hide the panel after its last current-repository tab closes"
        );
        assert_eq!(panel.selected, None);
        assert_eq!(panel.tabs.len(), 1);
        // The app polls a hidden panel; that must not create another tab.
        assert!(panel.poll().is_none());
        assert_eq!(panel.tabs.len(), 1);
        assert_eq!(panel.tabs[0].id, second_id);
        assert_eq!(panel.tabs[0].input, "git status");
        assert_eq!(output(&panel.tabs[0].session), ["retained output"]);

        panel.focus();
        panel.select_repository(Some(first));
        assert!(panel
            .selected
            .is_some_and(|id| id != first_id && id != second_id));
        assert_eq!(panel.tabs.len(), 2);
        assert!(
            !panel.close_tab(second_id),
            "closing an inactive repository's tab must not hide the current panel"
        );
        assert!(panel.selected.is_some());
    }

    #[test]
    fn tab_history_and_output_limits_bound_memory_and_refuse_cross_repo_fallback() {
        let mut panel = TerminalPanel::default();
        for index in 0..MAX_TABS {
            assert!(panel.add_tab(Path::new(&format!("repo-{index}"))));
        }
        panel.select_repository(Some(Path::new("different-repository")));
        assert_eq!(
            panel.selected, None,
            "never execute in a previous repository when the tab limit is reached"
        );
        assert_eq!(panel.tabs.len(), MAX_TABS);
        let tab = &mut panel.tabs[0];
        for index in 0..MAX_HISTORY + 10 {
            tab.remember(&format!("command {index}"));
        }
        assert_eq!(tab.history.len(), MAX_HISTORY);
        assert_eq!(tab.history[0], "command 10");
        let wide = "é".repeat(MAX_LINE_CHARS);
        for _ in 0..(MAX_OUTPUT_BYTES / wide.len() + 10) {
            tab.session.push(LineKind::Output, &wide);
        }
        assert!(tab.session.output_bytes <= MAX_OUTPUT_BYTES);
        assert_eq!(
            tab.session.output_bytes,
            tab.session
                .lines
                .iter()
                .map(|(_, line)| line.len())
                .sum::<usize>()
        );
        tab.session.clear();
        assert_eq!(tab.session.output_bytes, 0);
    }

    #[test]
    fn polling_drains_hidden_tabs_and_reports_each_repository() {
        let first = crate::git::test_support::TestRepo::new();
        let second = crate::git::test_support::TestRepo::new();
        first
            .open()
            .config()
            .unwrap()
            .set_str("recorde.fixture", "first-repository")
            .unwrap();
        second
            .open()
            .config()
            .unwrap()
            .set_str("recorde.fixture", "second-repository")
            .unwrap();
        let mut panel = TerminalPanel::default();
        panel.add_tab(first.path());
        panel.add_tab(second.path());
        for tab in &mut panel.tabs {
            assert!(tab
                .session
                .run(&tab.repo, "git config --local recorde.fixture"));
        }
        assert!(panel.is_running());
        let start = std::time::Instant::now();
        let mut completed = Vec::new();
        while completed.len() < 2 {
            if let Some(repo) = panel.poll() {
                completed.push(repo);
            }
            assert!(start.elapsed().as_secs() < 30);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(!panel.is_running());
        assert!(completed.contains(&first.path().to_path_buf()));
        assert!(completed.contains(&second.path().to_path_buf()));
        assert!(output(&panel.tabs[0].session).contains(&"first-repository".to_string()));
        assert!(!output(&panel.tabs[0].session).contains(&"second-repository".to_string()));
        assert!(output(&panel.tabs[1].session).contains(&"second-repository".to_string()));
    }
}
