use std::path::Path;

fn run(repo_path: &Path, args: &[&str]) -> Result<String, String> {
    let out = super::process::git(repo_path, args, &std::sync::atomic::AtomicBool::new(false))
        .map_err(|e| e.user_message())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Start a bisect session. `bad_oid` is the known-bad commit; good is HEAD~N or left empty
/// to let the user supply it manually.
pub fn start(repo_path: &Path) -> Result<(), String> {
    run(repo_path, &["bisect", "start"])?;
    Ok(())
}

pub fn mark_bad(repo_path: &Path, oid: &str) -> Result<Option<String>, String> {
    run(repo_path, &["bisect", "bad", &exact_commit(oid)?])?;
    Ok(next_step(repo_path))
}

pub fn mark_good(repo_path: &Path, oid: &str) -> Result<Option<String>, String> {
    run(repo_path, &["bisect", "good", &exact_commit(oid)?])?;
    Ok(next_step(repo_path))
}

/// Only exact object IDs reach Git, never option-like or symbolic input.
fn exact_commit(oid: &str) -> Result<String, String> {
    // git2 pads short hex with zeros, so require a full-length ID first.
    if oid.len() < 40 || !oid.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Bisect requires an exact commit ID.".into());
    }
    git2::Oid::from_str(oid)
        .map(|oid| oid.to_string())
        .map_err(|_| "Bisect requires an exact commit ID.".into())
}

pub fn reset(repo_path: &Path) -> Result<(), String> {
    run(repo_path, &["bisect", "reset"])?;
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Start,
    Good,
    Bad,
    Reset,
}

/// Queue a bisect step. Good and bad mark `oid`; afterwards the commit Git
/// chose next (or the first bad commit) becomes the selection.
pub fn spawn_step(step: Step, oid: Option<String>, state: crate::state::SharedState) {
    let (path, epoch) = {
        let mut s = state.lock().unwrap();
        s.remote_op = Some(
            match step {
                Step::Start => "Start bisect",
                Step::Good => "Mark commit good",
                Step::Bad => "Mark commit bad",
                Step::Reset => "Reset bisect",
            }
            .into(),
        );
        (s.repo_path.clone(), s.repo_epoch)
    };
    let Some(path) = path else { return };
    let output = state.clone();
    crate::git::mutation::spawn_operation(path, state, move |path| {
        let marked = || {
            oid.clone()
                .ok_or_else(|| "Select a commit first.".to_string())
        };
        let next = match step {
            Step::Start => start(path).map(|_| None),
            Step::Good => mark_good(path, &marked()?),
            Step::Bad => mark_bad(path, &marked()?),
            Step::Reset => reset(path).map(|_| None),
        }?;
        let mut s = output.lock().unwrap();
        if s.repo_path.as_deref() == Some(path) && s.repo_epoch == epoch {
            if let Some(next) = next {
                s.selected_commit = Some(next);
            }
        }
        Ok(())
    });
}

/// After good/bad, the commit to select: the first bad commit once found,
/// otherwise the next candidate Git checked out.
fn next_step(repo_path: &Path) -> Option<String> {
    super::reflog::bisect_first_bad(repo_path).or_else(|| super::reflog::bisect_current(repo_path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::reflog::{bisect_current, bisect_first_bad, bisect_is_active};
    use crate::git::test_support::TestRepo;

    #[test]
    fn bisect_marks_only_exact_commit_ids() {
        let repo = TestRepo::new();
        let head = repo.commit("file", b"data\n");
        start(repo.path()).unwrap();
        for input in ["--term-bad=x", "HEAD", "abc1234", ""] {
            assert!(mark_bad(repo.path(), input).is_err(), "{input}");
            assert!(mark_good(repo.path(), input).is_err(), "{input}");
        }
        assert!(repo.open().find_reference("refs/bisect/bad").is_err());
        reset(repo.path()).unwrap();
        assert_eq!(repo.head(), head);
    }

    #[test]
    fn bisect_follows_checked_out_candidates_and_reports_first_bad_commit() {
        let repo = TestRepo::new();
        let mut commits = Vec::new();
        for i in 0..8 {
            let content = if i >= 5 { "bad\n" } else { "good\n" };
            repo.write("marker", format!("{i}\n").as_bytes());
            repo.git_ok(&["add", "marker"]);
            commits.push(repo.commit("state", content.as_bytes()));
        }
        let first_bad = commits[5].to_string();
        let tip = commits[7].to_string();

        start(repo.path()).unwrap();
        assert!(bisect_is_active(repo.path()));
        assert_eq!(
            bisect_current(repo.path()),
            None,
            "no candidate before good and bad"
        );
        assert_eq!(mark_bad(repo.path(), &tip).unwrap(), None);
        let mut next = mark_good(repo.path(), &commits[0].to_string()).unwrap();
        let mut steps = 0;
        while bisect_first_bad(repo.path()).is_none() {
            let candidate = next.clone().expect("a candidate is checked out");
            assert_eq!(candidate, repo.head().to_string());
            assert_eq!(
                bisect_current(repo.path()).as_deref(),
                Some(candidate.as_str())
            );
            let bad = std::fs::read(repo.path().join("state")).unwrap() == b"bad\n";
            next = if bad {
                mark_bad(repo.path(), &candidate)
            } else {
                mark_good(repo.path(), &candidate)
            }
            .unwrap();
            steps += 1;
            assert!(steps < 8, "bisect did not converge");
        }
        assert_eq!(
            bisect_first_bad(repo.path()).as_deref(),
            Some(first_bad.as_str())
        );
        assert_eq!(next.as_deref(), Some(first_bad.as_str()));
        assert_eq!(bisect_current(repo.path()), None);

        reset(repo.path()).unwrap();
        assert!(!bisect_is_active(repo.path()));
        assert_eq!(bisect_first_bad(repo.path()), None);
        assert_eq!(repo.head().to_string(), tip);
        assert_eq!(
            repo.open().head().unwrap().shorthand().unwrap(),
            "main",
            "reset returns to the original branch"
        );
    }
}
