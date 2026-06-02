# User Guide

## Starting Recorde

Run the `recorde` binary normally, or pass a repository path as the first
argument:

```bash
recorde /path/to/repo
```

When no path is supplied, Recorde attempts to open the most recent repository
from the saved configuration.

## Opening, Cloning, and Initializing Repositories

Use the toolbar to open an existing repository, clone a remote repository, or
initialize a new repository. Recently opened repositories are persisted and shown
for quick access.

Opening a repository loads:

- Commit graph and branch information.
- Working tree status.
- Stashes.
- Merge, rebase, cherry-pick, revert, bisect, submodule, and worktree state.
- Ahead/behind counts against configured upstreams.

## Main Workspace

Recorde is organized around four primary areas:

- Toolbar: repository actions, remote actions, settings, provider views, and
  command entry points.
- Sidebar: branches, tags, stashes, conflicts, submodules, worktrees, and related
  contextual actions.
- Commit graph: commit history, branch labels, search, file history highlighting,
  and commit context actions.
- Right and bottom panels: changes, commit details, staging controls, and diffs.

## Reviewing History

Select a commit in the graph to load its details and changed files. Selecting a
file loads the corresponding diff. Commit context menus support workflows such
as creating branches or tags from a commit, checking out a detached commit,
starting a comparison, cherry-picking, reverting, and opening rebase/bisect
flows.

Use file history to narrow the graph to commits that touched a selected file.
Use blame to annotate the selected file with commit and author information.

## Staging and Committing

The staging panel separates staged and unstaged files. Supported workflows
include:

- Stage or unstage individual files.
- Stage all or unstage all.
- Stage, unstage, or discard individual hunks.
- Stage selected lines from a hunk.
- Discard a file or hunk after confirmation.
- Commit with an optional sign-off.
- Amend the previous commit.
- Soft, mixed, or hard reset, with destructive actions guarded by confirmation.

## Branches, Tags, and Stashes

From the sidebar and toolbar, Recorde supports:

- Creating branches from `HEAD` or a selected commit.
- Checking out, renaming, deleting, force-deleting, restoring, publishing, and
  setting upstreams for branches.
- Creating annotated or lightweight tags from `HEAD` or a selected commit.
- Pushing and deleting remote tags.
- Saving stashes, saving a single file to a stash, applying, popping, dropping,
  or creating a branch from a stash.

Protected branch defaults are `main` and `master`; these are persisted in the
configuration and can be adjusted.

## Syncing With Remotes

Remote operations run through a background job runner and report progress back
to the UI. Supported actions include:

- Fetch all remotes.
- Pull with merge, rebase, or fast-forward-only strategy.
- Push the current branch.
- Push with lease.
- Push and set upstream.

Remote operations can use stored HTTPS provider tokens where available.

## Conflicts and In-Progress Operations

Recorde detects merge, rebase, cherry-pick, revert, and bisect states. Conflict
files are shown in the UI, with actions for choosing ours/theirs, marking a file
resolved, and opening a configured merge tool.

Merge and rebase flows include continue and abort actions. Rebase also supports
skip. Cherry-pick and revert support continue and abort.

## Provider Panels

When an account and matching remote are configured, Recorde can load provider
metadata:

- Pull requests or merge requests.
- Issues.
- CI runs for the current branch.
- Releases.

GitHub, GitHub Enterprise, GitLab.com, and self-hosted GitLab are represented in
the account model.

## External Tools

Recorde can open a terminal, reveal the repository in the file manager, and open
configured diff or merge tools. Tool commands are stored in the application
configuration.

