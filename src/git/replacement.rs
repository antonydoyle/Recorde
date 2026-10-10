//! Match native Git's replacement lookup while retaining the original public OID.
use std::collections::{BTreeMap, HashSet};

pub(super) fn is_ref(name: &str) -> bool {
    name.starts_with(
        &std::env::var("GIT_REPLACE_REF_BASE").unwrap_or_else(|_| "refs/replace/".into()),
    )
}

pub(super) fn refs(repo: &git2::Repository) -> Result<BTreeMap<git2::Oid, git2::Oid>, git2::Error> {
    let mut result = BTreeMap::new();
    if std::env::var_os("GIT_NO_REPLACE_OBJECTS").is_some()
        || repo
            .config()?
            .get_bool("core.useReplaceRefs")
            .is_ok_and(|v| !v)
    {
        return Ok(result);
    }
    let prefix = std::env::var("GIT_REPLACE_REF_BASE").unwrap_or_else(|_| "refs/replace/".into());
    for reference in repo.references_glob(&format!("{prefix}*"))? {
        let reference = reference?;
        if let (Some(original), Some(target)) = (
            reference.name().ok().and_then(|n| n.strip_prefix(&prefix)),
            reference.target(),
        ) {
            result.insert(git2::Oid::from_str(original)?, target);
        }
    }
    Ok(result)
}

pub(super) fn resolve(
    id: git2::Oid,
    replacements: &BTreeMap<git2::Oid, git2::Oid>,
) -> Result<git2::Oid, git2::Error> {
    let mut id = id;
    let mut seen = HashSet::new();
    for _ in 0..5 {
        if !seen.insert(id) {
            return Err(git2::Error::from_str("Replacement object cycle."));
        }
        let Some(next) = replacements.get(&id) else {
            return Ok(id);
        };
        id = *next;
    }
    if replacements.contains_key(&id) {
        return Err(git2::Error::from_str(
            "Replacement object chain exceeds Git's depth limit.",
        ));
    }
    Ok(id)
}

pub(super) fn commit(
    repo: &git2::Repository,
    id: git2::Oid,
) -> Result<git2::Commit<'_>, git2::Error> {
    repo.find_commit(resolve(id, &refs(repo)?)?)
}

/// Native diff also honors replacements of trees and blobs below a commit.
pub(super) fn diff(
    repo: &git2::Repository,
    base: Option<&str>,
    target: &str,
) -> Result<git2::Diff<'static>, git2::Error> {
    git2::Oid::from_str(target)?;
    let args = if let Some(base) = base {
        git2::Oid::from_str(base)?;
        vec![
            "diff",
            "--binary",
            "--full-index",
            "--no-ext-diff",
            "--no-textconv",
            "-M",
            base,
            target,
            "--",
        ]
    } else {
        vec![
            "diff-tree",
            "--root",
            "-r",
            "--no-commit-id",
            "-p",
            "--binary",
            "--full-index",
            "--no-ext-diff",
            "--no-textconv",
            "-M",
            target,
            "--",
        ]
    };
    let output = super::process::git(
        repo.path(),
        &args,
        &std::sync::atomic::AtomicBool::new(false),
    )
    .map_err(|e| git2::Error::from_str(&e.user_message()))?;
    git2::Diff::from_buffer(&output.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacement_cycle_fails_instead_of_showing_original_objects() {
        let fixture = crate::git::test_support::TestRepo::new();
        let a = fixture.commit("file", b"a\n");
        let b = fixture.commit("file", b"b\n");
        let repo = fixture.open();
        repo.reference(&format!("refs/replace/{a}"), b, true, "fixture")
            .unwrap();
        repo.reference(&format!("refs/replace/{b}"), a, true, "fixture")
            .unwrap();
        assert!(resolve(a, &refs(&repo).unwrap()).is_err());
        assert!(super::super::graph::CommitGraph::from_repo_limit(&repo, 10).is_err());
        assert_eq!(fixture.head(), b);
        assert_eq!(fixture.index_bytes("file"), b"b\n");
    }
}
