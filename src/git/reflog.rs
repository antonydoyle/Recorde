use std::path::Path;

use crate::state::ReflogEntry;

/// Load reflog for HEAD (up to 200 entries).
pub fn load_reflog(repo_path: &Path) -> Vec<ReflogEntry> {
    let Ok(output) = super::process::git(
        repo_path,
        &["reflog", "--format=%H\t%h\t%gd\t%gs\t%ct", "-n", "200"],
        &std::sync::atomic::AtomicBool::new(false),
    ) else {
        return Vec::new();
    };

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(5, '\t');
            let oid = parts.next()?.trim().to_string();
            let short_oid = parts.next()?.trim().to_string();
            let refname = parts.next()?.trim().to_string();
            let message = parts.next()?.trim().to_string();
            let timestamp = parts.next()?.trim().parse::<i64>().unwrap_or(0);
            Some(ReflogEntry {
                oid,
                short_oid,
                refname,
                message,
                timestamp,
            })
        })
        .collect()
}

/// Recover a reflog entry by creating a branch at that OID.
pub fn recover(repo_path: &Path, oid: &str, branch_name: &str) -> Result<(), String> {
    let oid = git2::Oid::from_str(oid).map_err(|_| "Recovery requires an exact commit ID.")?;
    if branch_name.starts_with('-')
        || !git2::Reference::is_valid_name(&format!("refs/heads/{branch_name}"))
    {
        return Err("Invalid recovery branch name.".into());
    }
    super::process::git(
        repo_path,
        &["branch", "--", branch_name, &oid.to_string()],
        &std::sync::atomic::AtomicBool::new(false),
    )
    .map(|_| ())
    .map_err(|e| e.user_message())
}

/// Return true if a bisect is currently active in this repo.
pub fn bisect_is_active(repo_path: &Path) -> bool {
    git2::Repository::open(repo_path)
        .map(|r| r.path().join("BISECT_LOG").exists())
        .unwrap_or(false)
}

/// The commit under test. Git checks each candidate out, so HEAD is the
/// candidate once a bad and a good commit are known. `--no-checkout` sessions
/// record it in BISECT_HEAD instead. None once the first bad commit is found.
pub fn bisect_current(repo_path: &Path) -> Option<String> {
    let repo = git2::Repository::open(repo_path).ok()?;
    if let Ok(oid) = std::fs::read_to_string(repo.path().join("BISECT_HEAD")) {
        return Some(oid.trim().to_string());
    }
    if bisect_first_bad(repo_path).is_some() {
        return None;
    }
    let has_bad = repo.find_reference("refs/bisect/bad").is_ok();
    let has_good = repo
        .references_glob("refs/bisect/good-*")
        .ok()?
        .next()
        .is_some();
    if !(has_bad && has_good) {
        return None;
    }
    let head = repo.head().ok()?;
    head.target().map(|oid| oid.to_string())
}

/// The first bad commit, once a bisect session has found it. The result
/// survives a restart until the session is reset.
pub fn bisect_first_bad(repo_path: &Path) -> Option<String> {
    let repo = git2::Repository::open(repo_path).ok()?;
    let log = std::fs::read_to_string(repo.path().join("BISECT_LOG")).ok()?;
    let logged = log.lines().rev().find_map(|line| {
        let rest = line.strip_prefix("# first bad commit: [")?;
        let oid = git2::Oid::from_str(rest.split(']').next()?).ok()?;
        Some(oid.to_string())
    });
    logged.or_else(|| first_bad_from_refs(&repo))
}

/// Git versions differ in what BISECT_LOG records, so the result is also
/// derived from the session's refs: the search is over once the bad commit is
/// the only one not reachable from a good commit. Skipped commits leave others
/// in that set, matching Git's "only skipped commits left" outcome.
fn first_bad_from_refs(repo: &git2::Repository) -> Option<String> {
    let bad = repo.refname_to_id("refs/bisect/bad").ok()?;
    let mut walk = repo.revwalk().ok()?;
    walk.push(bad).ok()?;
    let mut goods = 0;
    for reference in repo.references_glob("refs/bisect/good-*").ok()?.flatten() {
        walk.hide(reference.target()?).ok()?;
        goods += 1;
    }
    if goods == 0 {
        return None;
    }
    let left: Vec<git2::Oid> = walk.take(2).collect::<Result<_, _>>().ok()?;
    (left == [bad]).then(|| bad.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_bad_commit_is_derived_from_refs_when_the_log_does_not_name_it() {
        let repo = crate::git::test_support::TestRepo::new();
        let good = repo.commit("state", b"good\n");
        let middle = repo.commit("state", b"unknown\n");
        let bad = repo.commit("state", b"bad\n");
        let git = repo.open();
        std::fs::write(git.path().join("BISECT_LOG"), "git bisect start\n").unwrap();
        git.reference("refs/bisect/bad", bad, true, "test").unwrap();
        assert_eq!(bisect_first_bad(repo.path()), None, "no good commit yet");
        git.reference(&format!("refs/bisect/good-{good}"), good, true, "test")
            .unwrap();
        assert_eq!(bisect_first_bad(repo.path()), None, "a candidate remains");
        git.reference(&format!("refs/bisect/good-{middle}"), middle, true, "test")
            .unwrap();
        assert_eq!(bisect_first_bad(repo.path()), Some(bad.to_string()));
        std::fs::remove_file(git.path().join("BISECT_LOG")).unwrap();
        assert_eq!(bisect_first_bad(repo.path()), None, "only during a session");
    }
    #[test]
    fn recovery_rejects_option_injection_and_preserves_existing_branches() {
        let repo = crate::git::test_support::TestRepo::new();
        let head = repo.commit("file", b"data\n");
        assert!(recover(repo.path(), &head.to_string(), "--delete").is_err());
        assert!(recover(repo.path(), "--force", "recovered").is_err());
        assert_eq!(repo.head(), head);
        recover(repo.path(), &head.to_string(), "recovered").unwrap();
        assert_eq!(
            repo.open().refname_to_id("refs/heads/recovered").unwrap(),
            head
        );
    }
}
