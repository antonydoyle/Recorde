# Undo, redo and recovery

## Undo a local operation

Use Undo last operation from the command palette or Ctrl+Z when a text field is not focused. On macOS use Command+Z. Review the confirmation describing the operation before restoring it. Redo undone operation uses Ctrl+Shift+Z, or Command+Shift+Z.

Undo and redo maintain a local operation history. A new operation clears the redo path. They are separate commands: choosing Undo again moves further back through available operations.

Recovery snapshots preserve the recorded HEAD, index and non-ignored working files, including untracked files and the state before a first commit. Staged and unstaged changes remain distinct. Supported local operations include commits, resets, discards, branch switches, merges and rebases. Linked worktrees have their own recovery records.

Staging and unstaging save fast index-only steps. These record HEAD, the branch, the index tree and any sparse-checkout state; they do not snapshot or restore working files, including untracked files. Undo restores the previous staged content, and Redo reapplies it, leaving working files untouched. These steps refuse if HEAD, the branch, the index or the recorded sparse state has changed since the operation.

Git Flow initialization/start/finish and fast-forwarding an inactive branch do not create Undo steps. Git Flow Finish retains the deleted branch tip in Recently deleted; restoring it recreates that branch but does not reverse completed merges or tags. For an inactive branch fast-forward, inspect that branch's reflog and preserve the earlier tip before a deliberate reset. An older Undo entry does not represent these actions and can refuse if they changed the recorded state.

## When recovery refuses

Restore checks the recorded branch, HEAD and file/index state before making changes. Later edits, changed sparse-checkout settings, a conflicting active Git sequence, or untracked/ignored paths that would be overwritten can prevent recovery. Read the error and preserve the newer work before trying again.

Commit or stash later work where appropriate, or copy it outside the repository before a deliberate manual recovery. A commit can add a recovery step; stash actions do not. Inspect the desired snapshot rather than assuming the next Undo still refers to the earlier action. Incomplete operations without a verified final state block older automatic Undo/Redo steps; their snapshots remain available for inspection in Settings.

Ignored files are excluded from snapshots. Recovery is not a backup of build outputs, ignored secrets, external tools, repository configuration, provider actions or remote state. It does not undo a push, remote branch deletion, posted comment or merged pull request. Restore and review local work before deciding whether to publish a corrective commit.

## Browse and retain snapshots

Open Settings > Repository > Recovery snapshots to see saved entries. Restore opens a confirmation; successful restore saves an inverse snapshot for redo. The original saved data remains available.

Use Keep to pin a snapshot. Automatic cleanup can keep all snapshots, or remove older snapshots after 30 or 90 days. Cleanup always preserves the newest 20 snapshots, pinned snapshots and incomplete operations. Delete removes an entry and its protected Git reference; Git garbage collection may later remove the saved data.

The default retention is Keep all. Creating a full snapshot hashes non-ignored working files, including untracked files, into Git's object store. Staging and unstaging only save the index and avoid this work. Large unignored build outputs can make operations that save full snapshots slower and increase disk usage. Add disposable generated files to `.gitignore` and choose a retention policy appropriate for the repository.

## Reset and discard

Reset current branch to here is available from a commit's context menu. Soft keeps changes staged, Mixed keeps them unstaged, and Hard discards tracked working changes. Review the selected commit and mode before confirming. A recovery snapshot is created before the change.

Reset records the displayed branch, HEAD and target commit. If the branch or HEAD changes while the action waits in the queue, it refuses; reopen the dialog and review the current state.

Discard is available for files and hunks. Review the confirmation carefully, especially for newly created files. Undo depends on a valid saved snapshot; it is not a substitute for keeping valuable work committed or backed up.

## Conflicts and cancellation

Resolve each conflicted file, review the result, mark it resolved, then Continue the active merge, rebase, cherry-pick or revert. Rebase also offers Skip; use it only when intentionally dropping that replayed commit. Abort stops the active sequence using Git's normal recovery behavior.

A conflicted squash merge also has Continue and Abort controls. Continue requires every conflict to be resolved and staged, then commits the squash result using its reviewed message. Abort clears the squash with Git's merge-preserving reset. Recorde verifies the original branch and HEAD before completing its squash sequence. Preserve later work and investigate if that verification refuses.

Conflict editor drafts are saved per worktree. Reopening the same conflict restores the draft. If the conflict changed, explicitly recover the older draft only after reviewing it. Draft save errors are displayed in the editor.

Cancel stops queued work or a running command and its owned child processes. It does not reverse writes that completed. Refresh and inspect status and the operation banner before retrying. A leftover Git lock must only be removed after verifying that no Git process is still using it.

Closing the application waits for running and queued background work, including command tabs. Keep working cancels the pending quit so you can use Stop or Cancel on the operation itself. Unsaved editor drafts must still be saved or explicitly discarded before shutdown.

## Stash actions and retained stashes

Apply, Pop, Branch from stash and Drop act on the reviewed stash object and list position. If another action changes the list before the queued job starts, Recorde refuses and asks you to refresh. Drop requires confirmation. Pop keeps the stash if applying it fails.

Removing a stash through Drop, a successful Pop or Branch from stash retains its commit under `refs/recorde/dropped-stashes/<fullOID>`. Stash actions are not Undo steps. To find retained entries, run:

```sh
git for-each-ref --format="%(refname)" refs/recorde/dropped-stashes/
```

Copy the exact reference printed for the desired stash. Apply it to the current worktree with `git stash apply <reference>`, or return it to the stash list with `git stash store -m "Recovered stash" <reference>`. Replace `<reference>` with that complete printed name. Applying can conflict; inspect the current branch and preserve later work first. Retained stash references are separate from snapshot retention and remain until deliberately removed.

## Recover a branch or inspect previous tips

Use Recently deleted in the sidebar to restore a deleted local branch. Use the reflog to inspect earlier reference tips after an unexpected checkout or history change. Creating a new branch at a known commit can preserve that history while you decide the next step.

See [Advanced history](Advanced-History.md) for sparse-checkout recovery and interactive rebase limits.
