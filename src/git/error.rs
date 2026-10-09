use std::fmt;

#[derive(Debug)]
pub enum GitError {
    Git2(git2::Error),
    Command { cmd: String, stderr: String },
    NotInRepo,
    ConflictDetected { conflicted_files: Vec<String> },
    Diverged { ahead: usize, behind: usize },
    CredentialFailed,
    Cancelled,
}

impl GitError {
    pub fn user_message(&self) -> String {
        let message: String = match self {
            GitError::Git2(e) => e.message().to_string(),
            GitError::Command { cmd, stderr } => {
                if stderr.is_empty() {
                    format!("Command failed: {}", cmd)
                } else {
                    format!("{}: {}", cmd, stderr.trim())
                }
            }
            GitError::NotInRepo => "No repository is open.".into(),
            GitError::ConflictDetected { conflicted_files } => {
                if conflicted_files.is_empty() {
                    "Conflicts detected — resolve them before continuing.".into()
                } else {
                    format!(
                        "Conflicts in {} file(s): {}",
                        conflicted_files.len(),
                        conflicted_files.first().map(|s| s.as_str()).unwrap_or("")
                    )
                }
            }
            GitError::Diverged { ahead, behind } => format!(
                "Branch has diverged: {} ahead, {} behind upstream. Pull with rebase or merge manually.",
                ahead, behind
            ),
            GitError::CredentialFailed => {
                "Authentication failed — check your SSH key or run 'gh auth login'.".into()
            }
            GitError::Cancelled => "Operation was cancelled.".into(),
        };
        crate::auth::redact::text(&message)
    }

    pub fn is_conflict(&self) -> bool {
        matches!(self, GitError::ConflictDetected { .. })
    }
}

impl fmt::Display for GitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.user_message())
    }
}

impl From<git2::Error> for GitError {
    fn from(e: git2::Error) -> Self {
        GitError::Git2(e)
    }
}
