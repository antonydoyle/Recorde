//! Witnessed branch switches and retained, explicitly restored saved work.
//! Submitted phases are persisted before Git: an interrupted operation is never replayed.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

const RECORD_LIMIT: u64 = 65_536;
const RECORD_COUNT: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    Local(String),
    Remote(String),
    New {
        name: String,
        oid: Option<String>,
    },
    Detached(String),
    Review {
        branch: String,
        oid: String,
        remote: String,
        remote_url_digest: String,
        reference: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Destination {
    branch: Option<String>,
    oid: String,
    create: bool,
    upstream: Option<String>,
    watched_refs: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Witness {
    head_ref: Option<String>,
    head_oid: String,
    files: super::recovery::TrackedState,
    index_entries: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preview {
    pub path: PathBuf,
    pub epoch: u64,
    git_dir: PathBuf,
    target: Target,
    destination: Destination,
    source: Witness,
    replacements: BTreeMap<String, String>,
    pub tracked: usize,
    pub untracked: usize,
    pub ignored: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    Prepared,
    StashSubmitted,
    Stashed,
    Switching,
    Switched,
    CheckoutFailed,
    CheckoutUnknown,
    RestoreSubmitted,
    Restored,
    RestoreFailed,
    RestoreUnknown,
}

impl Phase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Prepared => "Switch prepared; work has not been saved",
            Self::StashSubmitted => {
                "Saving work was interrupted; inspect stashes before continuing"
            }
            Self::Stashed => "Work saved; branch has not been switched",
            Self::Switching => "Checkout may have completed; inspect the current branch",
            Self::Switched => "Branch switched; saved work is available",
            Self::CheckoutFailed => "Checkout failed; saved work is retained",
            Self::CheckoutUnknown => "Checkout outcome needs inspection; saved work is retained",
            Self::RestoreSubmitted | Self::RestoreUnknown => {
                "Restore may have changed files; inspect and reconcile before applying again"
            }
            Self::Restored => "Saved work restored; stash retained",
            Self::RestoreFailed => "Restore stopped; inspect files and conflicts; stash retained",
        }
    }

    fn can_restore(self) -> bool {
        matches!(
            self,
            Self::Stashed
                | Self::Switching
                | Self::Switched
                | Self::CheckoutFailed
                | Self::CheckoutUnknown
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedWork {
    schema: u32,
    pub id: String,
    preview: Preview,
    pub phase: Phase,
    pub stash_oid: Option<String>,
    previous_stash: Option<String>,
    recovery_id: String,
    #[serde(default)]
    restore_before: Option<Witness>,
    pub outcome: Option<String>,
}

#[derive(Clone)]
pub struct RestorePreview {
    pub record: SavedWork,
    bytes: Vec<u8>,
    current: Witness,
    replacements: BTreeMap<String, String>,
    pub epoch: u64,
    pub tracked: usize,
    pub untracked: usize,
    pub ignored: usize,
}

#[derive(Clone, Copy)]
pub enum Choice {
    Bring,
    Stash,
}

fn oid(value: &str) -> Result<git2::Oid, String> {
    git2::Oid::from_str(value).map_err(|_| "Invalid saved-work object identity.".into())
}

fn replacements(repo: &git2::Repository) -> Result<BTreeMap<String, String>, String> {
    super::replacement::refs(repo)
        .map(|refs| {
            refs.into_iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect()
        })
        .map_err(|e| e.to_string())
}

fn clean_operation(repo: &git2::Repository) -> Result<(), String> {
    if repo.state() != git2::RepositoryState::Clean || super::merge::squash_in_progress(repo) {
        return Err("Finish the current Git operation before switching or restoring work.".into());
    }
    // Native stash does not save nested repositories or dirty submodule contents.
    for module in repo.submodules().map_err(|e| e.to_string())? {
        let status = repo
            .submodule_status(
                module.name().map_err(|e| e.to_string())?,
                git2::SubmoduleIgnore::None,
            )
            .map_err(|e| e.to_string())?;
        if status.intersects(
            git2::SubmoduleStatus::WD_MODIFIED
                | git2::SubmoduleStatus::WD_INDEX_MODIFIED
                | git2::SubmoduleStatus::WD_WD_MODIFIED
                | git2::SubmoduleStatus::WD_UNTRACKED,
        ) {
            return Err("Commit or stash changes inside submodules before switching.".into());
        }
    }
    Ok(())
}

fn witness(path: &Path, repo: &git2::Repository) -> Result<Witness, String> {
    use sha2::{Digest, Sha256};
    clean_operation(repo)?;
    let head = repo.find_reference("HEAD").map_err(|e| e.to_string())?;
    let head_ref = head
        .symbolic_target()
        .map_err(|e| e.to_string())?
        .map(str::to_owned);
    let head_oid = head
        .resolve()
        .and_then(|h| h.peel_to_commit())
        .map_err(|_| "Make the first commit before using safe branch switching.")?
        .id()
        .to_string();
    let files = super::recovery::complete_state(path, repo)?;
    let mut digest = Sha256::new();
    for entry in repo.index().map_err(|e| e.to_string())?.iter() {
        digest.update((entry.path.len() as u64).to_le_bytes());
        digest.update(entry.path);
        digest.update(entry.id.as_bytes());
        digest.update(entry.mode.to_le_bytes());
        digest.update((entry.flags & 0x3000).to_le_bytes());
        digest.update(entry.flags_extended.to_le_bytes());
    }
    Ok(Witness {
        head_ref,
        head_oid,
        files,
        index_entries: format!("{:x}", digest.finalize()),
    })
}

fn destination(repo: &git2::Repository, target: &Target) -> Result<Destination, String> {
    let mut watched_refs = BTreeMap::new();
    let mut watch = |name: &str| -> Result<String, String> {
        let value = repo
            .refname_to_id(name)
            .map_err(|_| format!("{name} changed or no longer exists. Review the switch again."))?
            .to_string();
        watched_refs.insert(name.into(), value.clone());
        Ok(value)
    };
    let mut upstream = None;
    let (branch, value, create) = match target {
        Target::Local(name) => (
            Some(name.clone()),
            watch(&format!("refs/heads/{name}"))?,
            false,
        ),
        Target::Detached(value) => (None, value.clone(), false),
        Target::New { name, oid: value } => (
            Some(name.clone()),
            value.clone().unwrap_or(
                repo.head()
                    .and_then(|h| h.peel_to_commit())
                    .map_err(|e| e.to_string())?
                    .id()
                    .to_string(),
            ),
            true,
        ),
        Target::Review {
            branch,
            oid: value,
            remote,
            remote_url_digest,
            reference,
        } => {
            let url = repo
                .find_remote(remote)
                .map_err(|e| e.to_string())?
                .url()
                .map_err(|e| e.to_string())?
                .to_string();
            if url_digest(&url) != *remote_url_digest || watch(reference)? != *value {
                return Err("Review source changed. Refresh before checking out.".into());
            }
            if let Ok(existing) = repo.find_branch(branch, git2::BranchType::Local) {
                if existing.get().target().map(|o| o.to_string()).as_ref() != Some(value) {
                    return Err(
                        "The review branch contains local commits. Rename it before continuing."
                            .into(),
                    );
                }
                (
                    Some(branch.clone()),
                    watch(&format!("refs/heads/{branch}"))?,
                    false,
                )
            } else {
                (Some(branch.clone()), value.clone(), true)
            }
        }
        Target::Remote(shorthand) => {
            let (_, name) = super::branches::split_remote_branch(repo, shorthand)?;
            let remote_ref = format!("refs/remotes/{shorthand}");
            let remote_oid = watch(&remote_ref)?;
            upstream = Some(remote_ref.clone());
            if let Ok(local) = repo.find_branch(&name, git2::BranchType::Local) {
                if local
                    .upstream()
                    .ok()
                    .and_then(|b| b.get().name().ok().map(str::to_owned))
                    .as_deref()
                    != Some(&remote_ref)
                {
                    return Err(format!("Local branch {name} does not track {shorthand}. Choose it from Local branches or use another name."));
                }
                (
                    Some(name.clone()),
                    watch(&format!("refs/heads/{name}"))?,
                    false,
                )
            } else {
                (Some(name), remote_oid, true)
            }
        }
    };
    let commit = repo.find_commit(oid(&value)?).map_err(|e| e.to_string())?;
    // Resolve now so malformed/cyclic replacement maps fail before a snapshot.
    super::replacement::commit(repo, commit.id()).map_err(|e| e.to_string())?;
    if let Some(name) = &branch {
        if !git2::Branch::name_is_valid(name).map_err(|e| e.to_string())? {
            return Err("Invalid destination branch name.".into());
        }
        if create && repo.find_reference(&format!("refs/heads/{name}")).is_ok() {
            return Err("The destination branch already exists. Review the switch again.".into());
        }
        super::merge::ensure_branch_not_checked_out(
            repo.workdir().ok_or("A worktree is required.")?,
            &format!("refs/heads/{name}"),
        )?;
    }
    Ok(Destination {
        branch,
        oid: value,
        create,
        upstream,
        watched_refs,
    })
}

impl Preview {
    pub fn capture(path: &Path, epoch: u64, target: Target) -> Result<Self, String> {
        let path = path.canonicalize().map_err(|e| e.to_string())?;
        let repo = git2::Repository::open(&path).map_err(|e| e.to_string())?;
        let source = witness(&path, &repo)?;
        let destination = destination(&repo, &target)?;
        let (tracked, untracked, ignored) = counts(&repo)?;
        Ok(Self {
            path,
            epoch,
            git_dir: repo.path().canonicalize().map_err(|e| e.to_string())?,
            target,
            destination,
            source,
            replacements: replacements(&repo)?,
            tracked,
            untracked,
            ignored,
        })
    }

    pub fn target_label(&self) -> &str {
        self.destination
            .branch
            .as_deref()
            .unwrap_or(&self.destination.oid)
    }
    pub fn is_detached(&self) -> bool {
        self.destination.branch.is_none()
    }
    pub fn target_oid(&self) -> &str {
        &self.destination.oid
    }
    pub fn source_label(&self) -> &str {
        self.source
            .head_ref
            .as_deref()
            .unwrap_or(&self.source.head_oid)
    }
    pub fn has_changes(&self) -> bool {
        self.tracked > 0 || self.untracked > 0
    }

    fn validate(&self) -> Result<git2::Repository, String> {
        let repo = git2::Repository::open(&self.path).map_err(|e| e.to_string())?;
        if repo.path().canonicalize().map_err(|e| e.to_string())? != self.git_dir
            || replacements(&repo)? != self.replacements
            || witness(&self.path, &repo)? != self.source
            || destination(&repo, &self.target)? != self.destination
        {
            return Err(
                "Branch, target, index or working files changed. Review the switch again.".into(),
            );
        }
        Ok(repo)
    }

    pub fn execute(
        &self,
        choice: Choice,
        session: impl Fn() -> Result<(), String>,
    ) -> Result<(), String> {
        session()?;
        self.validate()?;
        match choice {
            Choice::Bring => {
                super::recovery::protect(&self.path, "Switch branch with changes", || {
                    session()?;
                    self.validate()?;
                    checkout(&self.path, &self.destination)
                })
            }
            Choice::Stash => self.stash_switch(session),
        }
    }

    fn stash_switch(&self, session: impl Fn() -> Result<(), String>) -> Result<(), String> {
        if !self.has_changes() {
            return Err("There are no changes to save. Choose Switch branch.".into());
        }
        let repo = self.validate()?;
        // Reject redirected metadata before the original recovery journal is written.
        let _metadata = journal_dir(repo.path())?;
        // A full original snapshot is retained even if Git or journal creation later fails.
        let recovery = super::recovery::capture(&self.path, "Saved work before branch switch")?
            .ok_or("Cannot capture original work.")?;
        super::recovery::set_pinned(&self.path, &recovery.id, true)?;
        session()?;
        self.validate()?;
        let mut record = SavedWork {
            schema: 1,
            id: new_id(),
            preview: self.clone(),
            phase: Phase::Prepared,
            stash_oid: None,
            previous_stash: repo.refname_to_id("refs/stash").ok().map(|o| o.to_string()),
            recovery_id: recovery.id,
            restore_before: None,
            outcome: None,
        };
        record.write(None)?;
        fault("prepared")?;
        let mut bytes = record.bytes()?;
        session()?;
        self.validate()?;
        record.phase = Phase::StashSubmitted;
        record.write(Some(&bytes))?;
        bytes = record.bytes()?;
        fault("stash-submitted")?;
        let marker = record.marker();
        // An error/cancellation may still have saved work: keep Submitted, never retry automatically.
        super::process::git(
            &self.path,
            &["stash", "push", "--include-untracked", "-m", &marker],
            &AtomicBool::new(false),
        )
        .map_err(|e| {
            format!(
                "Saving work stopped. Inspect stashes before continuing: {}",
                e.user_message()
            )
        })?;
        fault("stash-created")?;
        let refreshed = git2::Repository::open(&self.path).map_err(|e| e.to_string())?;
        let saved = refreshed.refname_to_id("refs/stash").map_err(|_| {
            "Git did not produce a saved-work object. Inspect the repository before continuing."
        })?;
        if record.previous_stash.as_deref() == Some(&saved.to_string()) {
            return Err("No new stash was created. Existing stashes were left untouched.".into());
        }
        record.verify_stash(&refreshed, saved)?;
        // No force: a colliding or externally edited pin is a refusal.
        refreshed.reference(&record.pin(),saved,false,&marker).map_err(|e|format!("Work was saved, but its recovery pin could not be created. Inspect stashes: {e}"))?;
        fault("stash-pinned")?;
        record.stash_oid = Some(saved.to_string());
        record.phase = Phase::Stashed;
        record.write(Some(&bytes))?;
        bytes = record.bytes()?;
        fault("stashed")?;
        session()?;
        self.validate_after_stash(&refreshed)?;
        session()?;
        record.phase = Phase::Switching;
        record.write(Some(&bytes))?;
        bytes = record.bytes()?;
        fault("switching")?;
        session()?;
        let result = checkout(&self.path, &self.destination);
        fault("checkout-completed")?;
        record.phase = if result.is_ok() {
            Phase::Switched
        } else {
            Phase::CheckoutUnknown
        };
        record.outcome = result.as_ref().err().map(|e| crate::auth::redact::text(e));
        record.write(Some(&bytes)).map_err(|e|format!("Checkout may have completed; saved work is retained. Cannot record its outcome: {e}"))?;
        result.map_err(|e|format!("{e}\nSaved work is retained. Inspect the current branch, then use Saved work to restore it explicitly."))
    }

    fn validate_after_stash(&self, repo: &git2::Repository) -> Result<(), String> {
        let current = witness(&self.path, repo)?;
        let head_tree = repo
            .find_commit(oid(&self.source.head_oid)?)
            .map_err(|e| e.to_string())?
            .tree_id()
            .to_string();
        if current.head_ref != self.source.head_ref
            || current.head_oid != self.source.head_oid
            || current.files.index != head_tree
            || current.files.worktree != head_tree
            || destination(repo, &self.target)? != self.destination
            || replacements(repo)? != self.replacements
        {
            return Err("Repository changed while saving work. Checkout was not started; inspect the retained stash.".into());
        }
        Ok(())
    }
}

fn checkout(path: &Path, target: &Destination) -> Result<(), String> {
    let mut args = vec!["checkout", "--no-overwrite-ignore"];
    if target.create {
        if target.upstream.is_some() {
            args.push("--track");
        }
        args.extend([
            "-b",
            target.branch.as_deref().ok_or("Missing branch name.")?,
        ]);
        args.push(target.upstream.as_deref().unwrap_or(&target.oid));
    } else if let Some(branch) = &target.branch {
        args.push(branch);
    } else {
        args.extend(["--detach", &target.oid]);
    }
    args.push("--");
    super::merge::run_git_seq(path, &args).map_err(|e| e.user_message())
}

fn new_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!(
        "{}-{}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}
pub(super) fn url_digest(url: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(url.as_bytes()))
}
fn valid_id(id: &str) -> bool {
    let parts: Vec<_> = id.split('-').collect();
    (16..=96).contains(&id.len())
        && parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|c| c.is_ascii_digit()))
}

// Hold each metadata ancestor without following links, including during creation.
fn journal_dir(git_dir: &Path) -> Result<cap_std::fs::Dir, String> {
    use cap_fs_ext::DirExt;
    let mut parent = cap_std::fs::Dir::open_ambient_dir(git_dir, cap_std::ambient_authority())
        .map_err(|e| e.to_string())?;
    for part in ["recorde", "switch"] {
        match parent.create_dir(part) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.to_string()),
        }
        #[cfg(unix)]
        sync_directory(&parent)?;
        let next = parent
            .open_dir_nofollow(part)
            .map_err(|_| "Cannot open saved-work metadata through a link or reparse point.")?;
        #[cfg(windows)]
        {
            use cap_std::fs::MetadataExt;
            if next
                .dir_metadata()
                .map_err(|e| e.to_string())?
                .file_attributes()
                & 0x400
                != 0
            {
                return Err("Saved-work metadata is a reparse point.".into());
            }
        }
        parent = next;
    }
    Ok(parent)
}

#[cfg(unix)]
fn sync_directory(directory: &cap_std::fs::Dir) -> Result<(), String> {
    use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
    // cap-std directory capabilities may use O_PATH descriptors on Linux,
    // which cannot be fsynced. Reopen this held directory, without using an
    // ambient path, to obtain a read-capable descriptor for the durability barrier.
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let file = directory
        .open_with(".", &options)
        .map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_dir() {
        return Err("Saved-work durability barrier requires a directory.".into());
    }
    file.sync_all()
        .map_err(|e| format!("Cannot sync saved-work directory: {e}"))
}

impl SavedWork {
    pub fn source_label(&self) -> &str {
        self.preview.source_label()
    }
    pub fn target_label(&self) -> &str {
        self.preview.target_label()
    }
    fn marker(&self) -> String {
        format!("Recorde branch switch {}", self.id)
    }
    fn pin(&self) -> String {
        format!("refs/recorde/switch-stashes/{}", self.id)
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        serde_json::to_vec(self).map_err(|e| e.to_string())
    }
    fn write(&self, expected: Option<&[u8]>) -> Result<(), String> {
        if !valid_id(&self.id) {
            return Err("Invalid saved-work operation identity.".into());
        }
        let directory = journal_dir(&self.preview.git_dir)?;
        if expected.is_none()
            && directory
                .entries()
                .map_err(|e| e.to_string())?
                .take(RECORD_COUNT)
                .count()
                >= RECORD_COUNT
        {
            return Err("Saved-work journal limit reached. Inspect existing records before saving another switch.".into());
        }
        let bytes = self.bytes()?;
        if bytes.len() as u64 > RECORD_LIMIT {
            return Err("Saved-work journal exceeds its size limit.".into());
        }
        super::safe_path::ScopedFile::new(
            &self.preview.git_dir,
            &format!("recorde/switch/{}.json", self.id),
        )?
        .replace(expected, &bytes)?;
        #[cfg(unix)]
        sync_directory(&directory)?;
        Ok(())
    }

    fn validate_record(&self, path: &Path) -> Result<git2::Repository, String> {
        if self.schema != 1
            || !valid_id(&self.id)
            || path.canonicalize().map_err(|e| e.to_string())? != self.preview.path
        {
            return Err("Saved-work journal does not belong to this repository.".into());
        }
        let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
        if repo.path().canonicalize().map_err(|e| e.to_string())? != self.preview.git_dir {
            return Err("Saved-work Git directory changed.".into());
        }
        oid(&self.preview.source.head_oid)?;
        oid(&self.preview.source.files.index)?;
        oid(&self.preview.source.files.worktree)?;
        oid(&self.preview.destination.oid)?;
        if let Some(saved) = &self.stash_oid {
            self.verify_stash(&repo, oid(saved)?)?;
        }
        Ok(repo)
    }

    fn verify_stash(&self, repo: &git2::Repository, saved: git2::Oid) -> Result<(), String> {
        let stash = repo.find_commit(saved).map_err(|e| e.to_string())?;
        if stash.parent_count() < 2
            || stash.parent_count() > 3
            || stash.parent_id(0).map_err(|e| e.to_string())? != oid(&self.preview.source.head_oid)?
            || !stash
                .message()
                .unwrap_or_default()
                .ends_with(&self.marker())
        {
            return Err("The new stash does not match this switch operation. Checkout was not started; inspect saved work.".into());
        }
        let index = stash.parent(1).map_err(|e| e.to_string())?;
        if index.tree_id() != oid(&self.preview.source.files.index)? {
            return Err(
                "Saved index differs from the reviewed work. Inspect the retained stash.".into(),
            );
        }
        let mut combined = git2::Index::new().map_err(|e| e.to_string())?;
        combined
            .read_tree(&stash.tree().map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        if stash.parent_count() == 3 {
            let tree = stash
                .parent(2)
                .and_then(|c| c.tree())
                .map_err(|e| e.to_string())?;
            let mut extra = git2::Index::new().map_err(|e| e.to_string())?;
            extra.read_tree(&tree).map_err(|e| e.to_string())?;
            for entry in extra.iter() {
                if combined
                    .get_path(
                        Path::new(
                            std::str::from_utf8(&entry.path)
                                .map_err(|_| "Saved work requires UTF-8 paths.")?,
                        ),
                        0,
                    )
                    .is_some()
                {
                    return Err("Saved tracked and untracked paths overlap.".into());
                }
                combined.add(&entry).map_err(|e| e.to_string())?;
            }
        }
        if combined.write_tree_to(repo).map_err(|e| e.to_string())?
            != oid(&self.preview.source.files.worktree)?
        {
            return Err("Saved files differ from the reviewed work. Checkout was not started; inspect the retained stash.".into());
        }
        Ok(())
    }

    pub fn prepare_restore(&self, path: &Path, epoch: u64) -> Result<RestorePreview, String> {
        if !self.phase.can_restore() {
            return Err("This saved-work operation needs inspection; it cannot be automatically applied again.".into());
        }
        let repo = self.validate_record(path)?;
        let saved = oid(self
            .stash_oid
            .as_deref()
            .ok_or("No verified saved-work object is recorded. Inspect stashes manually.")?)?;
        if repo.refname_to_id(&self.pin()).ok() != Some(saved) {
            return Err(
                "Saved-work recovery pin changed. Inspect stashes before restoring.".into(),
            );
        }
        let bytes = read_record_bytes(&repo, &self.id)?.ok_or("Saved-work journal disappeared.")?;
        if bytes != self.bytes()? {
            return Err("Saved-work journal changed. Refresh before restoring.".into());
        }
        let (tracked, untracked, ignored) = counts(&repo)?;
        Ok(RestorePreview {
            record: self.clone(),
            bytes,
            current: witness(path, &repo)?,
            replacements: replacements(&repo)?,
            epoch,
            tracked,
            untracked,
            ignored,
        })
    }
}

impl RestorePreview {
    pub fn execute(&self, session: impl Fn() -> Result<(), String>) -> Result<(), String> {
        session()?;
        let path = &self.record.preview.path;
        let repo = self.record.validate_record(path)?;
        if witness(path, &repo)? != self.current || replacements(&repo)? != self.replacements {
            return Err("Branch, index or files changed. Review the restore again.".into());
        }
        let fresh = self.record.prepare_restore(path, self.epoch)?;
        if fresh.bytes != self.bytes
            || fresh.current != self.current
            || fresh.replacements != self.replacements
        {
            return Err("Saved work changed. Review the restore again.".into());
        }
        session()?;
        let mut record = self.record.clone();
        record.phase = Phase::RestoreSubmitted;
        record.restore_before = Some(self.current.clone());
        record.write(Some(&self.bytes))?;
        let bytes = record.bytes()?;
        fault("restore-submitted")?;
        session()?;
        let saved = record
            .stash_oid
            .as_deref()
            .ok_or("Missing saved-work identity.")?;
        let result = super::process::git(
            path,
            &["stash", "apply", "--index", saved],
            &AtomicBool::new(false),
        )
        .map(|_| ())
        .map_err(|e| e.user_message());
        fault("restore-applied")?;
        record.phase = if result.is_ok() {
            Phase::Restored
        } else {
            Phase::RestoreFailed
        };
        record.outcome = result.as_ref().err().map(|e| crate::auth::redact::text(e));
        record.write(Some(&bytes)).map_err(|e|format!("Restore may have changed files; its outcome could not be recorded. Inspect before any further apply: {e}"))?;
        result.map_err(|e|format!("{e}\nSaved work is retained. Inspect staged files and conflicts; this restore will not be repeated automatically."))
    }
}

fn read_record_bytes(repo: &git2::Repository, id: &str) -> Result<Option<Vec<u8>>, String> {
    if !valid_id(id) {
        return Err("Invalid saved-work journal name.".into());
    }
    super::safe_path::ScopedFile::new(repo.path(), &format!("recorde/switch/{id}.json"))?
        .read(RECORD_LIMIT)
}

pub fn load(path: &Path) -> (Vec<SavedWork>, Vec<String>) {
    let result = (|| -> Result<(Vec<SavedWork>, Vec<String>), String> {
        let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
        // Avoid creating metadata merely because a repository was opened.
        match std::fs::symlink_metadata(repo.path().join("recorde")) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok((Vec::new(), Vec::new()))
            }
            Err(e) => return Err(e.to_string()),
            Ok(_) => {}
        }
        let directory = journal_dir(repo.path())?;
        let mut records = Vec::new();
        let mut warnings = Vec::new();
        for (count, entry) in directory.entries().map_err(|e| e.to_string())?.enumerate() {
            if count >= RECORD_COUNT {
                warnings.push("Saved-work journal count exceeds the inspection limit; older records need manual inspection.".into());
                break;
            }
            let result = (|| -> Result<SavedWork, String> {
                let entry = entry.map_err(|e| e.to_string())?;
                let name = entry
                    .file_name()
                    .to_str()
                    .ok_or("Invalid saved-work filename.")?
                    .to_string();
                let id = name
                    .strip_suffix(".json")
                    .ok_or("Unexpected saved-work metadata file; retained for inspection.")?;
                let bytes =
                    read_record_bytes(&repo, id)?.ok_or("Saved-work journal disappeared.")?;
                let record: SavedWork = serde_json::from_slice(&bytes)
                    .map_err(|_| "Corrupt saved-work journal; retained for inspection.")?;
                if record.id != id {
                    return Err("Saved-work filename and record identity disagree.".into());
                }
                record.validate_record(path)?;
                Ok(record)
            })();
            match result {
                Ok(record) => records.push(record),
                Err(error) => warnings.push(crate::auth::redact::text(&error)),
            }
        }
        records.sort_by(|a, b| b.id.cmp(&a.id));
        Ok((records, warnings))
    })();
    result.unwrap_or_else(|e| (Vec::new(), vec![crate::auth::redact::text(&e)]))
}

fn counts(repo: &git2::Repository) -> Result<(usize, usize, usize), String> {
    let mut options = git2::StatusOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(true)
        .recurse_ignored_dirs(true);
    let (mut tracked, mut untracked, mut ignored) = (0, 0, 0);
    for entry in repo
        .statuses(Some(&mut options))
        .map_err(|e| e.to_string())?
        .iter()
    {
        let status = entry.status();
        if status.contains(git2::Status::IGNORED) {
            ignored += 1;
        } else if status.contains(git2::Status::WT_NEW) {
            untracked += 1;
        } else if status != git2::Status::CURRENT {
            tracked += 1;
        }
    }
    Ok((tracked, untracked, ignored))
}

fn session(state: &crate::state::SharedState, path: &Path, epoch: u64) -> Result<(), String> {
    let state = state.lock().unwrap();
    if state.repo_epoch != epoch || state.repo_path.as_deref() != Some(path) {
        return Err("Repository changed while the operation was queued. Review it again.".into());
    }
    Ok(())
}

pub fn request(target: Target, state: crate::state::SharedState) {
    let (path, epoch) = {
        let s = state.lock().unwrap();
        let Some(path) = s.repo_path.clone() else {
            return;
        };
        (path, s.repo_epoch)
    };
    request_for(&path, epoch, target, state);
}

/// A delayed producer must retain the repository/session which initiated it.
/// Stale completions are ignored, including navigation away and back again.
pub fn request_for(path: &Path, epoch: u64, target: Target, state: crate::state::SharedState) {
    let (path, epoch, id) = {
        let mut s = state.lock().unwrap();
        if s.repo_path.as_deref() != Some(path) || s.repo_epoch != epoch {
            return;
        }
        s.switch_request_id = s.switch_request_id.wrapping_add(1);
        s.pending_switch = None;
        (path.to_path_buf(), epoch, s.switch_request_id)
    };
    let work_path = path.clone();
    let guard = state.clone();
    super::mutation::spawn_job(
        path.clone(),
        "Review branch switch",
        state,
        move || {
            session(&guard, &work_path, epoch)?;
            Preview::capture(&work_path, epoch, target)
        },
        move |result, state| {
            let mut s = state.lock().unwrap();
            if s.repo_path.as_ref() != Some(&path)
                || s.repo_epoch != epoch
                || s.switch_request_id != id
            {
                return;
            }
            match result {
                Ok(preview) => s.pending_switch = Some(preview),
                Err(error) => s.error = Some(crate::auth::redact::text(&error)),
            }
        },
    );
}

pub fn submit(preview: Preview, choice: Choice, state: crate::state::SharedState) {
    state.lock().unwrap().remote_op = Some(
        match choice {
            Choice::Bring => "Switch branch",
            Choice::Stash => "Save work and switch branch",
        }
        .into(),
    );
    let path = preview.path.clone();
    let guard = state.clone();
    super::mutation::spawn_operation(path, state, move |_| {
        let result = preview.execute(choice, || session(&guard, &preview.path, preview.epoch));
        if matches!(choice, Choice::Stash) {
            guard.lock().unwrap().show_saved_work = true;
        }
        result
    });
}

pub fn request_restore(record: SavedWork, state: crate::state::SharedState) {
    let (path, epoch) = {
        let s = state.lock().unwrap();
        let Some(path) = s.repo_path.clone() else {
            return;
        };
        (path, s.repo_epoch)
    };
    let work_path = path.clone();
    let guard = state.clone();
    super::mutation::spawn_job(
        path.clone(),
        "Review saved-work restore",
        state,
        move || {
            session(&guard, &work_path, epoch)?;
            record.prepare_restore(&work_path, epoch)
        },
        move |result, state| {
            let mut s = state.lock().unwrap();
            if s.repo_path.as_ref() != Some(&path) || s.repo_epoch != epoch {
                return;
            }
            match result {
                Ok(preview) => s.pending_restore = Some(preview),
                Err(error) => s.error = Some(crate::auth::redact::text(&error)),
            }
        },
    );
}

pub fn submit_restore(preview: RestorePreview, state: crate::state::SharedState) {
    state.lock().unwrap().remote_op = Some("Restore saved work".into());
    let path = preview.record.preview.path.clone();
    let guard = state.clone();
    super::mutation::spawn_operation(path, state, move |_| {
        preview.execute(|| session(&guard, &preview.record.preview.path, preview.epoch))
    });
}

impl SavedWork {
    pub fn can_restore(&self) -> bool {
        self.phase.can_restore() && self.stash_oid.is_some()
    }

    /// Repair only the journal/pin after an interrupted stash creation.
    /// No stash creation, checkout or application is repeated.
    pub fn reconcile(&self, path: &Path) -> Result<(), String> {
        if self.phase != Phase::StashSubmitted || self.stash_oid.is_some() {
            return Err("This record does not need stash-creation reconciliation.".into());
        }
        let repo = self.validate_record(path)?;
        let bytes = read_record_bytes(&repo, &self.id)?.ok_or("Saved-work journal disappeared.")?;
        if bytes != self.bytes()? {
            return Err("Saved-work journal changed. Refresh before reconciling.".into());
        }
        let mut candidates = std::collections::BTreeSet::new();
        if let Ok(pin) = repo.refname_to_id(&self.pin()) {
            candidates.insert(pin);
        }
        if let Ok(log) = repo.reflog("refs/stash") {
            for entry in log.iter().take(RECORD_COUNT) {
                if entry
                    .message()
                    .ok()
                    .flatten()
                    .is_some_and(|m| m.ends_with(&self.marker()))
                {
                    candidates.insert(entry.id_new());
                }
            }
        }
        let candidates: Vec<_> = candidates
            .into_iter()
            .filter(|candidate| {
                self.previous_stash.as_deref() != Some(&candidate.to_string())
                    && self.verify_stash(&repo, *candidate).is_ok()
            })
            .collect();
        if candidates.len() != 1 {
            return Err("No unique stash matches the original HEAD, index, files and operation marker. Inspect retained stashes and recovery snapshot; no work was applied.".into());
        }
        let saved = candidates[0];
        match repo.find_reference(&self.pin()) {
            Ok(pin) if pin.target() == Some(saved) => {}
            Ok(_) => return Err("Saved-work pin changed. It was not overwritten.".into()),
            Err(e) if e.code() == git2::ErrorCode::NotFound => {
                repo.reference(&self.pin(), saved, false, &self.marker())
                    .map_err(|e| e.to_string())?;
            }
            Err(e) => return Err(e.to_string()),
        }
        let mut record = self.clone();
        record.stash_oid = Some(saved.to_string());
        record.phase = Phase::Stashed;
        record.outcome=Some("Saved-work record recovered. Checkout was not repeated; review the current branch before restoring.".into());
        record.write(Some(&bytes))
    }
}

pub fn reconcile(record: SavedWork, state: crate::state::SharedState) {
    let (path, epoch) = {
        let s = state.lock().unwrap();
        let Some(path) = s.repo_path.clone() else {
            return;
        };
        (path, s.repo_epoch)
    };
    let guard = state.clone();
    super::mutation::spawn_operation(path, state, move |path| {
        session(&guard, path, epoch)?;
        record.reconcile(path)
    });
}
impl RestorePreview {
    pub fn destination_label(&self) -> &str {
        self.current
            .head_ref
            .as_deref()
            .unwrap_or(&self.current.head_oid)
    }
}

fn fault(_phase: &str) -> Result<(), String> {
    #[cfg(test)]
    {
        if std::env::var("RECORDE_SWITCH_EXIT_AT").ok().as_deref() == Some(_phase) {
            std::process::exit(55);
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "switch_tests.rs"]
mod tests;
