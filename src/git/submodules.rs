use std::path::Path;

pub fn update(repo: &Path, path: &str) -> Result<(), String> {
    crate::git::merge::run_git_seq(
        repo,
        &["submodule", "update", "--init", "--recursive", "--", path],
    )
    .map_err(|e| e.user_message())
}
pub fn sync(repo: &Path, path: &str) -> Result<(), String> {
    crate::git::merge::run_git_seq(repo, &["submodule", "sync", "--recursive", "--", path])
        .map_err(|e| e.user_message())
}

#[derive(Debug, Clone)]
pub struct SubmoduleInfo {
    pub name: String,
    pub path: String,
    pub url: String,
    pub initialized: bool,
}

/// Parse `.gitmodules` and return one `SubmoduleInfo` per submodule.
/// Returns an empty Vec if `.gitmodules` is absent or unparseable.
pub fn list(repo_path: &Path) -> Vec<SubmoduleInfo> {
    let Ok(repo) = git2::Repository::open(repo_path) else {
        return Vec::new();
    };
    let Ok(modules) = repo.submodules() else {
        return Vec::new();
    };
    modules
        .into_iter()
        .filter_map(|module| {
            let name = module.name().ok()?.to_string();
            let path = module.path().to_string_lossy().into_owned();
            let url = module.url().ok().flatten().unwrap_or_default().to_string();
            let initialized = module.open().is_ok();
            Some(SubmoduleInfo {
                name,
                path,
                url,
                initialized,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_support::TestRepo;
    #[test]
    fn submodule_paths_with_spaces_can_sync_and_update_the_recorded_commit() {
        let source = TestRepo::new();
        let base = source.commit("file", b"base\n");
        source.commit("file", b"new\n");
        let repo = TestRepo::new();
        repo.commit("base", b"base");
        repo.git_ok(&[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            source.path().to_str().unwrap(),
            "nested module",
        ]);
        let modules = list(repo.path());
        assert_eq!(modules.len(), 1);
        assert_eq!(modules[0].path, "nested module");
        assert!(modules[0].initialized);
        repo.git_ok(&["-C", "nested module", "checkout", &base.to_string()]);
        update(repo.path(), "nested module").unwrap();
        assert_eq!(
            std::fs::read(repo.path().join("nested module/file")).unwrap(),
            b"new\n"
        );
        repo.git_ok(&[
            "config",
            "-f",
            ".gitmodules",
            "submodule.nested module.url",
            "https://example.invalid/replacement.git",
        ]);
        sync(repo.path(), "nested module").unwrap();
        assert_eq!(
            repo.open()
                .config()
                .unwrap()
                .get_string("submodule.nested module.url")
                .unwrap(),
            "https://example.invalid/replacement.git"
        );
    }
}
