# Branches and Git Flow

## Everyday branch work

Create a branch from the toolbar, the command palette, or a commit's context menu. The sidebar lists local branches and remote-tracking branches. Click a branch to select its tip in history; double-click to check it out. Use the sidebar filter when a repository has many refs.

A remote branch can be checked out as a local tracking branch. An existing local branch is reused only when it already tracks that same remote branch. Publish branch / set upstream connects a local branch to its remote destination.

Branch context menus also expose rename, merge, rebase, upstream and delete actions. Recently deleted branches retain their last tip and can be restored after restarting Recorde. Deleting a branch on a remote changes the shared repository and requires confirmation.

Remote branch deletion uses the reviewed remote-tracking tip as a lease. If someone has pushed a newer tip, deletion refuses; fetch and review before retrying. Fast-forward to upstream is available only for an inactive local branch that can advance without merging. A branch checked out in any worktree, including the main worktree when Recorde has a linked worktree open, cannot be moved this way. An unreadable worktree listing also blocks the action.

## Choose how to merge

Check out the destination branch first, then open Merge branch and choose the source. Commit or stash tracked changes before merging. Git also refuses to overwrite conflicting untracked files.

- Fast-forward if possible moves the destination tip forward when possible, otherwise creates a merge commit.
- Always create a merge commit preserves an explicit integration point. You can supply a commit message.
- Fast-forward only refuses divergent histories; it never creates a merge commit.
- Squash into one commit combines the source changes into a new commit without recording merge ancestry. Git will not consider that source branch merged for normal branch deletion.

Review conflicts before choosing Continue. Abort stops the current merge. Already completed operations require recovery or a deliberate follow-up commit.

Squash conflicts remain an explicit in-progress operation even though Git does not create a normal merge HEAD. Resolve and stage all conflicts, then Continue to create the squash commit. Abort uses a merge-preserving reset to clear the squash. Recorde refuses to continue or abort its recorded squash if the original branch or HEAD has changed. Incomplete recovery records are not offered as usable Undo steps.

## Rebase and graph previews

Rebase current branch replays local commits onto a selected base. Interactive rebase lets you reorder, reword, squash, fixup, edit, split or drop commits within supported ranges. Rebasing rewrites commit IDs; coordinate changes to published branches before pushing.

Drag a commit onto the checked-out HEAD row to preview cherry-picking it. Drag a branch label onto HEAD to preview a merge. Drag the checked-out branch label onto another commit or branch to preview rebasing onto that target. The preview shows source, destination, repository and expected HEAD. Dropping alone does not modify Git; Confirm queues the operation.

Click graph branch or tag labels to select their commit; double-click a branch label to check it out. Their context menus offer reference-specific branch and tag actions. Keyboard users can use these menus or the palette's Preview cherry-pick selected commit and Preview rebase onto selected commit commands. A changed HEAD or branch tip invalidates a preview. Protected branches cannot be rebased by these graph gestures, and graph rebase refuses merge-containing replay ranges.

Select several commits and choose Drop N commits to review an interactive rebase plan that removes them. Root commits, off-branch commits and merge-containing ranges are refused. Review the plan before starting; dropping rewrites history and can remove changes from the branch.

Squash and Drop plans are prepared in a background task. Preparing history plan offers Cancel; cancellation stops planning without rewriting history. Large-range checks use streamed Git output rather than a diagnostic-output size limit. A result from an older repository or changed HEAD is not applied to the current view.

Multi-selection Cherry-pick and Revert also show a confirmation listing the exact repository, branch, HEAD and ordered commits. Commit or stash tracked changes first; untracked files remain subject to Git's overwrite checks. Selected IDs must be distinct and complete, merge commits and replacement refs are not supported, and another Git sequence must not be active. A changed repository, branch or HEAD invalidates the operation before it runs.

See [Advanced history](Advanced-History.md) for merge-preserving interactive rebase and splitting commits.

## Initialize Git Flow

Open Branch > Git Flow, or search for Git Flow in the command palette. Git Flow uses a production branch for finished releases and a development branch for ongoing work.

1. Create the first commit and an existing production branch, usually main.
2. Choose production and development branch names. They must differ.
3. Set feature, release and hotfix prefixes, usually feature/, release/ and hotfix/.
4. Optionally set a version tag prefix such as v.
5. Choose Initialize Git Flow. A missing development branch is created from production.

Settings are local to this repository and use the same Git configuration keys as the git-flow command-line extension. Recorde does not require that extension to be installed.

## Start and finish work

- Feature starts from development and finishes by merging back into development.
- Release starts from development and finishes by merging into production, creating an annotated version tag, and merging into development.
- Hotfix starts from production and finishes by merging into production, creating an annotated version tag, and merging into development.

Enter only the name after the displayed prefix, then choose Start. Recorde creates and checks out the new branch. Only one release and one hotfix of each kind can be in progress at a time.

Commit or stash tracked changes before Finish. Successful completion checks out development and deletes the completed local branch, retaining its tip in Recently deleted. Finish does not push branches or tags. Review the results and publish the required branches and tags explicitly.

Finish first shows the source branch, destination branches and any release tag for confirmation. The reviewed branch tips and HEAD are checked again when queued work starts; a changed target requires a fresh review. Branch deletion verifies the finished tip has been merged into every required destination, so an outdated upstream tracking branch does not prevent completion.

If a finish merge conflicts, resolve the files, complete that merge, then reopen Git Flow and choose Finish again. Completed merge steps are skipped. A colliding release tag that points somewhere else is refused. Finishing may involve several Git operations; stopping midway does not roll back steps that already completed.

Git Flow operations are not recorded as Undo steps. Recently deleted can recreate the finished branch at its saved tip; it does not undo the completed merges, tags or Git Flow configuration.

See [Undo, redo and recovery](Recovery.md) before rewriting or restoring local history.
