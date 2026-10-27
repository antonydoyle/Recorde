# User Guide

## Your first commit

1. Open an existing repository, clone a remote URL, or initialize a new folder from File or the toolbar.
2. Check Settings > Accounts & SSH for your commit name and email. Add a provider account if you need authenticated remote operations.
3. Make an edit, then select its file in Changes to review the diff.
4. Stage the file, selected hunk or selected lines. Review the staged section so the commit contains exactly the intended changes.
5. Enter a clear commit summary and choose Commit. A commit records local history; it does not automatically push.
6. Publish the branch to set an upstream, or Push when one is already configured.

Use Help > User Guide to read this documentation offline. Search help topics to find a workflow, then use Back, Forward or Home to navigate. The same pages are maintained in the repository's docs/wiki directory.

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
sign-off and identity/signing controls. Undo actions are under Undo last commit:
Soft, Mixed and Hard each show the commit being undone and ask for confirmation.
The target is fixed when you confirm; if HEAD changes before the queued reset
starts, it is refused. Undoing or amending a commit that is already on the
upstream warns that the next push needs Force Push with Lease.

File menus open files in the editor set in App Settings (or `VISUAL`/`EDITOR`;
Notepad on Windows) and reveal them in the system file manager. File names are
passed as literal arguments, never through a shell.

The diff occupies the centre column below the graph. Its sliders menu contains
Unified/Split and whitespace options; the search icon opens Find. Light/dark mode,
high contrast, UI scale and colourblind graph colours are in Settings > Appearance.
UI scale 1.0 follows your monitor's native scaling; larger values zoom the interface
on top of that scale. The bundled UI font requires no installation.

## Reviewing History

Select a commit in the graph to load its details and changed files. Selecting a
file loads the corresponding diff. Commit context menus support workflows such
as creating branches or tags from a commit, checking out a detached commit,
resetting the current branch to a commit, starting a comparison, cherry-picking,
reverting, and opening rebase/bisect flows.

**Reset current branch to here** offers Soft (keep changes staged), Mixed (keep
changes unstaged) and Hard (discard changes to tracked files) modes. A recovery
snapshot of HEAD, the index and working files is saved first, so the reset can
be undone.

To bisect, choose **Bisect...** from a commit's context menu and start a session,
then close the dialog and right-click commits to **Mark Good** or **Mark Bad**.
Git checks out each commit to test and Recorde selects it. Once Git identifies
the first bad commit, the Bisect dialog shows it with **Show in graph**. **Reset**
ends the session and returns to the original branch.

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

**Undo last operation** (Ctrl+Z, or Command+Z on macOS, when no text field is
focused) restores the next available local recovery step after confirmation.
**Redo undone operation** uses Ctrl+Shift+Z, or Command+Shift+Z. Undo covers
local operations such as commits, resets,
discards, branch switches, merges and rebases. It does not undo pushes or other
remote changes, and it stops if files changed after the operation. All snapshots
are listed under Repository settings > Recovery snapshots.

See [Undo, redo and recovery](Recovery.md) for snapshot retention, untracked and
ignored files, first commits, and why recovery can refuse later edits.

## Branches, Tags, and Stashes

From the sidebar and toolbar, Recorde supports:

- Creating branches from `HEAD`, a selected commit, or a remote branch.
- Checking out, renaming, deleting, force-deleting, restoring, publishing, and
  setting upstreams for branches.
- Checking out a remote branch as a local branch that tracks it, merging or
  rebasing onto it, and deleting it from the remote.
- Creating annotated or lightweight tags from `HEAD` or a selected commit.
- Pushing and deleting remote tags.
- Saving stashes (optionally including untracked files), saving a single file
  (including a new file) to a stash, applying, popping, dropping, or creating a
  branch from a stash.

Click a branch or tag in the sidebar to select its commit in the graph, and
double-click a branch to check it out. The filter box at the top of the sidebar
narrows branches and tags by name and expands matching sections.

Checking out a remote branch reuses a local branch of the same name only if it
already tracks that remote branch; otherwise Recorde refuses rather than moving
it. Force deleting a branch, deleting a remote branch and deleting a remote tag
all ask for confirmation. A deleted branch's last tip is kept under Recently
deleted so it can be restored.

Protected branch defaults are `main` and `master`; these are persisted in the
configuration and can be adjusted. The same list drives the commit form's
"committing directly to" warning and the graph's rebase protection.

## Syncing With Remotes

Remote operations run through a background job runner and report progress back
to the UI. Supported actions include:

- Fetch all remotes.
- Pull uses Git configuration by default: `branch.<name>.rebase` overrides `pull.rebase`,
  and `pull.ff` controls merges when rebase is disabled. The tooltip and first context-menu
  item show the effective strategy. Settings can explicitly select merge, rebase, or
  fast-forward only; the Pull menu also offers explicit rebase and fast-forward-only actions.
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

GitHub, GitHub Enterprise, GitLab.com, self-hosted GitLab, Bitbucket Cloud and
Azure DevOps Services are represented in
the account model.

Available panels and actions differ: Bitbucket issues/releases and in-app merge
are unavailable, while Azure Boards and classic releases cover the whole project.
See [Provider Integrations](Provider-Integrations.md) for the capability matrix.
GitHub/GitLab also offer [browser sign-in](../browser-sign-in.md) with a registered
public client ID; other providers use the token form.

## External Tools

Recorde can open a terminal, reveal the repository in the file manager, and open
configured diff or merge tools. Tool commands are stored in the application
configuration.



## Graph operations with a preview

Drag a commit row onto the checked-out branch's HEAD row to preview cherry-picking it. Drag a branch label onto HEAD to preview a merge. Drag the checked-out branch label onto another commit or branch label to preview rebasing the current branch onto that target. The confirmation shows the repository, both endpoints, the expected HEAD, and the direction of the change. Dropping alone never changes Git state; use Confirm to queue the operation or Escape to cancel.

The current branch is the only branch these gestures modify. A changed repository, branch tip, or HEAD invalidates the operation, including changes while dragging or waiting in the queue. Rebasing a configured protected branch is disabled. Graph rebase also refuses replay ranges containing merge commits, so topology and recorded resolutions cannot be silently flattened. Recovery snapshots preserve staged, unstaged, and untracked work; Git may refuse the operation when local changes would be overwritten. Conflicts use the normal Continue and Abort controls.

Graph branch and tag labels select their commit on click; double-clicking a branch checks it out. Their context menus offer reference-specific actions. The command palette provides Preview cherry-pick selected commit and Preview rebase onto selected commit, using the same guarded confirmation as dragging. Merge branch, Rebase current branch, and Interactive rebase from selected commit remain available.

Use Columns, or right-click a graph header, to hide or reorder columns and choose
relative dates, ISO dates or ISO date/time in UTC. Commit remains visible. These
preferences and resized widths are saved per repository. Remote branches in the
sidebar are grouped by configured remote; filtering expands matching groups.

Multi-select commits and choose Drop N commits to review a rebase plan before
removing them. This requires a supported linear range on the current branch;
root, off-branch and merge-containing ranges are rejected.

## Commands and keyboard preferences

Open the command palette with Ctrl+K (Command+K on macOS). Search the available commands, use Up/Down to select a result, Enter to execute, or Escape to close. Unavailable commands explain the missing repository, selection, or operation prerequisite. The palette includes repository lifecycle, fetch/pull/push variants, branch/history actions, settings, provider views, searches, and pane navigation.

Settings > Keyboard lists every registered command and its effective binding. Choose Change and press a combination with Ctrl (Command on macOS), Clear to disable a binding, or Reset all shortcuts to defaults. Conflicts are rejected without replacing the old binding. Escape cancels recording. Window-closing and system combinations cannot be recorded. Custom bindings persist across restarts. Global shortcuts pause while editing text or using another dialog. Search commits and Find in diff are separate configurable commands.

See [Keyboard shortcuts](Keyboard-Shortcuts.md) for the complete default bindings.

## Templates, identities and tools

Initialize offers a default branch name, .gitignore template, optional license,
README and initial commit. Existing files are not overwritten. The initial
commit includes only generated files. Review a license before selecting it.

The integrated terminal runs one noninteractive shell command at a time in the
repository folder. Use an external terminal for interactive prompts, editors and
full-screen tools. Identity profiles, signing, SSH keys, the built-in file editor
and tool setup are covered in [Accounts, identities and tools](Accounts-and-Tools.md).

## Git Flow and merge choices

Merge branch offers fast-forward when possible, an explicit merge commit,
fast-forward-only, and squash-into-one-commit strategies. Choose the destination
by checking it out before merging.

Branch > Git Flow guides feature, release and hotfix branches using
repository-local production/development settings. Finishing a release or hotfix
merges both destinations and creates an annotated tag; publishing is a separate
step. See [Branches and Git Flow](Branching-and-Git-Flow.md) for the full workflow
and conflict recovery.

## Repository maintenance

Settings > Repository contains recovery snapshots, sparse checkout, history
optimization, full-history fetch for shallow repositories, linked worktrees,
submodules and other repository-specific controls. Sparse checkout requires a
clean working tree and index. Review paths before applying a new selection.

History initially loads a bounded set of commits. Load More reveals older
history; commit search covers the loaded history. Optimize history loading
builds Git's disposable history index when requested.

## Appearance and updates

Settings > Appearance includes light and dark themes, beige/orange and dark
orange-accented alternatives, high contrast, scale and colour-blind graph colours.
See [Themes and accessibility](Themes-and-Accessibility.md) for layout and
keyboard guidance.

Settings > Updates provides available update information and controls. Save
unsaved file editor buffers before installing an update that restarts the app.
Use Settings > About when checking the version for a bug report.
