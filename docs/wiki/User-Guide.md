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
for quick access. The chevron beside the repository name opens a scrollable list
with repository names, parent paths, and a marker for the current repository.
The same list is available under **File → Recent Repositories**. Hover over a row
to see its full path. The × button removes an entry from recent history; it does
not delete any files. Use **Open another repository…** to browse for a folder.

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

The Changes panel spans the window height, with a separately scrolling file list
above the commit form. Use the three-dot menu beside a file for staging and discard
actions, or check files for batch staging. Commit options contains co-authors,
sign-off and identity/signing controls. Undo actions are under Undo last commit.

The diff occupies the centre column below the graph. Its sliders menu contains
Unified/Split and whitespace options; the search icon opens Find. Light/dark mode,
high contrast, UI scale and colourblind graph colours are in Settings > Appearance.
UI scale 1.0 follows your monitor's native scaling; larger values zoom the interface
on top of that scale. The bundled UI font requires no installation.

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



## Graph operations with a preview

Drag a commit row onto the checked-out branch's HEAD row to preview cherry-picking it. Drag a branch label onto HEAD to preview a merge. Drag the checked-out branch label onto another commit or branch label to preview rebasing the current branch onto that target. The confirmation shows the repository, both endpoints, the expected HEAD, and the direction of the change. Dropping alone never changes Git state; use Confirm to queue the operation or Escape to cancel.

The current branch is the only branch these gestures modify. A changed repository, branch tip, or HEAD invalidates the operation, including changes while dragging or waiting in the queue. Rebasing a configured protected branch is disabled. Graph rebase also refuses replay ranges containing merge commits, so topology and recorded resolutions cannot be silently flattened. Recovery snapshots preserve staged, unstaged, and untracked work; Git may refuse the operation when local changes would be overwritten. Conflicts use the normal Continue and Abort controls.

Branch labels are focusable: activate one to open a merge preview into HEAD. The command palette also provides Preview cherry-pick selected commit and Preview rebase onto selected commit, using the same guarded confirmation as dragging. Merge branch, Rebase current branch, and Interactive rebase from selected commit remain available. Existing context menus remain available.

## Commands and keyboard preferences

Open the command palette with Ctrl+K (Command+K on macOS). Search the available commands, use Up/Down to select a result, Enter to execute, or Escape to close. Unavailable commands explain the missing repository, selection, or operation prerequisite. The palette includes repository lifecycle, fetch/pull/push variants, branch/history actions, settings, provider views, searches, and pane navigation.

Settings > Keyboard lists every registered command and its effective binding. Choose Change and press a combination with Ctrl (Command on macOS), Clear to disable a binding, or Reset all shortcuts to defaults. Conflicts are rejected without replacing the old binding. Escape cancels recording. Window-closing and system combinations cannot be recorded. Custom bindings persist across restarts. Global shortcuts pause while editing text or using another dialog. Search commits and Find in diff are separate configurable commands.
