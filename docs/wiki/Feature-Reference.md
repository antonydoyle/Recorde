# Feature Reference

## Repository Lifecycle

- Open an existing repository.
- Clone from a remote URL.
- Initialize a repository in a selected directory.
- Track up to ten recent repositories.
- Refresh repository state after local or external Git changes.

## Commit Graph and Review

- Render the commit graph from libgit2 repository data.
- Show branch/tag refs, commit time, abbreviated hash, author, and summary.
- Select commits and inspect changed files.
- Load commit diffs and working tree diffs.
- Compare two commits.
- View file history and blame data.
- Open reflog and recover commits to a branch.
- Start and drive a Git bisect session.

## Working Tree and Index

- Show files with staged and unstaged status separately.
- Support partially staged files.
- Stage and unstage files.
- Stage, unstage, and discard hunks.
- Stage selected lines.
- Export and import patch files.
- Create and amend commits.
- Reset `HEAD` with soft, mixed, or hard mode.

## Branches and Tags

- Checkout branch.
- Create branch from `HEAD`, a selected commit, or stash.
- Rename branch.
- Delete or force-delete branch.
- Restore recently deleted branches while the app is running.
- Publish branch and set upstream.
- Checkout detached commit.
- Create tags with optional messages.
- Push tags and delete local or remote tags.

## Integration Operations

- Fetch, pull, and push via the background job runner.
- Use pull strategies of merge, rebase, or fast-forward-only.
- Push with lease.
- Push and set upstream.
- Clone repositories.
- Detect Git LFS installation and tracked patterns.
- List submodules and linked worktrees.

## Conflict and Sequence Handling

- Detect merge, rebase, cherry-pick, revert, and bisect state.
- List conflicted files.
- Choose ours or theirs.
- Mark conflict files as resolved.
- Open configured merge tool.
- Continue or abort merge.
- Continue, abort, or skip rebase.
- Continue or abort cherry-pick and revert.

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
- High contrast mode.
- Font scaling.
- Colorblind graph option.
- Toggleable sidebar, right panel, and diff panel.
- Persisted graph column widths per repository.
- Embedded Material Icons font for installed binaries.
- Embedded Noto Symbols and monochrome emoji fallbacks on every platform.
- Native AccessKit accessibility with commit and branch labels, selected rows,
  and keyboard focus.

