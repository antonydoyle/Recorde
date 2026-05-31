# Recorde Git Functionality Implementation Plan

This plan is based on the original UI guide and mockup, the current Recorde codebase, and a feature comparison against GitHub Desktop and GitKraken Desktop.

## Current Baseline

Recorde already has a strong foundation:

- Open and reload a local repository.
- Render a GitKraken-style commit graph.
- Show branches, remotes, tags, and stashes.
- Switch, create, and delete local branches.
- Create and delete lightweight tags.
- Save, apply, pop, and drop stashes.
- Stage and unstage files, stage all, unstage all.
- Create commits and amend the previous commit.
- Show commit details and changed files.
- Show working tree and commit diffs in unified or split view.
- Fetch, pull, and push the current branch.
- Show ahead/behind state.
- Cherry-pick, revert, and start an interactive rebase.
- Open settings for app, Git identity, repository identity, and default editor.
- Open the repository in the configured editor.
- Open a provider compare/pull-request page in the browser.

The largest gaps are repo lifecycle management, hunk/line staging, complete merge/rebase flows, conflict resolution, provider authentication, in-app PR/MR management, Git LFS, submodules, worktrees, richer history tools, and accessibility themes.

## Phase 0: Git Engine Foundation

Goal: make future functionality reliable by replacing one-off background threads with a shared operation model.

Implementation notes:

- Add a central async Git job runner with operation IDs, progress text, cancellation where possible, command logging, and result classification.
- Standardize operation state so toolbar spinners, status bar text, modals, and notifications all read from the same source.
- Keep using `git2` for fast local reads, status, refs, and diff operations where it behaves well.
- Shell out to `git` for features libgit2 does not fully cover: Git LFS, worktrees, sparse checkout, some rebase/merge flows, hooks environment, and provider-specific credential behavior.
- Add repo capability detection:
  - bare repo
  - detached HEAD
  - merge, rebase, cherry-pick, or revert in progress
  - LFS installed and repo initialized
  - submodules
  - linked worktrees
  - sparse checkout
  - shallow or partial clone
  - provider remote type
- Add integration test helpers that create temporary repositories and run real Git commands.

Acceptance criteria:

- Every long-running Git action has visible progress and a clear terminal state.
- Errors contain enough context to diagnose what command or libgit2 operation failed.
- UI state cannot get stuck in a permanent loading state after failure.

## Phase 1: Repository Lifecycle And Workspaces

Goal: make Recorde useful from a cold start, not only after opening an existing repo.

Implementation notes:

- Add clone repository.
- Add initialize new repository.
- Add create repository from existing folder.
- Add add existing local repo.
- Add recent repositories, pinned favorites, and remove-from-list.
- Add multi-repository tabs or a workspace switcher.
- Add repository aliases.
- Add open in editor, open terminal, reveal in file manager, and copy repo path.
- Add change remote URL in repository settings.
- Add repository health screen showing current branch, upstream, dirty state, remotes, hooks, LFS, submodules, Git version, and repo path.

Acceptance criteria:

- A user can clone, create, open, and switch repositories without leaving the app.
- Multiple repos can be kept available without repeatedly using a file picker.

## Phase 2: Working Tree And Staging

Goal: make daily committing efficient and safe.

Implementation notes:

- Add hunk staging and unstaging.
- Add line-level staging and unstaging.
- Add discard file, discard hunk, and discard line actions with confirmation.
- Add undo for supported discard/reset operations where feasible.
- Add clear grouping for staged, unstaged, untracked, ignored, conflicted, deleted, renamed, and submodule changes.
- Add binary and image preview states.
- Add "open file externally" and "open containing folder".
- Add `.gitignore` helpers and templates.
- Warn when adding large files that probably belong in LFS.

Acceptance criteria:

- A user can create a commit from selected hunks without using the command line.
- Destructive actions are obvious, confirmed, and logged.

## Phase 3: Commit Workflow

Goal: support professional commit habits and repo conventions.

Implementation notes:

- Add commit templates.
- Add co-author trailers.
- Add signed-off-by support.
- Add GPG and SSH commit signing settings.
- Add amend with message changes, amend without changing message, and amend with staged changes.
- Add reset last commit: soft, mixed, and hard.
- Add commit hook output viewer.
- Add branch protection warnings before committing directly to protected branch names.
- Add commit message validation settings.
- Add copy commit message, copy commit SHA, and copy patch actions.

Acceptance criteria:

- Common amend, signing, co-authoring, and hook workflows are possible without manually editing Git config.
- Hook failures are visible and actionable.

## Phase 4: Branch, Tag, And Ref Management

Goal: make ref operations complete and discoverable.

Implementation notes:

- Add branch rename.
- Add force delete branch with explicit warning.
- Add publish branch.
- Add set, unset, and change upstream branch.
- Add track remote branch.
- Add checkout commit with detached HEAD warning.
- Add compare branch against current branch.
- Add merge branch into current branch.
- Add annotated tags.
- Add signed tags.
- Add push tag and delete remote tag.
- Add stale branch indicators.
- Persist graph column order, visibility, and width per repo.
- Allow graph columns to be shown, hidden, and reordered.

Acceptance criteria:

- A user can manage local and remote branch tracking without using command-line Git.
- Graph column customization survives app restart.

## Phase 5: Sync, Merge, And Rebase

Goal: cover the common remote synchronization workflows safely.

Implementation notes:

- Expand pull menu:
  - fetch all
  - pull fast-forward if possible
  - pull fast-forward only
  - pull with rebase
- Add push with set upstream.
- Add force push with lease, never plain force push by default.
- Add explicit merge branch into current.
- Add explicit rebase current branch onto another branch.
- Add continue, abort, and skip flows for merge, rebase, cherry-pick, and revert.
- Add dirty working tree preflight checks.
- Add upstream missing and divergence warnings.
- Add likely-conflict preflight checks where practical.
- Surface sync state in toolbar, status bar, and notifications.

Acceptance criteria:

- A user can resolve the full lifecycle of a merge/rebase/cherry-pick/revert operation from inside Recorde.
- Pull and push behavior is configurable and visible.

## Phase 6: Conflict Resolution

Goal: make conflicts impossible to miss and practical to resolve.

Implementation notes:

- Add a persistent conflict mode banner while a merge, rebase, cherry-pick, or revert is blocked.
- Add a conflict file list.
- Add actions for each conflicted file:
  - open file
  - choose ours
  - choose theirs
  - mark resolved
  - open external merge tool
- Add a basic three-way conflict viewer/editor.
- Add abort operation for the active Git sequence.
- Add clear "continue" action once all conflicts are resolved.
- Later enhancement: early conflict detection before merge/rebase.

Acceptance criteria:

- Conflict notifications are not easy to miss.
- A user can finish or abort a conflicted operation without leaving the app.

## Phase 7: History, Search, And Review

Goal: make the graph useful on large real repositories.

Implementation notes:

- Add advanced graph search:
  - author
  - committer
  - message
  - SHA
  - branch
  - path
  - date range
- Add file history.
- Add folder history.
- Add blame view.
- Add reflog viewer.
- Add recover branch or commit from reflog.
- Add compare two commits, branches, or tags.
- Add bisect helper:
  - start
  - mark good
  - mark bad
  - visualize current step
  - reset bisect
- Add commit graph caching for large repositories.

Acceptance criteria:

- A user can answer "when did this change and why?" from inside Recorde.
- Large repositories remain responsive while searching and scrolling.

## Phase 8: Stash, Patch, And WIP

Goal: support interrupted work and patch exchange.

Implementation notes:

- Add custom stash messages.
- Add stash selected files.
- Add stash selected hunks.
- Add stash preview diff.
- Add apply stash to new branch.
- Add branch from stash.
- Add export patch.
- Add import/apply patch.
- Consider a friendlier "shelf" UI on top of raw Git stash entries.

Acceptance criteria:

- A user can stash partial work and inspect it before applying.
- Patch workflows do not require leaving the app.

## Phase 9: Pull Request And Merge Request UI

Goal: move beyond opening a browser.

Implementation notes:

- Detect provider from remotes:
  - GitHub.com
  - GitHub Enterprise
  - GitLab.com
  - self-managed GitLab
- List PRs/MRs for the current repository:
  - mine
  - review requested
  - draft
  - blocked
  - merged
  - closed
- Create PR/MR from current branch.
- Support target branch, title, body, draft flag, reviewers, assignees, and labels where provider APIs allow.
- Load PR/MR templates from the repository.
- Show commits, changed files, checks, and review summary.
- Checkout PR branch.
- Open PR/MR in browser.
- Copy PR/MR URL.
- Link issues from branch names and commit messages.

Acceptance criteria:

- A user can create and inspect a PR/MR from Recorde.
- Provider-specific unsupported actions fail gracefully with clear messaging.

## Phase 10: Auth And Accounts

Goal: make provider and Git authentication first-class.

Implementation notes:

- Add account model:
  - provider
  - host
  - username
  - display name
  - avatar URL
  - token scopes
  - token storage key
- Support GitHub.com.
- Support GitHub Enterprise.
- Support GitLab.com.
- Support self-managed GitLab.
- Use OAuth device flow where possible.
- Add personal access token fallback for enterprise and self-hosted cases.
- Store secrets in the OS keychain using a Rust keyring crate.
- Bridge provider auth into Git HTTPS credential flows.
- Keep SSH support separate from provider API auth.
- Show SSH agent status and key diagnostics.
- Show host key and permission failures clearly.
- Add account switcher from the user icon.
- Add logout and revoke-token actions where the provider supports it.
- Add enterprise settings:
  - base URL
  - custom CA notes
  - proxy settings
  - SSO status where available

Acceptance criteria:

- A user can authenticate with GitHub, GitHub Enterprise, GitLab, or self-managed GitLab.
- HTTPS Git operations and provider API operations can share credentials safely.
- Secrets are not stored in plaintext config files.

## Phase 11: Git LFS

Goal: add explicit large-file support.

Current assessment: Recorde does not currently appear to support enabling or managing Git LFS.

Implementation notes:

- Detect `git lfs version`.
- Show whether LFS is installed and whether the current repo is initialized for LFS.
- Add initialize LFS for repo:
  - run `git lfs install`
  - update `.gitattributes`
  - show the resulting file as a normal staged change
- Add track pattern.
- Add untrack pattern.
- Add track selected file pattern from file context menu.
- Show LFS badges in changed files and commit details.
- Add LFS pull, fetch, and prune commands.
- Use a strong warning before prune because it removes local unreferenced LFS objects.
- Warn when adding files above configurable size thresholds.
- Show LFS pointer metadata in diffs.
- Document that existing tracked files need migration or remove/re-add to become LFS-managed.

Acceptance criteria:

- A user can initialize LFS and add tracking patterns from Recorde.
- Large file warnings appear before a user accidentally commits large binaries to normal Git history.

## Phase 12: Submodules, Worktrees, Sparse Checkout

Goal: cover advanced but common repository layouts.

Implementation notes:

- Submodules:
  - detect submodules
  - init/update
  - sync
  - deinit
  - open nested repo
  - show submodule dirty state and checked-out commit
- Worktrees:
  - list worktrees
  - create worktree
  - create branch with worktree
  - remove worktree
  - prune stale worktrees
  - open worktree as tab
- Sparse checkout:
  - detect sparse mode
  - show sparse path list
  - edit sparse paths
  - disable sparse checkout
- Add shallow and partial clone indicators.

Acceptance criteria:

- Repositories using submodules or worktrees are understandable from the UI.
- Worktree workflows are useful for parallel tasks and agent-assisted development.

## Phase 13: Provider Integrations

Goal: support surrounding development workflows, not only raw Git.

Implementation notes:

- Add GitHub/GitLab issues list.
- Add branch-from-issue.
- Add CI/checks view for the current branch, commit, and PR/MR.
- Add provider notifications for PR reviews and failed checks.
- Add releases/tags view.
- Add optional Jira/Trello integration later, behind a generic issue-provider abstraction.
- Add clear offline and unauthenticated states.

Acceptance criteria:

- A user can see why a branch is blocked without opening the provider website.
- Provider integrations degrade cleanly when auth is missing.

## Phase 14: Settings And Administration

Goal: make configuration discoverable and maintainable.

Implementation notes:

- Global Git identity.
- Repository Git identity.
- Default pull strategy.
- Push behavior.
- Fetch pruning.
- Auto-fetch interval.
- Default branch preference.
- Editor, terminal, file manager, diff tool, and merge tool.
- Commit signing.
- LFS settings.
- Keyboard shortcuts.
- Provider accounts.
- Credential diagnostics.
- Proxy and enterprise network notes.
- Import and export settings.

Acceptance criteria:

- Most behavior-changing preferences are discoverable from settings.
- Repo-level settings clearly override global settings where applicable.

## Phase 15: Quality, Packaging, And Safety

Goal: make Recorde shippable.

Implementation notes:

- Add platform packaging.
- Add app icon wiring for Linux, Windows, and macOS.
- Add release build documentation.
- Add optional update mechanism or documented install/update path.
- Add command log export for bug reports.
- Add crash/error report export.
- Add undo stack for supported destructive actions.
- Add graph layout tests.
- Add Git operation integration tests.
- Add UI smoke tests for key flows.

Acceptance criteria:

- A release build can be installed and launched like a normal desktop app.
- Critical Git operations are covered by repeatable tests.

## Phase 16: Light Mode And Accessibility

Goal: properly support light mode and high-contrast accessibility modes after the main UI surface stabilizes.

Implementation notes:

- Replace dark-only constants with semantic theme tokens.
- Implement proper light mode.
- Implement high-contrast dark mode.
- Implement high-contrast light mode.
- Audit contrast for:
  - toolbar buttons
  - selected rows
  - diff additions/deletions
  - graph lanes
  - branch/tag labels
  - disabled text
  - notification banners
- Add color-blind-safe graph palettes.
- Add reduced motion option.
- Add keyboard navigation for:
  - menus
  - command palette
  - graph rows
  - file list
  - dialogs
  - settings
- Add accessible labels and tooltips for icon-only buttons and status indicators.
- Add font scaling support.

Acceptance criteria:

- Light mode is not just inverted dark mode.
- High-contrast modes meet accessibility contrast targets.
- The app can be operated substantially by keyboard.

## Recommended Build Order

1. Phase 0: Git Engine Foundation.
2. Phase 2: Working Tree And Staging.
3. Phase 5: Sync, Merge, And Rebase.
4. Phase 6: Conflict Resolution.
5. Phase 10: Auth And Accounts.
6. Phase 9: Pull Request And Merge Request UI.
7. Phase 11: Git LFS.
8. Phase 12: Submodules, Worktrees, Sparse Checkout.
9. Phase 7: History, Search, And Review.
10. Phase 8: Stash, Patch, And WIP.
11. Phase 1: Repository Lifecycle And Workspaces.
12. Phase 13: Provider Integrations.
13. Phase 14: Settings And Administration.
14. Phase 15: Quality, Packaging, And Safety.
15. Phase 16: Light Mode And Accessibility.

Theme token work from Phase 16 should start early, but the full accessibility pass is best done after the major UI surfaces are stable.

## Reference Links

- [Original UI guide](file:///home/ant/Pictures/Recorde/ui-guide.md)
- [Original UI mockup](file:///home/ant/Pictures/Recorde/ui-mockup.png)
- [GitHub Desktop documentation](https://docs.github.com/en/desktop)
- [GitHub Desktop Git LFS documentation](https://docs.github.com/en/desktop/configuring-and-customizing-github-desktop/about-git-large-file-storage-and-github-desktop?apiVersion=2022-11-28)
- [GitKraken Desktop interface guide](https://support.gitkraken.com/start-here/interface/)
- [GitKraken Desktop Git LFS documentation](https://help.gitkraken.com/gitkraken-desktop/git-lfs/)
- [GitKraken Desktop product page](https://www.gitkraken.com/git-client)
