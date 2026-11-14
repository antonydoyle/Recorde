//! Conservative namespace closure for native history operations which can
//! overwrite ignored files even with --no-overwrite-ignore. No ignored bytes
//! are read or saved. Hooks, filters and custom drivers remain trusted code.
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[cfg(test)]
#[path = "ignored_tests.rs"]
mod tests;

const MAX_OBJECTS: usize = 100_000;
const MAX_ENTRIES: usize = 1_000_000;
const MAX_NAME_BYTES: usize = 32 * 1024 * 1024;
const MAX_SECONDS: u64 = 10;
const FALLBACK: &str = "Cannot prove this history operation will preserve ignored files. Protect those files and use Git manually, or move them outside this repository before retrying.";

struct Budget {
    started: Instant,
    objects: usize,
    entries: usize,
    bytes: usize,
}
impl Budget {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            objects: 0,
            entries: 0,
            bytes: 0,
        }
    }
    fn check(&self) -> Result<(), String> {
        super::process::check_local_cancellation()?;
        if self.objects > MAX_OBJECTS
            || self.entries > MAX_ENTRIES
            || self.bytes > MAX_NAME_BYTES
            || self.started.elapsed() > Duration::from_secs(MAX_SECONDS)
        {
            return Err(format!(
                "Ignored-file safety scan exceeded its bounded history budget. {FALLBACK}"
            ));
        }
        Ok(())
    }
    fn name(&mut self, names: &mut BTreeSet<String>, bytes: &[u8]) -> Result<(), String> {
        self.entries += 1;
        let name = std::str::from_utf8(bytes)
            .map_err(|_| format!("Non-UTF-8 history paths cannot be checked safely. {FALLBACK}"))?;
        #[cfg(windows)]
        if name.contains(['/', '\\']) {
            return Err(format!(
                "Separator-containing history component cannot be checked safely. {FALLBACK}"
            ));
        }
        #[cfg(windows)]
        super::safe_path::relative(name).map_err(|_| {
            format!("Ambiguous Windows history path cannot be checked safely. {FALLBACK}")
        })?;
        if names.insert(name.to_owned()) {
            self.bytes += bytes.len();
        }
        self.check()
    }
}

#[derive(PartialEq, Eq)]
struct Witness {
    head_ref: Option<String>,
    head: Option<git2::Oid>,
    inputs: Vec<git2::Oid>,
    replacements: BTreeMap<git2::Oid, git2::Oid>,
    config: git2::Oid,
    grafts: Option<Vec<u8>>,
}
fn grafts(repo: &git2::Repository) -> Result<Option<Vec<u8>>, String> {
    match std::fs::symlink_metadata(repo.commondir().join("info")) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
        Ok(_) => super::safe_path::ScopedFile::new(repo.commondir(), "info/grafts")?.read(65536),
    }
}
fn witness(repo: &git2::Repository, inputs: &[String]) -> Result<Witness, String> {
    let head = repo.find_reference("HEAD").map_err(|e| e.to_string())?;
    let config = repo.config().map_err(|e| e.to_string())?;
    let mut entries = config.entries(None).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    while let Some(entry) = entries.next() {
        let entry = entry.map_err(|e| e.to_string())?;
        bytes.extend_from_slice(entry.name_bytes());
        bytes.push(0);
        bytes.extend_from_slice(entry.value_bytes());
        bytes.push(0);
        if bytes.len() > 1024 * 1024 {
            return Err(format!(
                "Configuration exceeds the ignored-file safety limit. {FALLBACK}"
            ));
        }
    }
    Ok(Witness {
        head_ref: head
            .symbolic_target()
            .map_err(|e| e.to_string())?
            .map(str::to_owned),
        head: head.resolve().ok().and_then(|r| r.target()),
        inputs: inputs
            .iter()
            .map(|input| {
                repo.revparse_single(&format!("{input}^{{commit}}"))
                    .map(|c| c.id())
                    .map_err(|e| e.to_string())
            })
            .collect::<Result<_, _>>()?,
        replacements: super::replacement::refs(repo).map_err(|e| e.to_string())?,
        config: git2::Oid::hash_object(git2::ObjectType::Blob, &bytes)
            .map_err(|e| e.to_string())?,
        grafts: grafts(repo)?,
    })
}

fn ignored(path: &Path) -> Result<Vec<String>, String> {
    let mut command = super::process::command(path);
    command.args([
        "ls-files",
        "--others",
        "--ignored",
        "--exclude-standard",
        "--directory",
        "-z",
    ]);
    let output = super::process::run(
        &mut command,
        &std::sync::atomic::AtomicBool::new(false),
        Duration::from_secs(MAX_SECONDS),
    )
    .map_err(|e| e.user_message())?;
    if !output.status.success() {
        return Err(format!(
            "Cannot enumerate ignored worktree paths safely. {FALLBACK}"
        ));
    }
    if !output.stdout.is_empty() && output.stdout.last() != Some(&0) {
        return Err(format!("Incomplete ignored-path listing. {FALLBACK}"));
    }
    let mut roots = BTreeSet::new();
    for bytes in output.stdout.split(|b| *b == 0).filter(|b| !b.is_empty()) {
        let file = std::str::from_utf8(bytes)
            .map_err(|_| format!("Non-UTF-8 ignored paths cannot be checked safely. {FALLBACK}"))?;
        let root = file
            .trim_end_matches('/')
            .split('/')
            .next()
            .ok_or(FALLBACK)?;
        super::safe_path::relative(root)?;
        roots.insert(root.to_owned());
    }
    Ok(roots.into_iter().collect())
}

fn names(
    repo: &git2::Repository,
    state: &Witness,
    budget: &mut Budget,
) -> Result<BTreeSet<String>, String> {
    use git2::ObjectType::{Commit, Tree};
    let mut names = BTreeSet::new();
    let mut seen = HashSet::new();
    let mut active = HashSet::new();
    let mut pending: Vec<_> = state
        .inputs
        .iter()
        .copied()
        .chain(state.head)
        .map(|id| (id, Commit, false))
        .collect();
    while let Some((id, kind, exit)) = pending.pop() {
        if exit {
            active.remove(&(id, kind == Tree));
            continue;
        }
        if active.contains(&(id, kind == Tree)) {
            return Err(format!(
                "Cyclic raw/replacement history cannot be checked safely. {FALLBACK}"
            ));
        }
        if !seen.insert((id, kind == Tree)) {
            continue;
        }
        active.insert((id, kind == Tree));
        pending.push((id, kind, true));
        budget.objects += 1;
        budget.check()?;
        let effective =
            super::replacement::resolve(id, &state.replacements).map_err(|e| e.to_string())?;
        if effective != id {
            pending.push((effective, kind, false));
        }
        match kind {
            Commit => {
                let commit = repo
                    .find_commit(id)
                    .map_err(|e| format!("Cannot inspect complete history: {e}. {FALLBACK}"))?;
                pending.push((commit.tree_id(), Tree, false));
                pending.extend(commit.parent_ids().map(|id| (id, Commit, false)));
            }
            Tree => {
                let tree = repo.find_tree(id).map_err(|e| {
                    format!("Cannot inspect complete history tree: {e}. {FALLBACK}")
                })?;
                for entry in tree.iter() {
                    budget.name(&mut names, entry.name_bytes())?;
                    if entry.kind() == Some(Tree) {
                        pending.push((entry.id(), Tree, false));
                    }
                }
            }
            _ => unreachable!(),
        }
    }
    index_names(repo, &mut names, budget)?;
    Ok(names)
}
fn index_names(
    repo: &git2::Repository,
    names: &mut BTreeSet<String>,
    budget: &mut Budget,
) -> Result<(), String> {
    for entry in repo.index().map_err(|e| e.to_string())?.iter() {
        for name in entry.path.split(|b| *b == b'/') {
            budget.name(names, name)?;
        }
    }
    Ok(())
}

#[cfg(windows)]
fn same_name(a: &str, b: &str) -> Result<bool, String> {
    use windows_sys::Win32::Globalization::{CompareStringOrdinal, CSTR_EQUAL};
    let a: Vec<_> = a.encode_utf16().collect();
    let b: Vec<_> = b.encode_utf16().collect();
    let result =
        unsafe { CompareStringOrdinal(a.as_ptr(), a.len() as i32, b.as_ptr(), b.len() as i32, 1) };
    if result == 0 {
        Err(format!("Cannot compare Windows path aliases. {FALLBACK}"))
    } else {
        Ok(result == CSTR_EQUAL)
    }
}
#[cfg(not(windows))]
fn same_name(a: &str, b: &str) -> Result<bool, String> {
    if a.to_lowercase() == b.to_lowercase() || a.to_uppercase() == b.to_uppercase() {
        return Ok(true);
    }
    if !a.is_ascii() && !b.is_ascii() {
        return Err(format!(
            "Unicode filesystem aliases cannot be checked safely on this platform. {FALLBACK}"
        ));
    }
    Ok(false)
}
fn aliases(root: &Path, name: &str) -> Result<Vec<String>, String> {
    let result = vec![name.to_owned()];
    #[cfg(windows)]
    {
        use std::os::windows::ffi::{OsStrExt, OsStringExt};
        use windows_sys::Win32::Storage::FileSystem::GetShortPathNameW;
        let path = root.join(name);
        let wide: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut output = vec![0u16; 32768];
        let size =
            unsafe { GetShortPathNameW(wide.as_ptr(), output.as_mut_ptr(), output.len() as u32) }
                as usize;
        if size == 0 || size >= output.len() {
            return Err(format!(
                "Cannot establish Windows ignored-path aliases for {name}. {FALLBACK}"
            ));
        }
        let short = PathBuf::from(std::ffi::OsString::from_wide(&output[..size]));
        let short = short
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or(FALLBACK)?
            .to_owned();
        let mut result = result;
        result.push(short);
        Ok(result)
    }
    #[cfg(not(windows))]
    {
        let _ = root;
        Ok(result)
    }
}
fn check_names(
    path: &Path,
    roots: &[String],
    names: &BTreeSet<String>,
    budget: &Budget,
) -> Result<(), String> {
    for root in roots {
        for alias in aliases(path, root)? {
            // Git's conflict unique_path appends '~<label>' and optional numbers.
            let variants: Vec<_> = std::iter::once(alias.as_str())
                .chain(alias.match_indices('~').map(|(i, _)| &alias[..i]))
                .collect();
            for name in names {
                budget.check()?;
                let mut overlaps = false;
                for variant in &variants {
                    overlaps |= same_name(variant, name)?;
                }
                if overlaps {
                    return Err(format!("Recorde cannot prove this operation will preserve ignored files under {root}. Move those files outside this repository and retry, or use Git manually after protecting them. This safety check has not changed your files."));
                }
            }
        }
    }
    Ok(())
}

fn check_options(repo: &git2::Repository, inputs: &[String]) -> Result<(), String> {
    if grafts(repo)?.as_ref().is_some_and(|b| !b.is_empty()) {
        return Err(format!(
            "Legacy graft history cannot be checked safely with ignored files present. {FALLBACK}"
        ));
    }
    let config = repo.config().map_err(|e| e.to_string())?;
    for key in ["pull.twohead", "pull.octopus"] {
        if config.get_string(key).is_ok_and(|s| {
            s.split_whitespace()
                .any(|s| !matches!(s, "ort" | "recursive"))
        }) {
            return Err(format!("Custom merge strategies cannot be checked safely with ignored files present. {FALLBACK}"));
        }
    }
    let branch = repo
        .head()
        .ok()
        .and_then(|r| r.shorthand().ok().map(str::to_owned));
    for branch in branch.iter().chain(inputs.iter()) {
        if config
            .get_string(&format!("branch.{branch}.mergeOptions"))
            .is_ok_and(|v| !v.trim().is_empty())
        {
            return Err(format!("Custom branch merge options cannot be checked safely with ignored files present. {FALLBACK}"));
        }
    }
    Ok(())
}

pub(crate) struct Guard {
    path: PathBuf,
    inputs: Vec<String>,
    witness: Witness,
    names: Option<BTreeSet<String>>,
}
impl Guard {
    pub(crate) fn prepare(path: &Path, inputs: &[&str]) -> Result<Self, String> {
        let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
        let inputs: Vec<_> = inputs.iter().map(|s| (*s).to_owned()).collect();
        let state = witness(&repo, &inputs)?;
        let roots = ignored(path)?;
        let mut budget = Budget::new();
        let names = if roots.is_empty() {
            None
        } else {
            // A branch-specific custom strategy can escape ordinary Git's
            // namespace contract. External drivers/hooks remain trusted code.
            check_options(&repo, &inputs)?;
            let names = names(&repo, &state, &mut budget)?;
            check_names(path, &roots, &names, &budget)?;
            Some(names)
        };
        if witness(&repo, &inputs)? != state {
            return Err("Repository history or configuration changed during ignored-file safety scan. Retry the operation.".into());
        }
        Ok(Self {
            path: path.to_owned(),
            inputs,
            witness: state,
            names,
        })
    }
    pub(crate) fn operand(&self, index: usize) -> String {
        self.witness.inputs[index].to_string()
    }
    pub(crate) fn validate(&self) -> Result<(), String> {
        let repo = git2::Repository::open(&self.path).map_err(|e| e.to_string())?;
        if witness(&repo, &self.inputs)? != self.witness {
            return Err(
                "Repository history or configuration changed after review. Retry the operation."
                    .into(),
            );
        }
        let roots = ignored(&self.path)?;
        if roots.is_empty() {
            return Ok(());
        }
        check_options(&repo, &self.inputs)?;
        let mut budget = Budget::new();
        let mut names = match &self.names {
            Some(names) => names.clone(),
            None => names(&repo, &self.witness, &mut budget)?,
        };
        index_names(&repo, &mut names, &mut budget)?;
        check_names(&self.path, &roots, &names, &budget)?;
        if witness(&repo, &self.inputs)? != self.witness {
            return Err(
                "Repository changed during the ignored-file safety recheck. Retry the operation."
                    .into(),
            );
        }
        Ok(())
    }
}

/// Freeze native sequencer inputs, then recheck after recovery bookkeeping and
/// immediately before Continue/Skip/Abort enters native Git.
pub(crate) struct SequenceGuard {
    path: PathBuf,
    metadata: BTreeMap<String, Option<Vec<u8>>>,
    labels: BTreeMap<String, git2::Oid>,
    guard: Guard,
}
fn metadata(repo: &git2::Repository) -> Result<BTreeMap<String, Option<Vec<u8>>>, String> {
    let mut result = BTreeMap::new();
    let mut total = 0;
    for name in [
        "ORIG_HEAD",
        "MERGE_HEAD",
        "REBASE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
    ] {
        let bytes = super::safe_path::ScopedFile::new(repo.path(), name)?.read(65536)?;
        total += bytes.as_ref().map_or(0, Vec::len);
        result.insert(name.into(), bytes);
    }
    for directory in ["rebase-merge", "rebase-apply", "sequencer"] {
        let exists = match std::fs::symlink_metadata(repo.path().join(directory)) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => return Err(e.to_string()),
            Ok(m) if !m.is_dir() || m.file_type().is_symlink() => {
                return Err(format!(
                    "Cannot inspect redirected sequencer state. {FALLBACK}"
                ))
            }
            Ok(_) => true,
        };
        result.insert(
            format!("{directory}/"),
            if exists { Some(Vec::new()) } else { None },
        );
        for name in [
            "orig-head",
            "onto",
            "head-name",
            "stopped-sha",
            "strategy",
            "strategy_opts",
            "git-rebase-todo",
            "done",
            "todo",
        ] {
            let name = format!("{directory}/{name}");
            let bytes = if exists {
                super::safe_path::ScopedFile::new(repo.path(), &name)?.read(256 * 1024)?
            } else {
                None
            };
            total += bytes.as_ref().map_or(0, Vec::len);
            result.insert(name, bytes);
            if total > 1024 * 1024 {
                return Err(format!(
                    "Sequencer metadata exceeds safety limits. {FALLBACK}"
                ));
            }
        }
    }
    Ok(result)
}
fn rewritten(repo: &git2::Repository) -> Result<BTreeMap<String, git2::Oid>, String> {
    let mut result = BTreeMap::new();
    let mut bytes = 0;
    for reference in repo
        .references_glob("refs/rewritten/*")
        .map_err(|e| e.to_string())?
    {
        let reference = reference.map_err(|e| e.to_string())?;
        let name = reference.name().map_err(|e| e.to_string())?;
        bytes += name.len();
        if result.len() >= 256 || bytes > 65536 {
            return Err(format!(
                "Too many rewritten labels to check safely. {FALLBACK}"
            ));
        }
        result.insert(
            name.into(),
            reference.peel_to_commit().map_err(|e| e.to_string())?.id(),
        );
    }
    Ok(result)
}
fn commit_operand(
    repo: &git2::Repository,
    token: &str,
    inputs: &mut Vec<String>,
) -> Result<(), String> {
    repo.revparse_single(&format!("{token}^{{commit}}"))
        .map_err(|_| format!("Cannot resolve sequencer commit/ref {token}. {FALLBACK}"))?;
    inputs.push(token.to_owned());
    Ok(())
}
fn label_operand(
    repo: &git2::Repository,
    token: &str,
    declared: &BTreeSet<String>,
    inputs: &mut Vec<String>,
) -> Result<(), String> {
    let reference = format!("refs/rewritten/{token}");
    if repo.find_reference(&reference).is_ok() {
        return commit_operand(repo, &reference, inputs);
    }
    if repo.revparse_single(&format!("{token}^{{commit}}")).is_ok() {
        return commit_operand(repo, token, inputs);
    }
    // A future label denotes the result of parsed replay commands. Their raw
    // inputs already contribute to the namespace closure, including renames.
    if declared.contains(token) {
        return Ok(());
    }
    commit_operand(repo, token, inputs)
}
fn sequence_options(metadata: &BTreeMap<String, Option<Vec<u8>>>) -> Result<(), String> {
    for (name, bytes) in metadata {
        let Some(bytes) = bytes else { continue };
        match name.rsplit('/').next() {
            Some("strategy") => {
                let value = std::str::from_utf8(bytes).map_err(|_| FALLBACK)?.trim();
                if !value.is_empty() && !matches!(value, "ort" | "recursive") {
                    return Err(format!("Saved rebase strategy cannot be checked safely with ignored files present. {FALLBACK}"));
                }
            }
            Some("strategy_opts") => {
                if bytes.iter().any(|b| !b.is_ascii_whitespace()) {
                    return Err(format!("Saved rebase strategy options cannot be checked safely with ignored files present. {FALLBACK}"));
                }
            }
            _ => {}
        }
    }
    Ok(())
}
impl SequenceGuard {
    pub(crate) fn prepare(path: &Path) -> Result<Self, String> {
        let repo = git2::Repository::open(path).map_err(|e| e.to_string())?;
        let metadata = metadata(&repo)?;
        if !ignored(path)?.is_empty() && metadata.get("rebase-apply/").is_some_and(Option::is_some)
        {
            return Err(format!("The apply-backend rebase state cannot be checked safely with ignored files present. {FALLBACK}"));
        }
        if !ignored(path)?.is_empty() {
            sequence_options(&metadata)?;
        }
        let labels = rewritten(&repo)?;
        let mut inputs = vec!["HEAD".to_owned()];
        inputs.extend(labels.keys().cloned());
        let mut declared = BTreeSet::new();
        for (name, bytes) in &metadata {
            let Some(bytes) = bytes else { continue };
            if !matches!(
                name.rsplit('/').next(),
                Some("git-rebase-todo" | "done" | "todo")
            ) {
                continue;
            }
            let text = std::str::from_utf8(bytes).map_err(|_| FALLBACK)?;
            for line in text.lines() {
                let fields: Vec<_> = line
                    .split_ascii_whitespace()
                    .take_while(|s| !s.starts_with('#'))
                    .collect();
                if matches!(fields.first().copied(), Some("label" | "l")) {
                    if fields.len() != 2
                        || !git2::Reference::is_valid_name(&format!("refs/rewritten/{}", fields[1]))
                    {
                        return Err(format!("Unsupported edited rebase label. {FALLBACK}"));
                    }
                    declared.insert(fields[1].to_owned());
                }
            }
        }
        for (name, bytes) in &metadata {
            let Some(bytes) = bytes else { continue };
            let text = std::str::from_utf8(bytes).map_err(|_| FALLBACK)?;
            let leaf = name.rsplit('/').next().unwrap_or(name);
            if matches!(leaf, "head-name" | "strategy" | "strategy_opts" | "") {
                continue;
            }
            if !matches!(leaf, "git-rebase-todo" | "done" | "todo") {
                for token in text.split_ascii_whitespace() {
                    if token.len() != 40 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
                        return Err(format!(
                            "Cannot verify edited sequencer anchors. {FALLBACK}"
                        ));
                    }
                    commit_operand(&repo, token, &mut inputs)?;
                }
                continue;
            }
            for line in text.lines() {
                let fields: Vec<_> = line
                    .split_ascii_whitespace()
                    .take_while(|s| !s.starts_with('#'))
                    .collect();
                let Some(command) = fields.first().copied() else {
                    continue;
                };
                let mut args = fields.iter().skip(1).copied();
                match command {
                    "pick" | "p" | "reword" | "r" | "edit" | "e" | "squash" | "s" | "drop"
                    | "d" => commit_operand(&repo, args.next().ok_or(FALLBACK)?, &mut inputs)?,
                    "fixup" | "f" => {
                        let mut token = args.next().ok_or(FALLBACK)?;
                        if matches!(token, "-C" | "-c") {
                            token = args.next().ok_or(FALLBACK)?;
                        }
                        commit_operand(&repo, token, &mut inputs)?;
                    }
                    "reset" | "t" => {
                        let token = args.next().ok_or(FALLBACK)?;
                        if args.next().is_some() {
                            return Err(FALLBACK.into());
                        }
                        label_operand(&repo, token, &declared, &mut inputs)?;
                    }
                    "merge" | "m" => {
                        let mut token = args.next().ok_or(FALLBACK)?;
                        if matches!(token, "-C" | "-c") {
                            commit_operand(&repo, args.next().ok_or(FALLBACK)?, &mut inputs)?;
                            token = args.next().ok_or(FALLBACK)?;
                        }
                        label_operand(&repo, token, &declared, &mut inputs)?;
                        for token in args {
                            label_operand(&repo, token, &declared, &mut inputs)?;
                        }
                    }
                    "label" | "l" => {}
                    "break" | "b" | "noop" => {
                        if args.next().is_some() {
                            return Err(FALLBACK.into());
                        }
                    }
                    // Native exec, like hooks/drivers, is an explicitly trusted
                    // program. It is not sandboxed by ignored-file protection.
                    "exec" | "x" => {}
                    _ => {
                        return Err(format!(
                            "Unsupported edited sequencer command {command}. {FALLBACK}"
                        ))
                    }
                }
            }
        }
        let refs: Vec<_> = inputs.iter().map(String::as_str).collect();
        let guard = Guard::prepare(path, &refs)?;
        let result = Self {
            path: path.to_owned(),
            metadata,
            labels,
            guard,
        };
        result.validate()?;
        Ok(result)
    }
    pub(crate) fn validate(&self) -> Result<(), String> {
        let repo = git2::Repository::open(&self.path).map_err(|e| e.to_string())?;
        if !ignored(&self.path)?.is_empty()
            && self
                .metadata
                .get("rebase-apply/")
                .is_some_and(Option::is_some)
        {
            return Err(format!("The apply-backend rebase state cannot be checked safely with ignored files present. {FALLBACK}"));
        }
        if metadata(&repo)? != self.metadata || rewritten(&repo)? != self.labels {
            return Err("Sequencer state changed during the ignored-file safety check. Refresh before continuing.".into());
        }
        if !ignored(&self.path)?.is_empty() {
            sequence_options(&self.metadata)?;
        }
        self.guard.validate()?;
        if metadata(&repo)? != self.metadata || rewritten(&repo)? != self.labels {
            return Err(
                "Sequencer state changed before native continuation. Refresh before continuing."
                    .into(),
            );
        }
        Ok(())
    }
}
