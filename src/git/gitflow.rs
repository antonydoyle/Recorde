//! Git Flow branching without the git-flow extension. Settings use the same
//! `gitflow.*` configuration keys, so repositories stay compatible with it.
use std::path::Path;

use super::merge::run_git_seq;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowConfig {
    pub main: String,
    pub develop: String,
    pub feature: String,
    pub release: String,
    pub hotfix: String,
    pub version_tag: String,
}

impl Default for FlowConfig {
    fn default() -> Self {
        Self {
            main: "main".into(),
            develop: "develop".into(),
            feature: "feature/".into(),
            release: "release/".into(),
            hotfix: "hotfix/".into(),
            version_tag: String::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Feature,
    Release,
    Hotfix,
}

impl Kind {
    pub const ALL: [Self; 3] = [Self::Feature, Self::Release, Self::Hotfix];

    pub fn label(self) -> &'static str {
        match self {
            Self::Feature => "Feature",
            Self::Release => "Release",
            Self::Hotfix => "Hotfix",
        }
    }

    pub fn prefix(self, config: &FlowConfig) -> &str {
        match self {
            Self::Feature => &config.feature,
            Self::Release => &config.release,
            Self::Hotfix => &config.hotfix,
        }
    }

    /// The branch a new feature, release or hotfix starts from.
    pub fn base(self, config: &FlowConfig) -> &str {
        match self {
            Self::Hotfix => &config.main,
            _ => &config.develop,
        }
    }
}

/// The repository's Git Flow settings, or None before initialization.
pub fn config(path: &Path) -> Option<FlowConfig> {
    let repo = git2::Repository::open(path).ok()?;
    let config = repo.config().ok()?.snapshot().ok()?;
    let get = |key: &str| config.get_string(key).ok();
    let defaults = FlowConfig::default();
    Some(FlowConfig {
        main: get("gitflow.branch.master")?,
        develop: get("gitflow.branch.develop")?,
        feature: get("gitflow.prefix.feature").unwrap_or(defaults.feature),
        release: get("gitflow.prefix.release").unwrap_or(defaults.release),
        hotfix: get("gitflow.prefix.hotfix").unwrap_or(defaults.hotfix),
        version_tag: get("gitflow.prefix.versiontag").unwrap_or_default(),
    })
}

fn valid_branch(name: &str) -> Result<(), String> {
    if !name.is_empty() && git2::Branch::name_is_valid(name).unwrap_or(false) {
        Ok(())
    } else {
        Err(format!("\"{name}\" is not a valid branch name."))
    }
}

/// Save the settings and create the develop branch from main when missing.
pub fn init(path: &Path, flow: &FlowConfig) -> Result<(), String> {
    valid_branch(&flow.main)?;
    valid_branch(&flow.develop)?;
    if flow.main == flow.develop {
        return Err("The production and development branches must differ.".into());
    }
    for prefix in [&flow.feature, &flow.release, &flow.hotfix] {
        valid_branch(&format!("{prefix}name"))?;
    }
    let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
    let main = repo
        .find_branch(&flow.main, git2::BranchType::Local)
        .map_err(|_| format!("Branch {} does not exist yet.", flow.main))?;
    let tip = main.get().peel_to_commit().map_err(|e| e.to_string())?;
    if repo
        .find_branch(&flow.develop, git2::BranchType::Local)
        .is_err()
    {
        repo.branch(&flow.develop, &tip, false)
            .map_err(|e| e.to_string())?;
    }
    let mut config = repo.config().map_err(|e| e.to_string())?;
    for (key, value) in [
        ("gitflow.branch.master", &flow.main),
        ("gitflow.branch.develop", &flow.develop),
        ("gitflow.prefix.feature", &flow.feature),
        ("gitflow.prefix.release", &flow.release),
        ("gitflow.prefix.hotfix", &flow.hotfix),
        ("gitflow.prefix.versiontag", &flow.version_tag),
    ] {
        config.set_str(key, value).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn require(path: &Path) -> Result<FlowConfig, String> {
    config(path).ok_or_else(|| "Initialize Git Flow for this repository first.".into())
}

/// Branches of one kind that are in progress, without their prefix.
pub fn in_progress(path: &Path, flow: &FlowConfig, kind: Kind) -> Vec<String> {
    let Ok(repo) = git2::Repository::open(path) else {
        return Vec::new();
    };
    let Ok(branches) = repo.branches(Some(git2::BranchType::Local)) else {
        return Vec::new();
    };
    let mut names: Vec<String> = branches
        .flatten()
        .filter_map(|(branch, _)| {
            let name = branch.name().ok().flatten()?;
            name.strip_prefix(kind.prefix(flow))
                .filter(|rest| !rest.is_empty())
                .map(str::to_owned)
        })
        .collect();
    names.sort();
    names
}

/// Create `<prefix><name>` from its base branch and check it out.
pub fn start(path: &Path, kind: Kind, name: &str) -> Result<String, String> {
    let flow = require(path)?;
    let name = name.trim();
    if name.is_empty() {
        return Err(format!("Enter a {} name.", kind.label().to_lowercase()));
    }
    let branch = format!("{}{name}", kind.prefix(&flow));
    valid_branch(&branch)?;
    if kind != Kind::Feature {
        if let Some(open) = in_progress(path, &flow, kind).first() {
            return Err(format!(
                "Finish {}{open} before starting another {}.",
                kind.prefix(&flow),
                kind.label().to_lowercase()
            ));
        }
    }
    let base = kind.base(&flow);
    let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
    if repo.find_branch(&branch, git2::BranchType::Local).is_ok() {
        return Err(format!("Branch {branch} already exists."));
    }
    repo.find_branch(base, git2::BranchType::Local)
        .map_err(|_| format!("Branch {base} does not exist."))?;
    run_git_seq(path, &["checkout", "-b", &branch, base, "--"]).map_err(|e| e.user_message())?;
    Ok(branch)
}

fn require_clean(path: &Path) -> Result<(), String> {
    let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
    if repo.state() != git2::RepositoryState::Clean {
        return Err("Finish the current merge or other operation first.".into());
    }
    let mut options = git2::StatusOptions::new();
    options.include_untracked(false).include_ignored(false);
    let dirty = repo
        .statuses(Some(&mut options))
        .map_err(|e| e.to_string())?
        .iter()
        .any(|entry| entry.status() != git2::Status::CURRENT);
    if dirty {
        return Err("Commit or stash changes before finishing.".into());
    }
    Ok(())
}

fn tip(repo: &git2::Repository, branch: &str) -> Result<git2::Oid, String> {
    repo.find_branch(branch, git2::BranchType::Local)
        .ok()
        .and_then(|b| b.get().target())
        .ok_or_else(|| format!("Branch {branch} does not exist."))
}

/// Merge `source` into `target` with a merge commit. Already merged branches
/// are skipped, so Finish can be chosen again after resolving a conflict.
fn merge_into(path: &Path, target: &str, source: &str) -> Result<(), String> {
    let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
    let source_tip = tip(&repo, source)?;
    let target_tip = tip(&repo, target)?;
    if source_tip == target_tip
        || repo
            .graph_descendant_of(target_tip, source_tip)
            .map_err(|e| e.to_string())?
    {
        return Ok(());
    }
    run_git_seq(path, &["checkout", target, "--"]).map_err(|e| e.user_message())?;
    run_git_seq(path, &["merge", "--no-ff", "--no-edit", "--", source]).map_err(|e| {
        format!(
            "Merging {source} into {target} stopped: {}. Resolve and commit the merge, \
             then choose Finish again.",
            e.user_message()
        )
    })
}

/// Merge the branch back, tag releases and hotfixes, then delete the branch.
/// Returns the branch that is checked out afterwards.
pub fn finish(path: &Path, kind: Kind, name: &str) -> Result<String, String> {
    let flow = require(path)?;
    require_clean(path)?;
    if name.trim().is_empty() {
        return Err("Enter the name of the branch to finish.".into());
    }
    let branch = format!("{}{name}", kind.prefix(&flow));
    valid_branch(&branch)?;
    if branch == flow.main || branch == flow.develop || flow.main == flow.develop {
        return Err("Git Flow cannot finish a production or development branch.".into());
    }
    let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
    let finished_tip = tip(&repo, &branch)?;
    // Validate every destination before checkout or merge changes repository state.
    tip(&repo, &flow.develop)?;
    let release_tag = if kind == Kind::Feature {
        None
    } else {
        let main_tip = tip(&repo, &flow.main)?;
        let tag = format!("{}{name}", flow.version_tag);
        let reference = format!("refs/tags/{tag}");
        if tag.starts_with('-') || !git2::Reference::is_valid_name(&reference) {
            return Err(format!("\"{tag}\" is not a valid tag name."));
        }
        let exists = match repo.find_reference(&reference) {
            Ok(existing) => {
                let tagged = existing.peel_to_commit().map_err(|e| e.to_string())?.id();
                // A retry after resolving the develop merge may reuse the tag
                // created on production by the first attempt. Other collisions
                // must never silently publish the wrong release commit.
                if tagged != main_tip
                    || !(main_tip == finished_tip
                        || repo
                            .graph_descendant_of(main_tip, finished_tip)
                            .map_err(|e| e.to_string())?)
                {
                    return Err(format!(
                        "Tag {tag} already exists and does not identify this finished release."
                    ));
                }
                true
            }
            Err(error) if error.code() == git2::ErrorCode::NotFound => false,
            Err(error) => return Err(error.to_string()),
        };
        Some((tag, exists))
    };
    if kind == Kind::Feature {
        merge_into(path, &flow.develop, &branch)?;
    } else {
        merge_into(path, &flow.main, &branch)?;
        let (tag, exists) = release_tag.unwrap();
        if !exists {
            let target = tip(&repo, &flow.main)?.to_string();
            let message = format!("{} {name}", kind.label());
            run_git_seq(path, &["tag", "-a", "-m", &message, &tag, &target])
                .map_err(|e| e.user_message())?;
        }
        merge_into(path, &flow.develop, &branch)?;
    }
    run_git_seq(path, &["checkout", &flow.develop, "--"]).map_err(|e| e.user_message())?;
    // `branch -d` consults the branch's upstream, which may lag behind a
    // correctly finished local feature. Verify our actual destinations and
    // delete only the exact tip reviewed above instead.
    if tip(&repo, &branch)? != finished_tip {
        return Err("The branch changed while finishing; review it before retrying.".into());
    }
    for destination in
        std::iter::once(&flow.develop).chain((kind != Kind::Feature).then_some(&flow.main))
    {
        let destination_tip = tip(&repo, destination)?;
        if destination_tip != finished_tip
            && !repo
                .graph_descendant_of(destination_tip, finished_tip)
                .map_err(|e| e.to_string())?
        {
            return Err(format!(
                "The finished tip is not merged into {destination}; branch retained."
            ));
        }
    }
    let reference = format!("refs/heads/{branch}");
    super::merge::ensure_branch_not_checked_out(path, &reference)?;
    super::branches::save_deleted(&repo, &branch, &finished_tip.to_string())
        .map_err(|e| e.to_string())?;
    run_git_seq(
        path,
        &["update-ref", "-d", &reference, &finished_tip.to_string()],
    )
    .map_err(|e| e.user_message())?;
    // Match branch deletion's config cleanup after the compare-and-delete.
    let mut config = repo.config().map_err(|e| e.to_string())?;
    for key in ["remote", "merge", "rebase", "pushRemote", "description"] {
        let _ = config.remove_multivar(&format!("branch.{branch}.{key}"), ".*");
    }
    Ok(flow.develop)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_support::TestRepo;

    fn initialized() -> TestRepo {
        let repo = TestRepo::new();
        repo.commit("app.txt", b"v1\n");
        assert!(config(repo.path()).is_none());
        init(repo.path(), &FlowConfig::default()).unwrap();
        repo
    }

    #[test]
    fn finish_deletes_a_feature_merged_locally_even_when_its_upstream_lags() {
        let repo = initialized();
        start(repo.path(), Kind::Feature, "tracked").unwrap();
        repo.git_ok(&["branch", "upstream-behind"]);
        repo.git_ok(&[
            "branch",
            "--set-upstream-to=upstream-behind",
            "feature/tracked",
        ]);
        let work = repo.commit("local.txt", b"not yet pushed\n");
        assert_eq!(
            finish(repo.path(), Kind::Feature, "tracked").unwrap(),
            "develop"
        );
        let git = repo.open();
        assert!(git
            .find_branch("feature/tracked", git2::BranchType::Local)
            .is_err());
        assert!(git
            .graph_descendant_of(tip(&git, "develop").unwrap(), work)
            .unwrap());
        assert_eq!(
            std::fs::read(repo.path().join("local.txt")).unwrap(),
            b"not yet pushed\n"
        );
        assert!(super::super::branches::load_deleted(repo.path())
            .iter()
            .any(|saved| saved.name == "feature/tracked" && saved.oid == work.to_string()));
    }

    fn branch_tip(repo: &TestRepo, name: &str) -> git2::Oid {
        tip(&repo.open(), name).unwrap()
    }

    #[test]
    fn init_saves_compatible_settings_and_creates_develop() {
        let repo = initialized();
        let flow = config(repo.path()).unwrap();
        assert_eq!(flow, FlowConfig::default());
        assert_eq!(branch_tip(&repo, "develop"), repo.head());
        let mut same = FlowConfig::default();
        same.develop = "main".into();
        assert!(init(repo.path(), &same).is_err());
        let empty = TestRepo::new();
        assert!(init(empty.path(), &FlowConfig::default()).is_err());
    }

    #[test]
    fn feature_finish_merges_into_develop_and_deletes_branch() {
        let repo = initialized();
        assert_eq!(
            start(repo.path(), Kind::Feature, "login").unwrap(),
            "feature/login"
        );
        assert!(start(repo.path(), Kind::Feature, "login").is_err());
        let work = repo.commit("login.txt", b"login\n");
        repo.write("app.txt", b"dirty\n");
        assert!(finish(repo.path(), Kind::Feature, "login").is_err());
        repo.write("app.txt", b"v1\n");
        assert_eq!(
            finish(repo.path(), Kind::Feature, "login").unwrap(),
            "develop"
        );
        let git = repo.open();
        let develop = branch_tip(&repo, "develop");
        let merge = git.find_commit(develop).unwrap();
        assert_eq!(merge.parent_count(), 2, "feature merges use --no-ff");
        assert!(git.graph_descendant_of(develop, work).unwrap());
        assert!(git
            .find_branch("feature/login", git2::BranchType::Local)
            .is_err());
        assert_eq!(git.head().unwrap().shorthand().ok(), Some("develop"));
        assert!(crate::git::branches::load_deleted(repo.path())
            .iter()
            .any(|d| d.name == "feature/login"));
    }

    #[test]
    fn release_finish_merges_both_ways_and_tags_main() {
        let repo = initialized();
        let mut flow = FlowConfig::default();
        flow.version_tag = "v".into();
        init(repo.path(), &flow).unwrap();
        start(repo.path(), Kind::Release, "1.0").unwrap();
        assert!(start(repo.path(), Kind::Release, "1.1").is_err());
        let bump = repo.commit("version.txt", b"1.0\n");
        assert_eq!(
            in_progress(repo.path(), &flow, Kind::Release),
            vec!["1.0".to_string()]
        );
        finish(repo.path(), Kind::Release, "1.0").unwrap();
        let git = repo.open();
        let main = branch_tip(&repo, "main");
        let develop = branch_tip(&repo, "develop");
        assert!(git.graph_descendant_of(main, bump).unwrap());
        assert!(git.graph_descendant_of(develop, bump).unwrap());
        let tag = git
            .find_reference("refs/tags/v1.0")
            .unwrap()
            .peel_to_commit()
            .unwrap()
            .id();
        assert_eq!(tag, main);
        assert!(in_progress(repo.path(), &flow, Kind::Release).is_empty());
    }

    #[test]
    fn finish_rejects_bad_tags_before_changing_repository_state() {
        for tag_prefix in ["bad tag/", ""] {
            let repo = initialized();
            let flow = FlowConfig {
                version_tag: tag_prefix.into(),
                ..FlowConfig::default()
            };
            init(repo.path(), &flow).unwrap();
            start(repo.path(), Kind::Release, "1.0").unwrap();
            let work = repo.commit("version.txt", b"1.0\n");
            let main = branch_tip(&repo, "main");
            let develop = branch_tip(&repo, "develop");
            if tag_prefix.is_empty() {
                // The existing tag points to production, before this release.
                repo.git_ok(&["tag", "1.0", &main.to_string()]);
            }
            repo.write("untracked.txt", b"keep me\n");
            let index = std::fs::read(repo.open().path().join("index")).unwrap();
            assert!(finish(repo.path(), Kind::Release, "1.0").is_err());
            assert_eq!(repo.head(), work);
            assert_eq!(
                repo.open().head().unwrap().shorthand().ok(),
                Some("release/1.0")
            );
            assert_eq!(branch_tip(&repo, "main"), main);
            assert_eq!(branch_tip(&repo, "develop"), develop);
            assert_eq!(branch_tip(&repo, "release/1.0"), work);
            assert_eq!(
                std::fs::read(repo.open().path().join("index")).unwrap(),
                index
            );
            assert_eq!(repo.index_bytes("version.txt"), b"1.0\n");
            assert_eq!(
                std::fs::read(repo.path().join("version.txt")).unwrap(),
                b"1.0\n"
            );
            assert_eq!(
                std::fs::read(repo.path().join("untracked.txt")).unwrap(),
                b"keep me\n"
            );
            if tag_prefix.is_empty() {
                assert_eq!(repo.open().refname_to_id("refs/tags/1.0").unwrap(), main);
            }
        }
    }

    #[test]
    fn finish_refuses_missing_destinations_and_reserved_branches() {
        let repo = initialized();
        start(repo.path(), Kind::Release, "1.0").unwrap();
        let work = repo.commit("version.txt", b"1.0\n");
        let main = branch_tip(&repo, "main");
        repo.git_ok(&["branch", "-D", "develop"]);
        assert!(finish(repo.path(), Kind::Release, "1.0").is_err());
        assert_eq!(repo.head(), work);
        assert_eq!(branch_tip(&repo, "main"), main);
        assert!(repo.open().find_reference("refs/tags/1.0").is_err());
        assert_eq!(repo.index_bytes("version.txt"), b"1.0\n");
        assert_eq!(
            std::fs::read(repo.path().join("version.txt")).unwrap(),
            b"1.0\n"
        );

        // Settings may also have been edited outside Recorde.
        repo.open()
            .config()
            .unwrap()
            .set_str("gitflow.prefix.feature", "")
            .unwrap();
        assert!(finish(repo.path(), Kind::Feature, "main").is_err());
        assert_eq!(branch_tip(&repo, "main"), main);
        assert_eq!(repo.head(), work);
    }

    #[test]
    fn hotfix_starts_from_main_and_conflicts_can_be_resumed() {
        let repo = initialized();
        // Develop moves on with a change that conflicts with the hotfix.
        repo.git_ok(&["checkout", "develop"]);
        repo.commit("app.txt", b"develop change\n");
        start(repo.path(), Kind::Hotfix, "1.0.1").unwrap();
        assert_eq!(
            repo.open().head().unwrap().peel_to_commit().unwrap().id(),
            branch_tip(&repo, "main")
        );
        let fix = repo.commit("app.txt", b"hotfix\n");
        let error = finish(repo.path(), Kind::Hotfix, "1.0.1").unwrap_err();
        assert!(error.contains("choose Finish again"), "{error}");
        // Main already has the fix and its tag; resolve develop and resume.
        assert!(repo
            .open()
            .graph_descendant_of(branch_tip(&repo, "main"), fix)
            .unwrap());
        repo.write("app.txt", b"resolved\n");
        repo.git_ok(&["add", "app.txt"]);
        repo.git_ok(&["commit", "--no-edit"]);
        finish(repo.path(), Kind::Hotfix, "1.0.1").unwrap();
        let git = repo.open();
        assert!(git.find_reference("refs/tags/1.0.1").is_ok());
        assert!(git
            .find_branch("hotfix/1.0.1", git2::BranchType::Local)
            .is_err());
        assert!(git
            .graph_descendant_of(branch_tip(&repo, "develop"), fix)
            .unwrap());
    }
}
