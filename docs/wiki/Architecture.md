# Architecture

Recorde is a native egui application with a small set of clear layers:

- `main.rs` configures the native window and starts eframe using the shared library.
  Application modules are compiled once, in `lib.rs`; the executable does not
  redeclare them or duplicate their unit tests.
- `RecordeApp` in `app.rs` owns UI components, settings, modal inputs, and
  command dispatch.
- `AppState` in `state.rs` is shared between UI code and background work through
  `Arc<Mutex<AppState>>`.
- `git/` modules perform repository operations using `git2` and selected Git CLI
  commands where appropriate.
- `ui/` modules render focused panels and return user actions to `RecordeApp`.
- `providers/` modules translate GitHub, GitLab, Bitbucket Cloud and Azure DevOps API responses into provider-neutral
  structs.
- `auth/` and `config.rs` persist local settings and provider credentials.

## Application Startup

Startup happens in `src/main.rs`:

1. Install the crash handler.
2. Initialize logging through `env_logger`.
3. Read an optional repository path from the first CLI argument.
4. Load the PNG app icon from `assets/icon.png`.
5. Configure an eframe native viewport with title, app id, size, and minimum size.
6. Construct `RecordeApp`.

`RecordeApp::new` then loads configuration, applies theme settings, registers
fonts, creates shared state and UI panels, and opens either the CLI-provided repo
or the saved active repository, restoring its workspace tabs.

## Shared State Model

`AppState` is the single shared model for repository data and transient UI state.
It contains:

- Current repository path.
- Commit graph and selected commit/file/diff.
- Working tree files, stashes, conflicts, submodules, and worktrees.
- Branch, ahead/behind, dirty count, and capability data.
- Active Git jobs and console log lines.
- Modal state and destructive-action confirmation state.
- Provider lists for PRs/MRs, issues, CI runs, and releases.
- History/review data such as reflog, blame, compare, and bisect state.

UI components clone the `SharedState` handle and lock it briefly when rendering or
reacting to events.

## UI Composition

The main UI is composed from reusable structs in `src/ui/`:

- `Toolbar` emits high-level repository and remote actions.
- `Sidebar` emits branch, tag, stash, conflict, submodule, and worktree actions.
- `CommitGraphView` renders history and commit context actions.
- `DetailPanel` shows selected commit metadata and changed files.
- `DiffView` renders current diff hunks and line-level state.
- `StagingPanel` handles staged and unstaged changes plus commit controls.
- `StatusBar` shows repository status and operation progress.
- `RebaseModal` edits an interactive rebase todo list.

`RecordeApp` handles actions returned by these components. This keeps individual
widgets focused on presentation while the app object coordinates side effects.

## Git Operation Flow

Operation workers use the following paths:

- Read workers return repository-epoch and request-generation scoped data.
  Stale results are rejected, including provider responses after account/remote changes.
- Local mutations reserve FIFO jobs shared by the canonical common Git directory,
  so linked worktrees serialize writes. Same-user OS locks coordinate Recorde
  processes. Native Git commands use supervised process groups/jobs with timeout,
  cancellation and concurrent bounded output draining.
- Remote jobs submitted to `GitRunner`, which owns a worker thread and returns
  progress events through an mpsc channel.

`RecordeApp` drains runner progress once per frame, updates `active_ops`, refreshes
repository state after terminal operations, and renders progress to the UI.

## Module Boundaries

| Module | Responsibility |
| --- | --- |
| `git/operations.rs` | Repository loading, status collection, diffs, commit details, provider load orchestration. |
| `git/runner.rs` and `git/job.rs` | Background remote operation queue and progress events. |
| `git/remote.rs` | Fetch, pull, push, upstream, and ahead/behind logic. |
| `git/branches.rs` | Branch, tag, stash, cherry-pick, and revert operations. |
| `git/staging.rs` | Index, commit, amend, reset, hunk, line, and discard workflows. |
| `git/merge.rs`, `git/rebase.rs`, `git/conflicts.rs` | Conflict and sequence-operation handling. |
| `git/graph.rs`, `git/history.rs`, `git/blame.rs`, `git/reflog.rs` | Review and history data. |
| `git/capabilities.rs` | Repository state detection. |
| `git/lifecycle.rs` | Clone, init, terminal, and file-manager helpers. |
| `providers/github.rs`, `providers/gitlab.rs`, `providers/bitbucket.rs`, `providers/azure.rs` | Provider API adapters with explicit action and scope limits. |

## Error Handling

Git-facing modules usually return `git2::Error`, `GitError`, or `Result<_, String>`.
User-visible failures are written to `AppState.error` and often appended to
`console_log`. Remote jobs report `Started`, `Update`, `Done`, or `Failed`
events.

Destructive actions are routed through confirmation state before execution where
the UI can prevent accidental data loss.


## Recovery and credentials

Recovery checkpoints preserve independent index/worktree trees, non-ignored
untracked files, HEAD and branch identity using protected refs and atomic journals.
Restoration checks the captured post-operation state and saves an inverse snapshot.
Sequencer continuation matches native control-file witnesses; mismatches leave
manual recovery available. Sparse checkout must be disabled before restoration.

Credentials use OS vaults with exact endpoint/account bindings. Provider reads
and writes reject redirects, cap responses and redact errors. Confirmed review
mutations bind to repository, endpoint, account and expected head. Drafts persist
without restoring confirmations or submitting work automatically.

## Diffs, sessions and paging

Unified text diffs support click, Ctrl/Cmd toggle and Shift range selection.
`multi_selection_patch` emits one file header and recalculates coordinates across
selected hunks. One `git apply --cached` call changes the index atomically; a
captured diff must still match before applying. Identical background refreshes
preserve selections, including their loading interval; repository epochs and
content/context changes invalidate them. Gutter clicks retain single-line actions.

Conflict side selection captures stage IDs/modes before queueing. Missing chosen
stages resolve to deletion; every conflict path is passed literally to Git.

### Recovery, sessions and paging

Recovery snapshots use an alternate native Git index to capture separate index
and non-ignored worktree trees, including unborn repositories. Protected refs keep
objects alive; exact completed-state checks guard undo and each undo saves an
inverse. Checkout recovery restores the original HEAD binding without rewriting
unrelated branch refs. Sequencer witnesses include HEAD and bounded control-file
hashes, allowing restart/continue recovery while refusing unrelated operations.

Disk navigation stores identifiers and preferences only; data is freshly loaded.
Repository-owned forms remain scoped in memory. Full and status-only capability
refreshes share provider invalidation. History snapshots share a cursor whose
worker owns its libgit2 repository and revwalk; subsequent pages reuse traversal
and commit metadata. Ref changes and shallow histories start a fresh walker.
Loaded geometry is rebuilt to connect page boundaries. The worker exits when its
last snapshot is released.

Batch staging uses NUL-delimited literal pathspec files and one native index
operation. Selected-range discard holds the confirmed diff and validates raw
patch bytes inside the mutation queue. Ignore-file edits reject symlinks, escape
literal filenames and replace the file atomically.

Repositories with Git commit-graph indexes use bounded native `rev-list` pages
with captured OID roots, original-object semantics and cached loaded IDs/metadata.
Each response is at most 4,096 commits. Revision roots go through a temporary
stdin file to avoid Windows command-line length limits. Indexed traversals do not
keep a subprocess open between requests. Unindexed histories keep the existing
libgit2 revwalk. Backend changes invalidate the cursor; refs/HEAD changes still
invalidate all cached pages.
