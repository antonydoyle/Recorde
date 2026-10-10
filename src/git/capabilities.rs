use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct RepoCapabilities {
    pub provider_target: Option<(String, String)>,
    pub is_bare: bool,
    pub is_detached_head: bool,
    pub merge_in_progress: bool,
    pub rebase_in_progress: bool,
    pub rebase_edit: bool,
    pub rebase_split: bool,
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
    AzureDevOps,
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
    let merge_in_progress = git_dir.join("MERGE_HEAD").exists();
    let cherry_pick_in_progress = git_dir.join("CHERRY_PICK_HEAD").exists();
    let revert_in_progress = git_dir.join("REVERT_HEAD").exists();
    let rebase_in_progress =
        git_dir.join("rebase-merge").exists() || git_dir.join("rebase-apply").exists();

    // Submodules: .gitmodules in the workdir
    let has_submodules = repo
        .workdir()
        .map(|wd| wd.join(".gitmodules").exists())
        .unwrap_or(false);

    // Linked worktrees: .git/worktrees/ directory with entries
    let has_worktrees = repo.worktrees().is_ok_and(|names| !names.is_empty());

    // Sparse checkout: config core.sparseCheckout = true
    let is_sparse = repo
        .config()
        .ok()
        .and_then(|c| c.get_bool("core.sparseCheckout").ok())
        .unwrap_or(false);

    // Shallow: .git/shallow exists and is non-empty
    let is_shallow = repo.is_shallow();

    // LFS: `git lfs version` exit-zero means installed
    let lfs_installed = crate::git::lfs::is_lfs_installed();

    // LFS: check whether this repo actively uses LFS
    let workdir = repo.workdir().unwrap_or(repo_path);
    let lfs_active = crate::git::lfs::repo_uses_lfs(workdir);
    let lfs_patterns = crate::git::lfs::tracked_patterns(workdir);

    // Provider: resolve origin using exact parsed hosts and configured accounts.
    let provider = detect_provider(&repo);

    RepoCapabilities {
        provider_target: super::remote::provider_remote(&repo).ok().and_then(|name| {
            let url = repo.find_remote(&name).ok()?.url().ok()?.to_string();
            Some((name, url))
        }),
        is_bare,
        is_detached_head,
        merge_in_progress,
        rebase_in_progress,
        rebase_edit: super::rebase::is_edit_pause(&repo),
        rebase_split: git_dir.join("rebase-merge/recorde-split").is_file(),
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
    let Ok(name) = super::remote::provider_remote(repo) else {
        return ProviderType::Unknown;
    };
    let Ok(remote) = repo.find_remote(&name) else {
        return ProviderType::Unknown;
    };
    let Some(url) = remote.url().ok() else {
        return ProviderType::Unknown;
    };
    let Ok(endpoint) = crate::auth::endpoint::parse(url, true) else {
        return ProviderType::Unknown;
    };
    match endpoint.host.as_str() {
        "github.com" => return ProviderType::GitHub,
        "gitlab.com" => return ProviderType::GitLab,
        "bitbucket.org" => return ProviderType::Bitbucket,
        "dev.azure.com" => return ProviderType::AzureDevOps,
        _ => {}
    }
    let config = crate::config::Config::load();
    let binding = config.get_repo_config(repo.workdir().unwrap_or(repo.path()));
    let username = binding
        .provider_accounts
        .get(&endpoint.host)
        .or(endpoint.username.as_ref());
    let account = config.accounts.iter().find(|account| {
        crate::auth::endpoint::normalize_host(&account.host)
            .ok()
            .as_deref()
            == Some(&endpoint.host)
            && username.is_none_or(|user| user == &account.username)
    });
    match account.map(|account| &account.provider) {
        Some(
            crate::auth::ProviderKind::GitHubCom | crate::auth::ProviderKind::GitHubEnterprise,
        ) => ProviderType::GitHub,
        Some(crate::auth::ProviderKind::GitLabCom | crate::auth::ProviderKind::GitLabSelf) => {
            ProviderType::GitLab
        }
        Some(crate::auth::ProviderKind::BitbucketCloud) => ProviderType::Bitbucket,
        Some(crate::auth::ProviderKind::AzureDevOps) => ProviderType::AzureDevOps,
        None => ProviderType::Unknown,
    }
}
