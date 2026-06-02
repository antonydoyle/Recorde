# Architecture

Recorde is a native egui application with a small set of clear layers:

- `main.rs` configures the native window and starts eframe.
- `RecordeApp` in `app.rs` owns UI components, settings, modal inputs, and
  command dispatch.
- `AppState` in `state.rs` is shared between UI code and background work through
  `Arc<Mutex<AppState>>`.
- `git/` modules perform repository operations using `git2` and selected Git CLI
  commands where appropriate.
- `ui/` modules render focused panels and return user actions to `RecordeApp`.
- `providers/` modules translate GitHub/GitLab API responses into provider-neutral
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
or the most recent saved repo.

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

There are two operation styles:

- Immediate helper calls that spawn a short-lived thread and update `AppState`
  when complete. Repository loading and many libgit2 queries use this pattern.
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
| `providers/github.rs`, `providers/gitlab.rs` | HTTP API calls for provider metadata. |

## Error Handling

Git-facing modules usually return `git2::Error`, `GitError`, or `Result<_, String>`.
User-visible failures are written to `AppState.error` and often appended to
`console_log`. Remote jobs report `Started`, `Progress`, `Done`, or `Failed`
events.

Destructive actions are routed through confirmation state before execution where
the UI can prevent accidental data loss.

