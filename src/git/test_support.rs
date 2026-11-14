use std::path::Path;
use std::process::Output;

pub struct TestRepo {
    dir: tempfile::TempDir,
}

#[derive(Debug, PartialEq, Eq)]
pub struct RepoSnapshot {
    head_ref: Option<String>,
    head: Option<git2::Oid>,
    head_tree: Option<git2::Oid>,
    index: Vec<(Vec<u8>, git2::Oid, u32, u16)>,
    index_tree: Option<git2::Oid>,
    branches: std::collections::BTreeMap<String, git2::Oid>,
    files: std::collections::BTreeMap<std::path::PathBuf, SnapshotEntry>,
    status: Vec<(String, u32)>,
    operation: String,
}

#[derive(Debug, PartialEq, Eq)]
enum SnapshotEntry {
    Directory,
    File(Vec<u8>),
    Link(std::path::PathBuf),
}

/// Compare user-visible state, excluding Git caches/reflogs and recovery bookkeeping.
pub fn snapshot(path: &Path) -> RepoSnapshot {
    let repo = git2::Repository::open(path).unwrap();
    let head = repo.head().ok();
    let mut index = repo.index().unwrap();
    let entries = index
        .iter()
        .map(|e| (e.path, e.id, e.mode, e.flags & 0x3000))
        .collect();
    let index_tree = index.write_tree_to(&repo).ok();
    let branches = repo
        .references()
        .unwrap()
        .filter_map(|r| {
            let r = r.unwrap();
            let name = r.name().unwrap();
            (name.starts_with("refs/heads/"))
                .then(|| (name.to_string(), r.resolve().unwrap().target().unwrap()))
        })
        .collect();
    let mut files = std::collections::BTreeMap::new();
    fn collect_files(
        root: &Path,
        directory: &Path,
        files: &mut std::collections::BTreeMap<std::path::PathBuf, SnapshotEntry>,
    ) {
        for entry in std::fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let relative = path.strip_prefix(root).unwrap().to_path_buf();
            if relative == Path::new(".git") {
                continue;
            }
            let kind = entry.file_type().unwrap();
            let value = if kind.is_symlink() {
                SnapshotEntry::Link(std::fs::read_link(&path).unwrap())
            } else if kind.is_dir() {
                SnapshotEntry::Directory
            } else {
                SnapshotEntry::File(std::fs::read(&path).unwrap())
            };
            files.insert(relative, value);
            if kind.is_dir() {
                collect_files(root, &path, files);
            }
        }
    }
    collect_files(path, path, &mut files);
    let mut options = git2::StatusOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(true)
        .recurse_ignored_dirs(true);
    let status = repo
        .statuses(Some(&mut options))
        .unwrap()
        .iter()
        .map(|s| (s.path().unwrap().to_string(), s.status().bits()))
        .collect();
    let head_ref = repo
        .find_reference("HEAD")
        .unwrap()
        .symbolic_target()
        .unwrap()
        .map(str::to_string);
    RepoSnapshot {
        head_ref,
        head: head.as_ref().and_then(|r| r.target()),
        head_tree: head
            .as_ref()
            .and_then(|r| r.peel_to_tree().ok())
            .map(|t| t.id()),
        index: entries,
        index_tree,
        branches,
        files,
        status,
        operation: format!("{:?}", repo.state()),
    }
}

/// The destination tracks a file that is ignored and locally present on main.
pub fn ignored_collision() -> (TestRepo, git2::Oid) {
    let fixture = TestRepo::new();
    fixture.commit(".gitignore", b"/ignored.env\n");
    let target = fixture.commit("ignored.env", b"destination tracked bytes\n");
    fixture.git_ok(&["branch", "destination"]);
    fixture.git_ok(&["rm", "ignored.env"]);
    fixture.git_ok(&["commit", "-m", "remove from main"]);
    fixture.write("ignored.env", b"local ignored bytes\r\n\xff\x00");
    (fixture, target)
}

impl TestRepo {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut opts = git2::RepositoryInitOptions::new();
        opts.initial_head("main");
        let repo = git2::Repository::init_opts(dir.path(), &opts).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "Test Committer").unwrap();
        config
            .set_str("user.email", "committer@example.com")
            .unwrap();
        config.set_bool("commit.gpgsign", false).unwrap();
        config.set_bool("core.autocrlf", false).unwrap();
        config.set_bool("core.safecrlf", false).unwrap();
        let hooks = dir.path().join("empty-hooks");
        std::fs::create_dir(&hooks).unwrap();
        config
            .set_str("core.hooksPath", hooks.to_str().unwrap())
            .unwrap();
        Self { dir }
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    pub fn open(&self) -> git2::Repository {
        git2::Repository::open(self.path()).unwrap()
    }

    pub fn write(&self, name: &str, content: &[u8]) {
        std::fs::write(self.path().join(name), content).unwrap();
    }

    pub fn commit(&self, name: &str, content: &[u8]) -> git2::Oid {
        self.write(name, content);
        let repo = self.open();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new(name)).unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let author = git2::Signature::now("Original Author", "author@example.com").unwrap();
        let committer = repo.signature().unwrap();
        let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
        let parents: Vec<_> = parent.iter().collect();
        repo.commit(
            Some("HEAD"),
            &author,
            &committer,
            "fixture commit",
            &tree,
            &parents,
        )
        .unwrap()
    }

    pub fn index_bytes(&self, name: &str) -> Vec<u8> {
        let repo = self.open();
        let entry = repo.index().unwrap().get_path(Path::new(name), 0).unwrap();
        let blob = repo.find_blob(entry.id).unwrap();
        blob.content().to_vec()
    }

    pub fn head(&self) -> git2::Oid {
        self.open().head().unwrap().target().unwrap()
    }

    pub fn git(&self, args: &[&str]) -> Output {
        let mut command = super::process::background_command("git");
        command
            .arg("-C")
            .arg(self.path())
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env(
                "GIT_CONFIG_GLOBAL",
                self.path().join("absent-global-config"),
            )
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_EDITOR", "true");
        super::process::run(
            &mut command,
            &std::sync::atomic::AtomicBool::new(false),
            std::time::Duration::from_secs(30),
        )
        .unwrap()
    }

    pub fn git_ok(&self, args: &[&str]) {
        let out = self.git(args);
        assert!(
            out.status.success(),
            "git {:?}: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
