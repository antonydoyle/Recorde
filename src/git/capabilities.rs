use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct RepoCapabilities {
    pub is_bare: bool,
    pub is_detached_head: bool,
    pub merge_in_progress: bool,
    pub rebase_in_progress: bool,
    pub cherry_pick_in_progress: bool,
    pub revert_in_progress: bool,
    pub has_submodules: bool,
    pub has_worktrees: bool,
    pub is_sparse: bool,
    pub is_shallow: bool,
    pub lfs_installed: bool,
    pub lfs_active: bool,
    pub lfs_patterns: Vec<String>,
    pub provider: ProviderType,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ProviderType {
    GitHub,
    GitLab,
    Bitbucket,
    #[default]
    Unknown,
}

pub fn detect(repo_path: &Path) -> RepoCapabilities {
    let Ok(repo) = git2::Repository::open(repo_path) else {
        return RepoCapabilities::default();
    };

    let git_dir = repo.path(); // .git/ directory (or bare root)

    let is_bare = repo.is_bare();

    let is_detached_head = repo.head_detached().unwrap_or(false);

    // In-progress state: look for the sentinel files git uses
    let merge_in_progress     = git_dir.join("MERGE_HEAD").exists();
    let cherry_pick_in_progress = git_dir.join("CHERRY_PICK_HEAD").exists();
    let revert_in_progress    = git_dir.join("REVERT_HEAD").exists();
    let rebase_in_progress    = git_dir.join("rebase-merge").exists()
        || git_dir.join("rebase-apply").exists();

    // Submodules: .gitmodules in the workdir
    let has_submodules = repo.workdir()
        .map(|wd| wd.join(".gitmodules").exists())
        .unwrap_or(false);

    // Linked worktrees: .git/worktrees/ directory with entries
    let has_worktrees = git_dir.join("worktrees").exists()
        && git_dir.join("worktrees").read_dir().map(|mut d| d.next().is_some()).unwrap_or(false);

    // Sparse checkout: config core.sparseCheckout = true
    let is_sparse = repo.config()
        .ok()
        .and_then(|c| c.get_bool("core.sparseCheckout").ok())
        .unwrap_or(false);

    // Shallow: .git/shallow exists and is non-empty
    let is_shallow = git_dir.join("shallow").metadata().map(|m| m.len() > 0).unwrap_or(false);

    // LFS: `git lfs version` exit-zero means installed
    let lfs_installed = crate::git::lfs::is_lfs_installed();

    // LFS: check whether this repo actively uses LFS
    let workdir = repo.workdir().unwrap_or(repo_path);
    let lfs_active = crate::git::lfs::repo_uses_lfs(workdir);
    let lfs_patterns = crate::git::lfs::tracked_patterns(workdir);

    // Provider: detect from first remote URL
    let provider = detect_provider(&repo);

    RepoCapabilities {
        is_bare,
        is_detached_head,
        merge_in_progress,
        rebase_in_progress,
        cherry_pick_in_progress,
        revert_in_progress,
        has_submodules,
        has_worktrees,
        is_sparse,
        is_shallow,
        lfs_installed,
        lfs_active,
        lfs_patterns,
        provider,
    }
}

fn detect_provider(repo: &git2::Repository) -> ProviderType {
    let Ok(remotes) = repo.remotes() else { return ProviderType::Unknown };
    for name in remotes.iter().flatten() {
        let Ok(remote) = repo.find_remote(name) else { continue };
        let Some(url) = remote.url() else { continue };
        if url.contains("github.com") {
            return ProviderType::GitHub;
        }
        if url.contains("gitlab.com") || url.contains("gitlab.") {
            return ProviderType::GitLab;
        }
        if url.contains("bitbucket.org") {
            return ProviderType::Bitbucket;
        }
    }
    ProviderType::Unknown
}
