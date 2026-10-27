//! Checkout provider review refs, including fork contributions, without guessing branch names.
use super::capabilities::ProviderType;
use std::path::Path;
use std::sync::atomic::AtomicBool;

pub(crate) fn verify_remote(path: &Path, remote: &str, expected_url: &str) -> Result<(), String> {
    let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
    let current = repo.find_remote(remote).map_err(|e| e.to_string())?;
    if current.url().ok() != Some(expected_url) {
        return Err("Review remote changed. Refresh the list before continuing.".into());
    }
    Ok(())
}

pub fn checkout(
    path: &Path,
    remote: &str,
    provider: &ProviderType,
    number: u32,
    expected: &str,
) -> Result<(), String> {
    if expected.len() != 40 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Review has no full head commit; refresh its details.".into());
    }
    let expected = git2::Oid::from_str(expected)
        .map_err(|_| "Review has no valid head commit; refresh its details.")?;
    let source = match provider {
        ProviderType::GitHub => format!("refs/pull/{number}/head"),
        ProviderType::GitLab => format!("refs/merge-requests/{number}/head"),
        ProviderType::Bitbucket | ProviderType::AzureDevOps => expected.to_string(),
        _ => return Err("Review checkout is not supported for this provider.".into()),
    };
    let verify_head = || -> Result<(), String> {
        if matches!(
            provider,
            ProviderType::Bitbucket | ProviderType::AzureDevOps
        ) {
            let (owner, repository, _, auth, current_provider) =
                super::operations::resolve_repo_info_for_remote(path, remote)?;
            if &current_provider != provider {
                return Err("Review provider changed. Refresh before checking out.".into());
            }
            let current = if *provider == ProviderType::Bitbucket {
                crate::providers::bitbucket::review_head(&owner, &repository, number, &auth)?
            } else {
                crate::providers::azure::review_head(&owner, &repository, number, &auth)?
            };
            if current != expected.to_string() {
                return Err("Review head changed. Refresh before checking out.".into());
            }
        }
        Ok(())
    };
    checkout_verified(path, remote, &source, number, expected, verify_head)
}

fn checkout_verified(
    path: &Path,
    remote: &str,
    source: &str,
    number: u32,
    expected: git2::Oid,
    verify_head: impl Fn() -> Result<(), String>,
) -> Result<(), String> {
    verify_head()?;
    let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
    repo.find_remote(remote).map_err(|e| e.to_string())?;
    if repo.state() != git2::RepositoryState::Clean {
        return Err("Finish the current Git operation before checking out a review.".into());
    }
    let branch = format!("review/{number}-{}", &expected.to_string()[..8]);
    let reference = format!("refs/recorde/reviews/{number}");
    super::process::git(
        path,
        &[
            "fetch",
            "--no-tags",
            "--",
            remote,
            &format!("+{source}:{reference}"),
        ],
        &AtomicBool::new(false),
    )
    .map_err(|e| e.user_message())?;
    if repo.refname_to_id(&reference).map_err(|e| e.to_string())? != expected {
        return Err(
            "The review changed since it was loaded. Refresh the list before checking it out."
                .into(),
        );
    }
    verify_head()?;
    let result = if let Ok(existing) = repo.find_branch(&branch, git2::BranchType::Local) {
        if existing.get().target() != Some(expected) {
            return Err(format!("Branch {branch} contains local commits. Rename it before checking out this review again."));
        }
        super::process::git(path, &["checkout", &branch, "--"], &AtomicBool::new(false))
    } else {
        super::process::git(
            path,
            &["checkout", "-b", &branch, &expected.to_string(), "--"],
            &AtomicBool::new(false),
        )
    };
    result.map(|_| ()).map_err(|e| e.user_message())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_support::TestRepo;

    #[test]
    fn exact_commit_checkout_checks_cloud_head_again_after_fetch() {
        let repo = TestRepo::new();
        let base = repo.commit("file", b"base\n");
        let review = repo.commit("file", b"review\n");
        let remote = tempfile::tempdir().unwrap();
        git2::Repository::init_bare(remote.path()).unwrap();
        repo.git_ok(&["remote", "add", "origin", remote.path().to_str().unwrap()]);
        repo.git_ok(&["push", "origin", "HEAD:refs/heads/topic"]);
        repo.git_ok(&["reset", "--hard", &base.to_string()]);
        let count = std::cell::Cell::new(0);
        let error = checkout_verified(
            repo.path(),
            "origin",
            &review.to_string(),
            7,
            review,
            || {
                count.set(count.get() + 1);
                if count.get() > 1 {
                    Err("Cloud head changed".into())
                } else {
                    Ok(())
                }
            },
        )
        .unwrap_err();
        assert!(error.contains("changed"));
        assert_eq!(repo.head(), base);
        assert_eq!(count.get(), 2);
        repo.write("file", b"local changes\n");
        assert!(checkout_verified(
            repo.path(),
            "origin",
            &review.to_string(),
            7,
            review,
            || Ok(())
        )
        .is_err());
        assert_eq!(repo.head(), base);
        assert_eq!(
            std::fs::read(repo.path().join("file")).unwrap(),
            b"local changes\n"
        );
        repo.git_ok(&["restore", "file"]);
        checkout_verified(
            repo.path(),
            "origin",
            &review.to_string(),
            7,
            review,
            || Ok(()),
        )
        .unwrap();
        assert_eq!(repo.head(), review);
    }

    #[test]
    fn changed_remote_cannot_retarget_a_review_form() {
        let repo = TestRepo::new();
        let old = "https://github.com/old/project.git";
        repo.open().remote("origin", old).unwrap();
        verify_remote(repo.path(), "origin", old).unwrap();
        repo.open()
            .remote_set_url("origin", "https://github.com/new/project.git")
            .unwrap();
        let error = crate::providers::write::prepare(
            repo.path().into(),
            Some(("origin".into(), old.into())),
        )
        .err()
        .unwrap();
        assert!(error.contains("remote changed"));
    }
    #[test]
    fn review_ref_checkout_preserves_dirty_files_and_rejects_changed_heads() {
        for provider in [ProviderType::GitHub, ProviderType::GitLab] {
            let repo = TestRepo::new();
            let base = repo.commit("file", b"base\n");
            let review = repo.commit("file", b"review\n");
            let remote = tempfile::tempdir().unwrap();
            git2::Repository::init_bare(remote.path()).unwrap();
            repo.git_ok(&["remote", "add", "upstream", remote.path().to_str().unwrap()]);
            let reference = if provider == ProviderType::GitHub {
                "refs/pull/42/head"
            } else {
                "refs/merge-requests/42/head"
            };
            repo.git_ok(&["push", "upstream", &format!("HEAD:{reference}")]);
            repo.git_ok(&["reset", "--hard", &base.to_string()]);
            repo.write("file", b"local edit\n");
            assert!(checkout(repo.path(), "upstream", &provider, 42, &review.to_string()).is_err());
            assert_eq!(repo.head(), base);
            assert_eq!(
                std::fs::read(repo.path().join("file")).unwrap(),
                b"local edit\n"
            );
            repo.git_ok(&["restore", "file"]);
            assert!(checkout(repo.path(), "upstream", &provider, 42, &base.to_string()).is_err());
            assert_eq!(repo.head(), base);
            checkout(repo.path(), "upstream", &provider, 42, &review.to_string()).unwrap();
            assert_eq!(repo.head(), review);
            assert_eq!(repo.index_bytes("file"), b"review\n");
        }
    }
}
