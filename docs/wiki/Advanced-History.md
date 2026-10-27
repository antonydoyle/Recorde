# Advanced history and recovery

## Edit or split a commit

Choose **edit / split** in Interactive Rebase. Git pauses after applying that
commit. Amend it using Changes, or select **Split commit** in the operation
banner. Split moves HEAD to the parent and keeps the files in the working tree.
Stage selected files, hunks or lines and commit each part. The parts retain the
original author; normal commit hooks and repository signing settings apply.
Continue becomes safe once at least one replacement commit exists and all
remaining changes have been committed and the final tree exactly reproduces the
original commit. Stashing or discarding part of the original change blocks Continue. The split marker survives
restarting Recorde. Abort uses Git's native rebase recovery.

## Ranges containing merges

Recorde edits Git's `--rebase-merges` plan. Reorder, squash, fixup and drop work
within consecutive commits on the same branch segment. Git's labels, branch
points, resets and merge steps remain in place. A move across those boundaries,
a squash into a merge, or dropping an entire segment is rejected before replay.
Merge commits themselves support Pick and Reword.

Git recreates merges during replay. Recorde checks two-parent merges using
`--remerge-diff` and refuses a range containing manual resolution changes that
could otherwise disappear. Octopus merges are refused because Git cannot verify
their manual resolutions this way. Select a range excluding that merge, or use a native
Git workflow that explicitly reapplies and reviews its resolution. See Git's
[rebase documentation](https://git-scm.com/docs/git-rebase).

## Sparse recovery

Recovery saves the index and worktree trees, sparse patterns, cone/index settings
and skip-worktree paths. Undo and redo retain the sparse selection, staged versus
unstaged changes and ignored files. Linked worktrees keep separate recovery
records. Changing the sparse selection after an operation invalidates automatic
recovery until the original selection is restored. Later file or index edits
also require preservation before recovery.

## Large and replaced histories

Graph Squash and Drop selection plans run in cancellable background tasks. Cancel while Preparing history plan is visible to stop planning; the repository has not been rewritten at that stage. Large range listings stream through a private temporary file instead of the ordinary diagnostic-output buffer. Plans support up to 100,000 commits; a larger range asks you to choose a more recent starting commit. The reviewed repository, branch, HEAD and replacement refs must still match before a prepared plan is used.

Cold repositories load bounded native Git pages without writing a commit graph.
The loaded graph is ordered with children before parents. With skewed commit
clocks, loading more history can refine row positions; commit identity remains
the original OID. **Optimize history loading**, under Settings > Repository >
Checkout and history, explicitly writes Git's disposable index to enable stable
topology ordering from the first page.

Replacement refs affect graph metadata, ancestry, commit details and diffs,
including replacement trees and blobs. The visible OID remains the original
object name, matching native Git. Changed replacement refs invalidate cached
pages, and invalid replacement cycles fail rather than falling back to an
inconsistent original-object view. Git does not use its commit-graph acceleration
while replacement refs are active.
