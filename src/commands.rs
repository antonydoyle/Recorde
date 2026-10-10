//! Shared command identities, discovery and portable shortcut preferences.
use egui::{Key, Modifiers};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Palette,
    Open,
    Clone,
    Init,
    Settings,
    Shortcuts,
    Accounts,
    Theme,
    Fetch,
    Pull,
    PullRebase,
    PullFf,
    Push,
    PushLease,
    PushUpstream,
    Branch,
    SwitchBranch,
    Merge,
    Rebase,
    Stash,
    Search,
    DiffSearch,
    AllBranches,
    Reflog,
    Bisect,
    Provider,
    Refresh,
    Terminal,
    Reveal,
    CopyPath,
    Sidebar,
    Graph,
    Changes,
    Diff,
    NewTag,
    CopyCommit,
    InteractiveRebase,
    PreviewCherryPick,
    PreviewGraphRebase,
}
impl Command {
    pub const ALL: &'static [Self] = &[
        Self::Palette,
        Self::Open,
        Self::Clone,
        Self::Init,
        Self::Settings,
        Self::Shortcuts,
        Self::Accounts,
        Self::Theme,
        Self::Fetch,
        Self::Pull,
        Self::PullRebase,
        Self::PullFf,
        Self::Push,
        Self::PushLease,
        Self::PushUpstream,
        Self::Branch,
        Self::SwitchBranch,
        Self::Merge,
        Self::Rebase,
        Self::Stash,
        Self::Search,
        Self::DiffSearch,
        Self::AllBranches,
        Self::Reflog,
        Self::Bisect,
        Self::Provider,
        Self::Refresh,
        Self::Terminal,
        Self::Reveal,
        Self::CopyPath,
        Self::Sidebar,
        Self::Graph,
        Self::Changes,
        Self::Diff,
        Self::NewTag,
        Self::CopyCommit,
        Self::InteractiveRebase,
        Self::PreviewCherryPick,
        Self::PreviewGraphRebase,
    ];
    pub fn id(self) -> &'static str {
        match self {
            Self::Palette => "Palette",
            Self::Open => "Open",
            Self::Clone => "Clone",
            Self::Init => "Init",
            Self::Settings => "Settings",
            Self::Shortcuts => "Shortcuts",
            Self::Accounts => "Accounts",
            Self::Theme => "Theme",
            Self::Fetch => "Fetch",
            Self::Pull => "Pull",
            Self::PullRebase => "PullRebase",
            Self::PullFf => "PullFf",
            Self::Push => "Push",
            Self::PushLease => "PushLease",
            Self::PushUpstream => "PushUpstream",
            Self::Branch => "Branch",
            Self::SwitchBranch => "SwitchBranch",
            Self::Merge => "Merge",
            Self::Rebase => "Rebase",
            Self::Stash => "Stash",
            Self::Search => "Search",
            Self::DiffSearch => "DiffSearch",
            Self::AllBranches => "AllBranches",
            Self::Reflog => "Reflog",
            Self::Bisect => "Bisect",
            Self::Provider => "Provider",
            Self::Refresh => "Refresh",
            Self::Terminal => "Terminal",
            Self::Reveal => "Reveal",
            Self::CopyPath => "CopyPath",
            Self::Sidebar => "Sidebar",
            Self::Graph => "Graph",
            Self::Changes => "Changes",
            Self::Diff => "Diff",
            Self::NewTag => "NewTag",
            Self::CopyCommit => "CopyCommit",
            Self::InteractiveRebase => "InteractiveRebase",
            Self::PreviewCherryPick => "PreviewCherryPick",
            Self::PreviewGraphRebase => "PreviewGraphRebase",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Palette => "Command palette",
            Self::Open => "Open repository",
            Self::Clone => "Clone repository",
            Self::Init => "Initialize repository",
            Self::Settings => "App settings",
            Self::Shortcuts => "Keyboard shortcuts",
            Self::Accounts => "Accounts and SSH",
            Self::Theme => "Toggle theme",
            Self::Fetch => "Fetch all remotes",
            Self::Pull => "Pull",
            Self::PullRebase => "Pull with rebase",
            Self::PullFf => "Pull fast-forward only",
            Self::Push => "Push",
            Self::PushLease => "Push with lease",
            Self::PushUpstream => "Publish branch / set upstream",
            Self::Branch => "Create branch",
            Self::SwitchBranch => "Switch branch",
            Self::Merge => "Merge branch",
            Self::Rebase => "Rebase current branch",
            Self::Stash => "Stash changes",
            Self::Search => "Search commits",
            Self::DiffSearch => "Find in diff",
            Self::AllBranches => "Toggle all branches",
            Self::Reflog => "Open reflog",
            Self::Bisect => "Bisect",
            Self::Provider => "Issues, CI and releases",
            Self::Refresh => "Refresh repository",
            Self::Terminal => "Open terminal",
            Self::Reveal => "Reveal repository in file manager",
            Self::CopyPath => "Copy repository path",
            Self::Sidebar => "Focus sidebar",
            Self::Graph => "Focus commit graph",
            Self::Changes => "Focus changes",
            Self::Diff => "Focus diff",
            Self::NewTag => "Create tag at selected commit",
            Self::CopyCommit => "Copy selected commit SHA",
            Self::InteractiveRebase => "Interactive rebase from selected commit",
            Self::PreviewCherryPick => "Preview cherry-pick selected commit",
            Self::PreviewGraphRebase => "Preview rebase onto selected commit",
        }
    }
    pub fn disabled_reason(self, s: &crate::state::AppState) -> Option<&'static str> {
        if s.pending_discard.is_some() || !matches!(s.modal, crate::state::ModalKind::None) {
            return Some("Finish or cancel the open confirmation first");
        }
        if matches!(
            self,
            Self::Palette
                | Self::Open
                | Self::Clone
                | Self::Init
                | Self::Settings
                | Self::Shortcuts
                | Self::Accounts
                | Self::Theme
        ) {
            return None;
        }
        if s.repo_path.is_none() {
            return Some("Open a repository first");
        }
        if s.repo_load_path.is_some() {
            return Some("Repository is loading");
        }
        if matches!(
            self,
            Self::NewTag
                | Self::CopyCommit
                | Self::InteractiveRebase
                | Self::PreviewCherryPick
                | Self::PreviewGraphRebase
        ) && s.selected_commit.is_none()
        {
            return Some("Select a commit first");
        }
        if self == Self::Provider
            && !s
                .capabilities
                .as_ref()
                .is_some_and(|c| c.provider_target.is_some())
        {
            return Some("No supported provider remote");
        }
        let mutating = matches!(
            self,
            Self::Fetch
                | Self::Pull
                | Self::PullRebase
                | Self::PullFf
                | Self::Push
                | Self::PushLease
                | Self::PushUpstream
                | Self::Branch
                | Self::SwitchBranch
                | Self::Merge
                | Self::Rebase
                | Self::Stash
                | Self::Bisect
                | Self::NewTag
                | Self::InteractiveRebase
                | Self::PreviewCherryPick
                | Self::PreviewGraphRebase
        );
        if mutating && (!s.local_jobs.is_empty() || !s.active_ops.is_empty()) {
            return Some("Repository operation is queued or running");
        }
        if mutating
            && s.capabilities.as_ref().is_some_and(|c| {
                c.is_bare
                    || c.merge_in_progress
                    || c.rebase_in_progress
                    || c.cherry_pick_in_progress
                    || c.revert_in_progress
            })
        {
            return Some("Finish the current operation first; a working tree is required");
        }
        if matches!(
            self,
            Self::Pull
                | Self::PullRebase
                | Self::PullFf
                | Self::Push
                | Self::PushLease
                | Self::PushUpstream
                | Self::Merge
                | Self::Rebase
                | Self::InteractiveRebase
                | Self::PreviewCherryPick
                | Self::PreviewGraphRebase
        ) && s.head_branch.is_none()
        {
            return Some("Check out a branch first");
        }
        if self == Self::Stash && s.dirty_count == 0 {
            return Some("No changes to stash");
        }
        None
    }
    pub fn available(self, s: &crate::state::AppState) -> bool {
        self.disabled_reason(s).is_none()
    }
    pub fn default_shortcut(self) -> Option<Shortcut> {
        let (key, shift) = match self {
            Self::Palette => ("K", false),
            Self::Fetch => ("F", true),
            Self::Pull => ("P", true),
            Self::Push => ("U", true),
            Self::SwitchBranch => ("B", false),
            Self::Search => ("F", false),
            Self::Sidebar => ("1", false),
            Self::Graph => ("2", false),
            Self::Changes => ("3", false),
            Self::Diff => ("4", false),
            _ => return None,
        };
        Some(Shortcut {
            key: key.into(),
            shift,
            alt: false,
        })
    }
}
/// Command is Control on Windows/Linux and Command on macOS. Missing entries use
/// defaults; an empty key disables a binding. Store strings for forward compatibility.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Shortcuts(pub BTreeMap<String, Shortcut>);
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Shortcut {
    pub key: String,
    pub shift: bool,
    pub alt: bool,
}
impl Shortcut {
    pub fn key(&self) -> Option<Key> {
        Key::from_name(&self.key).or_else(|| {
            Key::ALL
                .iter()
                .copied()
                .find(|key| key.name().eq_ignore_ascii_case(&self.key))
        })
    }
    /// The same validity rule applies to recording, saved overrides and dispatch.
    pub fn valid_key(&self) -> Option<Key> {
        let key = self.key()?;
        if matches!(key, Key::Escape | Key::Enter | Key::Tab | Key::Q | Key::W)
            || (self.alt && key == Key::Delete)
        {
            None
        } else {
            Some(key)
        }
    }
    pub fn from_event(key: Key, modifiers: Modifiers) -> Option<Self> {
        if !modifiers.command || (cfg!(target_os = "macos") && modifiers.ctrl) {
            return None;
        }
        let binding = Self {
            key: key.name().into(),
            shift: modifiers.shift,
            alt: modifiers.alt,
        };
        binding.valid_key().map(|_| binding)
    }
    pub fn matches(&self, key: Key, modifiers: Modifiers) -> bool {
        self.valid_key() == Some(key)
            && modifiers.command
            && self.shift == modifiers.shift
            && self.alt == modifiers.alt
            && (!cfg!(target_os = "macos") || !modifiers.ctrl)
    }
    pub fn label(&self) -> String {
        format!(
            "{}+{}{}{}",
            if cfg!(target_os = "macos") {
                "Cmd"
            } else {
                "Ctrl"
            },
            if self.shift { "Shift+" } else { "" },
            if self.alt { "Alt+" } else { "" },
            self.key
        )
    }
}
impl Shortcuts {
    pub fn get(&self, command: Command) -> Option<Shortcut> {
        self.0
            .get(command.id())
            .cloned()
            .or_else(|| command.default_shortcut())
            .and_then(|mut binding| {
                binding.key = binding.valid_key()?.name().into();
                Some(binding)
            })
    }
    pub fn set(&mut self, command: Command, binding: Option<Shortcut>) -> Result<(), String> {
        let binding = binding
            .map(|mut binding| {
                binding.key = binding
                    .valid_key()
                    .ok_or("Unknown or reserved keyboard combination")?
                    .name()
                    .into();
                Ok::<_, String>(binding)
            })
            .transpose()?;
        if let Some(binding) = &binding {
            if let Some(other) = Command::ALL
                .iter()
                .find(|&&other| other != command && self.get(other).as_ref() == Some(binding))
            {
                return Err(format!(
                    "{} is already assigned to {}",
                    binding.label(),
                    other.label()
                ));
            }
        }
        self.0
            .insert(command.id().into(), binding.unwrap_or_default());
        Ok(())
    }
    pub fn take_command(&self, ctx: &egui::Context, blocked: bool) -> Option<Command> {
        if blocked || ctx.wants_keyboard_input() {
            return None;
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Key {
                key,
                pressed: true,
                repeat: false,
                modifiers,
                ..
            } = event
            {
                if let Some(command) = self.command(key, modifiers) {
                    ctx.input_mut(|i| {
                        i.consume_key(modifiers, key);
                    });
                    return Some(command);
                }
            }
        }
        None
    }
    pub fn command(&self, key: Key, modifiers: Modifiers) -> Option<Command> {
        let mut found = Command::ALL
            .iter()
            .copied()
            .filter(|&c| self.get(c).is_some_and(|b| b.matches(key, modifiers)));
        let first = found.next()?;
        if found.next().is_some() {
            None
        } else {
            Some(first)
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preferences_persist_disable_conflict_and_reset() {
        let mut shortcuts = Shortcuts::default();
        let binding = shortcuts.get(Command::Palette).unwrap();
        assert!(shortcuts.set(Command::Open, Some(binding.clone())).is_err());
        shortcuts.set(Command::Palette, None).unwrap();
        shortcuts.set(Command::Open, Some(binding)).unwrap();
        let restored: Shortcuts = toml::from_str(&toml::to_string(&shortcuts).unwrap()).unwrap();
        assert!(restored.get(Command::Palette).is_none());
        assert_eq!(
            restored.command(Key::K, Modifiers::COMMAND),
            Some(Command::Open)
        );
        assert_eq!(
            Shortcuts::default().command(Key::K, Modifiers::COMMAND),
            Some(Command::Palette)
        );
    }
    #[test]
    fn modifier_matching_is_exact_and_ambiguous_config_is_inert() {
        let mut s = Shortcuts::default();
        assert_eq!(
            s.command(Key::F, Modifiers::COMMAND | Modifiers::SHIFT),
            Some(Command::Fetch)
        );
        assert_eq!(s.command(Key::F, Modifiers::COMMAND | Modifiers::ALT), None);
        s.0.insert(Command::Open.id().into(), s.get(Command::Palette).unwrap());
        assert_eq!(s.command(Key::K, Modifiers::COMMAND), None);
        let state = crate::state::new_shared_state();
        let state = state.lock().unwrap();
        assert!(!Command::Fetch.available(&state));
        assert!(Command::Settings.available(&state));
    }
    #[test]
    fn text_focus_modal_guards_and_nonrepeating_input_frames() {
        let shortcuts = Shortcuts::default();
        let ctx = egui::Context::default();
        let event = |repeat| egui::Event::Key {
            key: Key::K,
            physical_key: None,
            pressed: true,
            repeat,
            modifiers: Modifiers::COMMAND,
        };
        let input = |events| egui::RawInput {
            events,
            ..Default::default()
        };
        ctx.begin_frame(input(vec![event(false)]));
        assert_eq!(shortcuts.take_command(&ctx, true), None);
        assert_eq!(shortcuts.take_command(&ctx, false), Some(Command::Palette));
        assert_eq!(shortcuts.take_command(&ctx, false), None);
        let _ = ctx.end_frame();
        ctx.begin_frame(input(vec![]));
        let mut text = String::new();
        egui::CentralPanel::default().show(&ctx, |ui| {
            ui.text_edit_singleline(&mut text).request_focus();
        });
        let _ = ctx.end_frame();
        ctx.begin_frame(input(vec![event(false)]));
        assert!(ctx.wants_keyboard_input());
        assert_eq!(shortcuts.take_command(&ctx, false), None);
        let _ = ctx.end_frame();
        assert!(Shortcut::from_event(Key::Q, Modifiers::COMMAND).is_none());
        assert!(Shortcut::from_event(Key::K, Modifiers::NONE).is_none());
    }
    #[test]
    fn reserved_bindings_in_saved_config_are_inert_and_set_rejects_them() {
        for (key, alt) in [
            (Key::Q, false),
            (Key::W, false),
            (Key::Enter, false),
            (Key::Tab, false),
            (Key::Escape, false),
            (Key::Delete, true),
        ] {
            let binding = Shortcut {
                key: key.name().into(),
                shift: false,
                alt,
            };
            let mut shortcuts = Shortcuts::default();
            assert!(shortcuts.set(Command::Push, Some(binding.clone())).is_err());
            shortcuts.0.insert(Command::Push.id().into(), binding);
            let restored: Shortcuts =
                toml::from_str(&toml::to_string(&shortcuts).unwrap()).unwrap();
            assert!(restored.get(Command::Push).is_none());
            let modifiers = if alt {
                Modifiers::COMMAND | Modifiers::ALT
            } else {
                Modifiers::COMMAND
            };
            assert!(restored.command(key, modifiers).is_none());
        }
    }

    #[test]
    fn discard_confirmation_blocks_all_registry_dispatch() {
        let mut state = crate::state::AppState::default();
        state.repo_path = Some(std::path::PathBuf::from("repository"));
        state.head_branch = Some("main".into());
        assert!(Command::Fetch.available(&state));
        state.pending_discard = Some(crate::state::PendingDiscard::ResetHard);
        for command in Command::ALL {
            assert!(
                !command.available(&state),
                "{} must respect the confirmation",
                command.label()
            );
        }
    }
    #[test]
    fn partial_saved_shortcuts_use_defaults_and_normalize_key_case() {
        let shortcuts: Shortcuts = toml::from_str("[Fetch]\nkey = 'k'\nshift = true").unwrap();
        let binding = shortcuts.get(Command::Fetch).unwrap();
        assert_eq!(binding.key, "K");
        assert!(!binding.alt);
        assert_eq!(
            shortcuts.command(Key::K, Modifiers::COMMAND | Modifiers::SHIFT),
            Some(Command::Fetch)
        );
    }
}
