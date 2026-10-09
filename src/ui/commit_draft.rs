use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};

#[derive(Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct CommitDraft {
    pub subject: String,
    pub body: String,
    pub is_amend: bool,
    pub co_author_input: String,
    pub co_authors: Vec<String>,
    pub signoff_enabled: bool,
    pub template_loaded: bool,
}

struct PendingCommit {
    repo: PathBuf,
    submitted: CommitDraft,
    completion: Receiver<bool>,
}

#[derive(Default)]
pub(super) struct Drafts {
    pub current: CommitDraft,
    repo: Option<PathBuf>,
    saved: HashMap<PathBuf, CommitDraft>,
    pending: Vec<PendingCommit>,
    persisted: Option<(PathBuf, CommitDraft)>,
    last_persist_attempt: Option<std::time::Instant>,
    pub save_errors: HashMap<PathBuf, String>,
}

impl Drafts {
    pub fn switch_to(&mut self, repo: Option<PathBuf>) {
        let repo = repo.map(|p| p.canonicalize().unwrap_or(p));
        if self.repo == repo {
            return;
        }
        let _ = self.persist_current();
        let previous = std::mem::take(&mut self.current);
        if let Some(old) = self.repo.take() {
            self.saved.insert(old, previous);
        }
        self.current = repo
            .as_ref()
            .and_then(|p| self.saved.remove(p).or_else(|| read_draft(p)))
            .unwrap_or_default();
        self.repo = repo;
        self.persisted = self.repo.clone().map(|p| (p, self.current.clone()));
    }

    pub fn busy(&self) -> bool {
        self.pending
            .iter()
            .any(|p| Some(&p.repo) == self.repo.as_ref())
    }

    pub fn submitted(&mut self, completion: Receiver<bool>) {
        let _ = self.persist_current();
        if let Some(repo) = self.repo.clone() {
            self.pending.push(PendingCommit {
                repo,
                submitted: self.current.clone(),
                completion,
            });
        }
    }

    pub fn poll(&mut self) {
        let errors = &mut self.save_errors;
        self.pending.retain(|pending| {
            let success = match pending.completion.try_recv() {
                Ok(success) => success,
                Err(TryRecvError::Empty) => return true,
                Err(TryRecvError::Disconnected) => false,
            };
            let draft = if self.repo.as_ref() == Some(&pending.repo) {
                Some(&mut self.current)
            } else {
                self.saved.get_mut(&pending.repo)
            };
            if let Some(draft) = draft {
                // Preserve any edits made while hooks/signing were running.
                if success && *draft == pending.submitted {
                    *draft = CommitDraft {
                        signoff_enabled: draft.signoff_enabled,
                        ..Default::default()
                    };
                    match write_draft(&pending.repo, draft) {
                        Ok(()) => {
                            errors.remove(&pending.repo);
                        }
                        Err(error) => {
                            errors.insert(pending.repo.clone(), error);
                        }
                    }
                }
            }
            false
        });
        if self.last_persist_attempt.is_none_or(|time| time.elapsed() >= std::time::Duration::from_millis(400)) {
            let _ = self.persist_current();
            for (repo, draft) in &self.saved {
                if self.save_errors.contains_key(repo) && write_draft(repo, draft).is_ok() {
                    self.save_errors.remove(repo);
                }
            }
        }
    }

    pub fn persist_current(&mut self) -> Result<(), String> {
        self.last_persist_attempt = Some(std::time::Instant::now());
        if let Some(repo) = &self.repo {
            if self
                .persisted
                .as_ref()
                .is_some_and(|(p, draft)| p == repo && draft == &self.current)
            {
                return Ok(());
            }
            if let Err(error) = write_draft(repo, &self.current) {
                self.save_errors.insert(repo.clone(), error.clone());
                return Err(error);
            }
            self.save_errors.remove(repo);
            self.persisted = Some((repo.clone(), self.current.clone()));
        }
        Ok(())
    }
}

impl Drop for Drafts {
    fn drop(&mut self) {
        let _ = self.persist_current();
    }
}

fn draft_path(repo: &std::path::Path) -> Result<PathBuf, String> {
    let repo = git2::Repository::open(repo).map_err(|e| e.message().to_string())?;
    Ok(repo.path().join("recorde").join("commit-draft.toml"))
}
fn read_draft(repo: &std::path::Path) -> Option<CommitDraft> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(draft_path(repo).ok()?).ok()?.take(1_048_577).read_to_end(&mut bytes).ok()?;
    if bytes.len() > 1_048_576 { return None; }
    toml::from_str(std::str::from_utf8(&bytes).ok()?).ok()
}
fn write_draft(repo: &std::path::Path, draft: &CommitDraft) -> Result<(), String> {
    use std::io::Write;
    let path = draft_path(repo)?;
    let directory = path.parent().ok_or("Invalid draft path.")?;
    std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    let mut file = tempfile::NamedTempFile::new_in(directory).map_err(|e| e.to_string())?;
    file.write_all(
        toml::to_string(draft)
            .map_err(|e| e.to_string())?
            .as_bytes(),
    )
    .map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn rejected_or_disconnected_commit_preserves_draft() {
        let mut drafts = Drafts::default();
        drafts.switch_to(Some("repo-a".into()));
        drafts.current.subject = "keep this".into();
        let (tx, rx) = mpsc::channel();
        drafts.submitted(rx);
        assert!(drafts.busy());
        tx.send(false).unwrap();
        drafts.poll();
        assert!(!drafts.busy());
        assert_eq!(drafts.current.subject, "keep this");
        let (tx, rx) = mpsc::channel();
        drafts.submitted(rx);
        drop(tx);
        drafts.poll();
        assert_eq!(drafts.current.subject, "keep this");
    }

    #[test]
    fn drafts_survive_restart_and_success_clears_the_disk_copy() {
        let repo = crate::git::test_support::TestRepo::new();
        let mut drafts = Drafts::default();
        drafts.switch_to(Some(repo.path().into()));
        drafts.current.subject = "recover after restart".into();
        drafts.current.body = "details".into();
        drafts.persist_current().unwrap();
        drop(drafts);
        let mut restored = Drafts::default();
        restored.switch_to(Some(repo.path().into()));
        assert_eq!(restored.current.subject, "recover after restart");
        let (tx, rx) = mpsc::channel();
        restored.submitted(rx);
        tx.send(true).unwrap();
        restored.poll();
        assert!(read_draft(repo.path()).unwrap().subject.is_empty());
    }

    #[test]
    fn completion_clears_only_the_submitted_repository_draft() {
        let mut drafts = Drafts::default();
        drafts.switch_to(Some("repo-a".into()));
        drafts.current.subject = "commit a".into();
        let (tx, rx) = mpsc::channel();
        drafts.submitted(rx);
        drafts.switch_to(Some("repo-b".into()));
        drafts.current.subject = "work on b".into();
        tx.send(true).unwrap();
        drafts.poll();
        assert_eq!(drafts.current.subject, "work on b");
        drafts.switch_to(Some("repo-a".into()));
        assert!(drafts.current.subject.is_empty());
        assert!(!drafts.current.template_loaded);
    }

    #[test]
    fn edits_during_commit_are_preserved() {
        let mut drafts = Drafts::default();
        drafts.switch_to(Some("repo-a".into()));
        drafts.current.subject = "submitted".into();
        let (tx, rx) = mpsc::channel();
        drafts.submitted(rx);
        drafts.current.subject = "next draft".into();
        tx.send(true).unwrap();
        drafts.poll();
        assert_eq!(drafts.current.subject, "next draft");
    }
}
