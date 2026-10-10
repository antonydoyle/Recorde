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
    for reference in repo.references()? {
        let reference = reference?;
        let Some(name) = reference
            .name()
            .ok()
            .filter(|name| name.starts_with(&prefix))
        else {
            continue;
        };
        // Native Git parses the first full object ID in the final component,
        // including nested replacement namespaces. Bad names are ignored.
        let Some(original) = name
            .rsplit('/')
            .next()
            .and_then(|name| name.get(..40))
            .and_then(|name| git2::Oid::from_str(name).ok())
        else {
            continue;
        };
        let target = reference
            .resolve()?
            .target()
            .ok_or_else(|| git2::Error::from_str("Replacement reference has no object target."))?;
        if result.insert(original, target).is_some() {
            return Err(git2::Error::from_str(
                "Duplicate replacement object reference.",
            ));
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
    // Git permits five lookups, including the terminal lookup with no replacement.
    Err(git2::Error::from_str(
        "Replacement object chain exceeds Git's depth limit.",
    ))
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
    fn symbolic_replacement_refs_and_invalid_names_match_native_history() {
        let fixture = crate::git::test_support::TestRepo::new();
        let base = fixture.commit("file", b"base");
        let head = fixture.commit("file", b"head");
        fixture.git_ok(&["commit-graph", "write", "--reachable"]);
        let repo = fixture.open();
        repo.reference("refs/replace/not-an-object", head, true, "fixture")
            .unwrap();
        assert!(refs(&repo).unwrap().is_empty());
        assert!(super::super::graph::CommitGraph::from_repo_limit(&repo, 10).is_ok());
        let signature = repo.signature().unwrap();
        let tree = repo.find_commit(head).unwrap().tree().unwrap();
        let effective = repo
            .commit(
                None,
                &signature,
                &signature,
                "symbolic replacement",
                &tree,
                &[&repo.find_commit(base).unwrap()],
            )
            .unwrap();
        repo.reference(
            "refs/recorde/replacement-fixture",
            effective,
            true,
            "fixture",
        )
        .unwrap();
        let name = format!("refs/replace/{head}");
        repo.reference_symbolic(&name, "refs/recorde/replacement-fixture", true, "fixture")
            .unwrap();
        assert_eq!(refs(&repo).unwrap().get(&head), Some(&effective));
        assert!(!super::super::history_cursor::Cursor::can_accelerate(&repo));
        let graph = super::super::graph::CommitGraph::from_repo_filtered(
            &repo,
            10,
            &Default::default(),
            true,
        )
        .unwrap();
        let native = fixture.git(&["log", "-1", "--format=%s", "main"]);
        assert!(native.status.success());
        assert_eq!(
            graph.nodes[0].summary,
            String::from_utf8(native.stdout).unwrap().trim()
        );
        let new_effective = repo
            .commit(
                None,
                &signature,
                &signature,
                "new symbolic target",
                &tree,
                &[],
            )
            .unwrap();
        repo.reference(
            "refs/recorde/replacement-fixture",
            new_effective,
            true,
            "fixture",
        )
        .unwrap();
        let refreshed = super::super::graph::CommitGraph::from_repo_page(
            &repo,
            10,
            &Default::default(),
            true,
            Some(&graph),
        )
        .unwrap();
        assert_eq!(refreshed.nodes.len(), 1);
        assert_eq!(refreshed.nodes[0].summary, "new symbolic target");
        assert_eq!(fixture.head(), head);
        assert_eq!(fixture.index_bytes("file"), b"head");
    }

    #[test]
    fn nested_replacement_names_and_duplicate_originals_match_native_git() {
        let fixture = crate::git::test_support::TestRepo::new();
        let original = fixture.commit("file", b"original");
        let target = fixture.commit("file", b"target");
        let repo = fixture.open();
        let nested = format!("refs/replace/nested/{original}-suffix");
        repo.reference(&nested, target, true, "fixture").unwrap();
        assert_eq!(refs(&repo).unwrap().get(&original), Some(&target));
        let native = fixture.git(&["cat-file", "commit", &original.to_string()]);
        assert!(
            native.status.success(),
            "{}",
            String::from_utf8_lossy(&native.stderr)
        );
        let raw_target = fixture.git(&[
            "--no-replace-objects",
            "cat-file",
            "commit",
            &target.to_string(),
        ]);
        assert_eq!(native.stdout, raw_target.stdout);
        repo.reference(&format!("refs/replace/{original}"), target, true, "fixture")
            .unwrap();
        assert!(refs(&repo).is_err());
        assert!(!fixture
            .git(&["cat-file", "commit", &original.to_string()])
            .status
            .success());
    }

    #[test]
    fn replacement_chain_depth_counts_the_terminal_lookup_like_native_git() {
        let fixture = crate::git::test_support::TestRepo::new();
        let original = fixture.commit("file", b"original");
        let repo = fixture.open();
        let signature = repo.signature().unwrap();
        let tree = repo.find_commit(original).unwrap().tree().unwrap();
        let mut current = original;
        for depth in 1..=5 {
            let target = repo
                .commit(
                    None,
                    &signature,
                    &signature,
                    &format!("replacement {depth}"),
                    &tree,
                    &[],
                )
                .unwrap();
            repo.reference(&format!("refs/replace/{current}"), target, true, "fixture")
                .unwrap();
            let native = fixture.git(&["cat-file", "commit", &original.to_string()]);
            let actual = resolve(original, &refs(&repo).unwrap());
            assert_eq!(actual.is_ok(), native.status.success(), "depth {depth}");
            if depth < 5 {
                assert_eq!(actual.unwrap(), target);
            } else {
                assert!(actual.is_err());
            }
            current = target;
        }
        assert_eq!(fixture.head(), original);
        assert_eq!(fixture.index_bytes("file"), b"original");
    }

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
