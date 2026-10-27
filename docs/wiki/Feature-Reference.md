# Feature Reference

## Repository Lifecycle

- Open an existing repository.
- Clone from a remote URL.
- Initialize a repository in a selected directory.
- Choose an initial branch, .gitignore template, license, README and optional initial commit without overwriting existing files.
- Track up to ten recent repositories.
- Refresh repository state after local or external Git changes.

## Commit Graph and Review

- Render paged commit history with branch lanes and bounded background loading.
- Show branch/tag refs, commit time, abbreviated hash, author, and summary.
- Hide and reorder graph columns, choose relative/ISO date/UTC date-time, and save choices per repository.
- Select commits and inspect changed files.
- Load commit diffs and working tree diffs.
- Compare two commits.
- View file history and blame data.
- Open reflog and recover commits to a branch.
- Start and drive a Git bisect session from the commit context menu, with the
  candidate under test selected and the first bad commit reported.
- Reset the current branch to any commit with soft, mixed, or hard mode.
- Undo or redo the last local operation from its recovery snapshot.
- Preview cherry-pick, merge and rebase using graph gestures or keyboard alternatives.
- Interactive rebase with reorder, reword, squash, fixup, drop and edit/split; supported merge topology is retained with validation.
- Drop multiple selected commits through a reviewed rebase plan for supported linear current-branch ranges.

## Working Tree and Index

- Show files with staged and unstaged status separately.
- Support partially staged files.
- Stage and unstage files.
- Stage, unstage, and discard hunks.
- Stage selected lines.
- Export and import patch files.
- Create and amend commits.
- Reset `HEAD` with soft, mixed, or hard mode.
- Stash all changes, optionally including untracked files, or a single file.
- Edit UTF-8 working-tree text files in the built-in editor, with external-change checks before saving.
- Reload current disk contents with explicit confirmation for unsaved edits; failed reloads preserve the buffer.
- Preserve unsaved editor buffers across repository switches and guard application close/update restart.

## Branches and Tags

- Checkout branch (double-click in the sidebar).
- Check out a remote branch as a local tracking branch.
- Create branch from `HEAD`, a selected commit, a remote branch, or stash.
- Rename branch.
- Delete or force-delete branch, with confirmation.
- Delete a branch from its remote, with confirmation.
- Restore recently deleted branches, including after restart.
- Filter sidebar branches and tags; click to select the ref's commit.
- Collapse remote branch groups; filtering expands matching groups. Graph reference labels select on click, check out branches on double-click and offer reference-specific context menus.
- Publish branch and set upstream.
- Checkout detached commit.
- Create tags with optional messages.
- Push tags and delete local or remote tags.
- Merge with default fast-forward, explicit merge commit, fast-forward-only or squash strategies.
- Initialize and run feature/release/hotfix Git Flow workflows, with annotated release tags and conflict retries.

## Integration Operations

- Fetch, pull, and push via the background job runner.
- Use pull strategies of merge, rebase, or fast-forward-only.
- Push with lease.
- Push and set upstream.
- Clone repositories.
- Detect Git LFS installation and tracked patterns.
- Manage submodules and linked worktrees from repository settings.
- Configure sparse checkout, fetch full shallow history and explicitly optimize history loading.

## Conflict and Sequence Handling

- Detect merge, rebase, cherry-pick, revert, and bisect state.
- List conflicted files.
- Choose ours or theirs.
- Mark conflict files as resolved.
- Open configured merge tool.
- Continue or abort merge.
- Continue, abort, or skip rebase.
- Continue or abort cherry-pick and revert.
- Persist conflict editor drafts per worktree and explicitly recover stale drafts.
- Undo/redo supported local operations with exact-state checks; preserve index, non-ignored files, initial-commit and sparse-checkout state.
- Browse, pin, restore, delete and retain recovery snapshots.

See [Recovery](Recovery.md) and [Advanced history](Advanced-History.md) for precise recovery and rebase limits.

## Provider Metadata

- Parse GitHub, GitLab, Bitbucket Cloud and Azure DevOps Services remote URLs.
- Load pull requests or merge requests.
- Load issues.
- Load CI runs.
- Load releases.
- Filter PRs/MRs and issues by open, closed, or all.
- Model check state, review state, CI status, labels, comments, changed files,
  and timestamps.

Capabilities differ by provider. Bitbucket issues/releases and in-app merge are
unavailable; Azure Boards and classic releases cover the whole project. See
[Provider Integrations](Provider-Integrations.md) for the action matrix and
current validation limits.

## UI and Accessibility

- Dark theme by default.
- Classic light/dark and orange-accented light/dark theme choices.
- High contrast mode.
- Font scaling.
- Colorblind graph option.
- Toggleable sidebar, right panel, and diff panel.
- Persisted graph column widths per repository.
- Embedded Material Icons font for installed binaries.
- Embedded Noto Symbols and monochrome emoji fallbacks on every platform.
- Native AccessKit accessibility with commit and branch labels, selected rows,
  and keyboard focus.
- Searchable command palette and persisted custom keyboard shortcuts.
- Searchable offline help with page navigation, shared with this repository wiki.

## Accounts and Tools

- OS-vault token storage and per-repository account selection.
- GitHub/GitLab device sign-in using a registered public client ID.
- Named profiles with commit identity, optional signing key, exact-host provider-account selections and optional global appearance/tool preferences.
- SSH diagnostics, key generation and public-key copying.
- Noninteractive integrated command runner with up to eight repository/worktree-scoped tabs, bounded output/history, copy output, background draining and explicit stop/close control.
- External terminal, editor, diff and merge tool integration.

See [Accounts, identities and tools](Accounts-and-Tools.md) for setup and boundaries. Profiles route to saved accounts while tokens stay in the OS vault; they do not isolate credential stores or SSH sessions. The integrated terminal does not support interactive prompts or full-screen programs.

