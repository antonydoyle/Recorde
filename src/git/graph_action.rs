//! Immutable graph-operation previews, validated again inside the mutation queue.
use crate::state::SharedState;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    pub oid: String,
    pub reference: Option<String>,
}
impl Endpoint {
    pub fn label(&self) -> String {
        self.reference
            .clone()
            .unwrap_or_else(|| self.oid[..self.oid.len().min(12)].into())
    }
    fn validate(&self, repo: &git2::Repository) -> Result<(), String> {
        let oid = git2::Oid::from_str(&self.oid).map_err(|e| e.to_string())?;
        repo.find_commit(oid)
            .map_err(|_| "The dragged commit is no longer available.")?;
        if let Some(reference) = &self.reference {
            let actual = repo
                .find_reference(reference)
                .and_then(|r| r.peel_to_commit())
                .map_err(|_| "A previewed branch was removed. Drag again.")?;
            if actual.id() != oid {
                return Err(
                    "A previewed branch moved. Drag again to review the new target.".into(),
                );
            }
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Merge,
    Rebase,
    CherryPick,
}
impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Self::Merge => "Merge",
            Self::Rebase => "Rebase",
            Self::CherryPick => "Cherry-pick",
        }
    }
}
#[derive(Clone)]
pub struct Drag {
    pub path: PathBuf,
    pub epoch: u64,
    pub source: Endpoint,
    head_ref: String,
    head_oid: String,
}
impl Drag {
    pub fn capture(path: &Path, epoch: u64, source: Endpoint) -> Result<Self, String> {
        let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
        source.validate(&repo)?;
        let head = repo.head().map_err(|e| e.to_string())?;
        if !head.is_branch() {
            return Err("Check out a branch before dragging graph items.".into());
        }
        let result = Self {
            path: path.canonicalize().map_err(|e| e.to_string())?,
            epoch,
            source,
            head_ref: head.name().map_err(|e| e.to_string())?.into(),
            head_oid: head
                .peel_to_commit()
                .map_err(|e| e.to_string())?
                .id()
                .to_string(),
        };
        Ok(result)
    }
    pub fn finish(self, destination: Endpoint) -> Result<Preview, String> {
        let result = Preview::capture(&self.path, self.epoch, self.source, destination)?;
        if result.head_ref != self.head_ref || result.head_oid != self.head_oid {
            return Err("HEAD changed while dragging. Drag again.".into());
        }
        Ok(result)
    }
}
#[derive(Clone)]
pub struct Preview {
    pub path: PathBuf,
    pub epoch: u64,
    pub head_ref: String,
    pub head_oid: String,
    pub source: Endpoint,
    pub destination: Endpoint,
    pub action: Action,
    pub protected_branches: Vec<String>,
}
impl Preview {
    pub fn capture(
        path: &Path,
        epoch: u64,
        source: Endpoint,
        destination: Endpoint,
    ) -> Result<Self, String> {
        let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
        let head = repo
            .head()
            .map_err(|_| "An existing checked-out branch is required.")?;
        if !head.is_branch() {
            return Err("Check out a branch before dragging graph items.".into());
        }
        let mut result = Self {
            path: path.canonicalize().map_err(|e| e.to_string())?,
            epoch,
            head_ref: head.name().map_err(|e| e.to_string())?.into(),
            head_oid: head
                .peel_to_commit()
                .map_err(|e| e.to_string())?
                .id()
                .to_string(),
            source,
            destination,
            action: Action::Merge,
            protected_branches: Vec::new(),
        };
        if result.source.reference.as_deref() == Some(&result.head_ref) {
            result.action = Action::Rebase;
        } else if result.source.reference.is_none() {
            result.action = Action::CherryPick;
        }
        result.validate(&result.path)?;
        Ok(result)
    }
    pub fn allows(&self, action: Action) -> bool {
        if action == Action::Rebase {
            self.source.reference.as_deref() == Some(&self.head_ref)
                && self.destination.oid != self.head_oid
        } else {
            self.destination.oid == self.head_oid
                && self.source.oid != self.head_oid
                && self
                    .destination
                    .reference
                    .as_deref()
                    .is_none_or(|r| r == self.head_ref)
        }
    }
    pub fn description(&self) -> String {
        let branch = self
            .head_ref
            .strip_prefix("refs/heads/")
            .unwrap_or(&self.head_ref);
        match self.action {
            Action::Merge => format!(
                "Merge {} ({}) into {branch}",
                self.source.label(),
                &self.source.oid[..7]
            ),
            Action::CherryPick => format!("Apply commit {} to {branch}", &self.source.oid[..7]),
            Action::Rebase => format!(
                "Replay {branch} onto {} ({})",
                self.destination.label(),
                &self.destination.oid[..7]
            ),
        }
    }
    pub fn validate(&self, path: &Path) -> Result<(), String> {
        if path.canonicalize().map_err(|e| e.to_string())? != self.path {
            return Err("Repository changed. Drag again.".into());
        }
        let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
        let head = repo.head().map_err(|e| e.to_string())?;
        if head.name().ok() != Some(self.head_ref.as_str())
            || head
                .peel_to_commit()
                .map_err(|e| e.to_string())?
                .id()
                .to_string()
                != self.head_oid
        {
            return Err("HEAD changed since the preview. Drag again.".into());
        }
        if repo.state() != git2::RepositoryState::Clean {
            return Err("Finish or abort the current Git operation first.".into());
        }
        if self.action == Action::Rebase
            && self
                .protected_branches
                .iter()
                .any(|name| self.head_ref.strip_prefix("refs/heads/") == Some(name.as_str()))
        {
            return Err("This branch is protected from graph rebase. Review protected branches in settings.".into());
        }
        self.source.validate(&repo)?;
        self.destination.validate(&repo)?;
        if self.action == Action::Rebase {
            verify_linear_replay(&repo, &self.head_oid, &self.destination.oid)?;
        }
        if !self.allows(self.action) {
            return Err("Drop a commit/branch onto the current branch to merge or cherry-pick, or drag the current branch onto a target to rebase.".into());
        }
        Ok(())
    }
    pub(crate) fn execute(&self, path: &Path) -> Result<(), String> {
        self.validate(path)?;
        // Recovery snapshots preserve staged, unstaged and untracked work; Git
        // retains its own refusal to overwrite local changes. Never auto-discard.
        super::recovery::protect_sequence(path, self.action.label(), || {
            // Snapshotting can take time: check refs again immediately before Git.
            self.validate(path)?;
            let (verb, oid) = match self.action {
                Action::Merge => ("merge", &self.source.oid),
                Action::Rebase => ("rebase", &self.destination.oid),
                Action::CherryPick => ("cherry-pick", &self.source.oid),
            };
            let args: Vec<&str> = match self.action {
                Action::Merge => vec![verb, "--no-edit", "--", oid],
                Action::Rebase => vec![verb, "--no-autostash", "--", oid],
                Action::CherryPick => vec![verb, "--", oid],
            };
            super::merge::run_git_seq(path, &args).map_err(|e| e.user_message())
        })
    }
    pub fn submit(self, state: SharedState) {
        {
            let mut s = state.lock().unwrap();
            if s.repo_epoch != self.epoch
                || s.repo_path
                    .as_ref()
                    .and_then(|p| p.canonicalize().ok())
                    .as_ref()
                    != Some(&self.path)
            {
                s.error = Some("Repository changed since the preview. Drag again.".into());
                return;
            }
            s.remote_op = Some(self.description());
        }
        let path = self.path.clone();
        let guard_state = state.clone();
        super::mutation::spawn_operation(path, state, move |path| {
            let s = guard_state.lock().unwrap();
            if s.repo_epoch != self.epoch
                || s.repo_path
                    .as_ref()
                    .and_then(|p| p.canonicalize().ok())
                    .as_ref()
                    != Some(&self.path)
            {
                return Err(
                    "Queued graph operation cancelled because the active repository changed."
                        .into(),
                );
            }
            drop(s);
            self.execute(path)
        });
    }
}
/// A plain rebase would drop merge commits and their recorded resolutions.
/// Refuse those gestures until a merge-preserving plan is explicitly reviewed.
fn verify_linear_replay(repo: &git2::Repository, head: &str, onto: &str) -> Result<(), String> {
    let mut walk = repo.revwalk().map_err(|e| e.to_string())?;
    walk.push(git2::Oid::from_str(head).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    walk.hide(git2::Oid::from_str(onto).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    for oid in walk {
        let commit = repo
            .find_commit(oid.map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        if commit.parent_count() > 1 {
            return Err("Graph rebase is unavailable because the replay range contains merge commits. Use a reviewed merge-preserving rebase to retain topology and conflict resolutions.".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_support::TestRepo;
    fn fixture() -> (TestRepo, Preview) {
        let repo = TestRepo::new();
        let base = repo.commit("base", b"base");
        repo.git_ok(&["checkout", "-b", "topic"]);
        let source = repo.commit("topic", b"topic");
        repo.git_ok(&["checkout", "main"]);
        let preview = Preview::capture(
            repo.path(),
            0,
            Endpoint {
                oid: source.to_string(),
                reference: Some("refs/heads/topic".into()),
            },
            Endpoint {
                oid: base.to_string(),
                reference: Some("refs/heads/main".into()),
            },
        )
        .unwrap();
        (repo, preview)
    }
    #[test]
    fn refuses_moved_refs_and_changed_head_even_at_same_commit() {
        let (repo, preview) = fixture();
        repo.git_ok(&["branch", "-f", "topic", "main"]);
        assert!(preview
            .execute(repo.path())
            .unwrap_err()
            .contains("branch moved"));
        let (repo, preview) = fixture();
        repo.git_ok(&["checkout", "-b", "other"]);
        assert!(preview
            .execute(repo.path())
            .unwrap_err()
            .contains("HEAD changed"));
    }
    #[test]
    fn merge_preserves_unrelated_dirty_index_worktree_and_untracked_files() {
        let (repo, preview) = fixture();
        repo.write("base", b"staged");
        repo.git_ok(&["add", "base"]);
        repo.write("base", b"unstaged");
        repo.write("untracked", b"keep");
        // Git can refuse a dirty-index merge. Either success or refusal must
        // preserve every user byte, and recovery must retain a snapshot.
        let _ = preview.execute(repo.path());
        assert!(!super::super::recovery::list(repo.path()).is_empty());
        assert_eq!(repo.index_bytes("base"), b"staged");
        assert_eq!(
            std::fs::read(repo.path().join("base")).unwrap(),
            b"unstaged"
        );
        assert_eq!(
            std::fs::read(repo.path().join("untracked")).unwrap(),
            b"keep"
        );
    }
    #[test]
    fn executes_merge_cherry_pick_and_rebase_from_frozen_targets() {
        let (repo, preview) = fixture();
        preview.execute(repo.path()).unwrap();
        assert_eq!(repo.head().to_string(), preview.source.oid);
        let (repo, mut preview) = fixture();
        preview.action = Action::CherryPick;
        preview.execute(repo.path()).unwrap();
        assert!(repo.path().join("topic").exists());
        let (repo, preview) = fixture();
        let preview =
            Preview::capture(repo.path(), 0, preview.destination, preview.source).unwrap();
        assert_eq!(preview.action, Action::Rebase);
        preview.execute(repo.path()).unwrap();
        assert!(repo.path().join("topic").exists());
    }
    #[test]
    fn wrong_repository_and_wrong_drop_direction_are_rejected() {
        let (repo, preview) = fixture();
        let other = TestRepo::new();
        other.commit("a", b"a");
        assert!(preview.execute(other.path()).is_err());
        let mut wrong = preview.clone();
        wrong.destination = wrong.source.clone();
        assert!(wrong.validate(repo.path()).is_err());
    }
    #[test]
    fn head_changes_during_drag_and_protected_rebase_are_rejected() {
        let (repo, preview) = fixture();
        let drag = Drag::capture(repo.path(), 0, preview.source.clone()).unwrap();
        repo.commit("new", b"new");
        assert!(drag
            .finish(Endpoint {
                oid: repo.head().to_string(),
                reference: Some("refs/heads/main".into())
            })
            .is_err());
        let (repo, preview) = fixture();
        let mut rebase =
            Preview::capture(repo.path(), 0, preview.destination, preview.source).unwrap();
        rebase.protected_branches = vec!["main".into()];
        assert!(rebase
            .execute(repo.path())
            .unwrap_err()
            .contains("protected"));
    }

    #[test]
    fn queued_graph_operation_cancels_on_repository_epoch_change() {
        let (repo, preview) = fixture();
        let original = repo.head();
        let state = crate::state::new_shared_state();
        state.lock().unwrap().repo_path = Some(repo.path().canonicalize().unwrap());
        let reservation = super::super::mutation::reserve(repo.path());
        preview.submit(state.clone());
        state.lock().unwrap().repo_epoch += 1;
        drop(reservation);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !state.lock().unwrap().local_jobs.is_empty() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(state.lock().unwrap().local_jobs.is_empty());
        assert_eq!(repo.head(), original);
    }
    #[test]
    fn graph_rebase_refuses_merge_topology_without_modifying_history() {
        let repo = TestRepo::new();
        let base = repo.commit("base", b"base");
        repo.git_ok(&["checkout", "-b", "side"]);
        repo.commit("side", b"side");
        repo.git_ok(&["checkout", "main"]);
        repo.commit("main", b"main");
        repo.git_ok(&["merge", "--no-ff", "--no-edit", "side"]);
        let head = repo.head();
        let result = Preview::capture(
            repo.path(),
            0,
            Endpoint {
                oid: head.to_string(),
                reference: Some("refs/heads/main".into()),
            },
            Endpoint {
                oid: base.to_string(),
                reference: None,
            },
        );
        assert!(result.err().unwrap().contains("merge commits"));
        assert_eq!(repo.head(), head);
        assert_eq!(
            repo.open()
                .head()
                .unwrap()
                .peel_to_commit()
                .unwrap()
                .parent_count(),
            2
        );
    }
}
