use std::path::Path;
use std::process::Output;

pub struct TestRepo {
    dir: tempfile::TempDir,
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
