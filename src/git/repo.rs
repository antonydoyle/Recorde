use std::path::Path;

use git2::Repository;

pub struct Repo {
    pub inner: Repository,
}

impl Repo {
    pub fn open(path: &Path) -> Result<Self, git2::Error> {
        let inner = Repository::open(path)?;
        Ok(Self { inner })
    }

    pub fn path(&self) -> &Path {
        self.inner.path()
    }
}
