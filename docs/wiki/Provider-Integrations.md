# Provider Integrations

Recorde has provider-neutral models for repository metadata and provider-specific
adapters for GitHub, GitLab, Bitbucket Cloud and Azure DevOps Services. The
capability matrix below describes the feature candidate; live-provider acceptance
has not been completed for that candidate.

## Supported Providers

- GitHub.com.
- GitHub Enterprise.
- GitLab.com.
- Self-hosted GitLab.
- Bitbucket Cloud (not Bitbucket Data Center).
- Azure DevOps Services (not Azure DevOps Server).

| Capability | GitHub / Enterprise | GitLab / self-hosted | Bitbucket Cloud | Azure DevOps Services |
| --- | --- | --- | --- | --- |
| Account authentication | PAT or registered-client browser sign-in | PAT or registered-client browser sign-in | Atlassian email and scoped API token | PAT |
| Review reads | PRs, checks, statuses and decisions | MRs and pipeline details | PRs, participant decisions and source-commit statuses | PRs, votes, policies and statuses |
| Create and review | Same/fork PRs, drafts, comments, inline, approve/request changes | Same/fork MRs, drafts, comments, inline and approval | Same/fork PRs, drafts, comments, inline, approve/request changes | Same/fork PRs within one organization, drafts, threads, inline, approve/reject |
| Merge | Expected-source-SHA direct merge and queue | Expected-source-SHA direct merge and train | Browser only; no atomic expected-head guard in the supported merge route | Expected-source-SHA merge/squash/rebase; policies enforced; accepted asynchronous completion is not reported as merged |
| Queue/train | Merge queue | Merge train | Unavailable | Unavailable |
| CI | Repository runs | Project pipelines | Current-branch Pipelines | Repository-and-branch Builds |
| Issues | Repository issues | Project issues | Unavailable; use the linked external tracker | Boards work items for the whole project |
| Releases | Repository releases | Project releases | Unavailable; use repository tags or Downloads | Classic releases for the whole project |

Browser sign-in currently uses device authorization and requires a registered
public client ID on the exact provider host. No client secret is bundled. See
[Browser sign-in](../browser-sign-in.md) for cancellation, refresh, vault limits
and the untested live-account boundary. Existing PAT accounts remain supported.

Bitbucket and Azure review votes apply to a PR rather than an immutable commit.
The candidate checks source/target/state before and after review submission and
reports a changed review; provider policies should reset approvals after pushes.
Comments and votes are submitted separately. No provider write is retried
automatically.

Checkout on Bitbucket/Azure fetches the exact resolved source commit and rechecks
the displayed review before fetch and checkout. A fork commit must be fetchable
from the selected remote; otherwise checkout fails safely without choosing
another remote. Azure fork creation stays within one organization. See the
[detailed cloud provider limits](../provider-support.md).

## Browser sign-in

GitHub and GitLab support device authorization from **Accounts & SSH**. Register
an application with the provider, enter its public client ID, then start sign-in.
Recorde displays a short code and opens the provider's verification page. Check
the account and requested permissions there. After authorization, Recorde shows
the verified account; choose **Save** to store it or **Discard** to cancel.

Client IDs are public configuration. Access and refresh tokens stay in the OS
credential vault. Recorde does not embed or request a client secret. Existing
personal access token accounts remain supported. Refresh failures require
signing in again; Recorde does not silently switch accounts.

For a GitHub OAuth application:

1. Open [New OAuth application](https://github.com/settings/applications/new)
   while signed in to the account that should own it.
2. Use **Recorde** as the name and the project's repository URL as its homepage.
3. GitHub requires a callback URL even for device authorization. A loopback URL
   such as `http://127.0.0.1:42813/oauth/callback` can fill that registration field;
   this device flow does not start a callback listener.
4. Enable **Device Flow** and copy the public **Client ID** into Recorde. Do not
   copy a client secret into Recorde, source control or support messages.

GitLab requires a registered public application and a server that supports its
device authorization endpoints. Recorde requests `api`, `read_user` and
`write_repository`; GitHub requests `repo`, `read:user`, `workflow` and
`offline_access`. GitHub refuses HTTPS pushes that change `.github/workflows`
files without `workflow`; accounts signed in before this scope was added must
sign in again. Classic personal access tokens need `repo` and `workflow` for the
same pushes; fine-grained tokens need Contents and Workflows write permission.
Self-hosted services use their own application registration and exact host/port.
Bitbucket and Azure currently use token accounts rather than browser sign-in.

Registration does not create accounts on other providers. Live acceptance of
these flows requires the relevant account and a registered application; mocked
protocol tests do not establish that a particular enterprise deployment enables
device authorization.

The account model stores provider kind, host, and username. Tokens are retrieved
from the OS credential vault when provider panels or native Git HTTPS operations
need them. Multiple accounts on a host require an explicit repository selection
or a matching HTTPS URL account username. Canonical transport placeholders
(`x-access-token` for GitHub and `oauth2` for GitLab) do not select an arbitrary
profile. Bitbucket clone usernames are protocol hints: a binding or one
unambiguous saved email identity selects the account; Git uses
`x-bitbucket-api-token-auth`. Hostnames and ports match exactly. Account selection and
provider calls run in background tasks, and API requests never forward tokens
through redirects. See [Configuration](Configuration.md) for migration and the
shared Git credential-helper behavior.

### SSH public-key upload permissions

GitLab public-key uploads use Bearer authentication, supporting both browser OAuth
and personal tokens with `api` scope. GitHub's existing browser sign-in scopes do
not request SSH-key write access. Use Copy and the provider's SSH keys page button,
or supply a classic PAT with `write:public_key` or a fine-grained token with Git SSH
keys write permission. Upload refusals explain this fallback; Recorde does not
expand OAuth scopes or retry writes automatically. These are provider permission
requirements, not evidence of live-account acceptance. See
[GitLab API authentication](https://docs.gitlab.com/api/rest/authentication/) and
[GitHub SSH key permissions](https://docs.github.com/en/rest/users/keys).

## Remote Parsing

Provider detection starts from the Git remote URL. Supported forms include:

```text
git@github.com:owner/repo
https://github.com/owner/repo
https://gitlab.com/group/project
ssh://git@gitlab.com/group/subgroup/project.git
https://bitbucket.org/workspace/repository.git
https://dev.azure.com/organization/project/_git/repository
git@ssh.dev.azure.com:v3/organization/project/repository
```

The parser extracts owner or namespace, repository/project name, and host.

## Common review models

Recorde models supported pull requests and merge requests as `PrItem`:

- Number.
- Title.
- Author.
- State.
- Head and base refs.
- URL.
- Last updated timestamp.
- Labels.
- Optional check state.
- Optional review state.
- Body.
- Comment count.
- Optional changed file count.

Filters support open, closed, and all.

## Issues

Issues are represented as provider-neutral `IssueItem` values with number, title,
author, state, URL, labels, body, comment count, and update timestamp.

Filters support open, closed, and all.

## CI Runs

CI runs are represented as `CiRun`:

- ID.
- Name.
- Status.
- URL.
- Branch.
- Last updated timestamp.

Statuses are normalized to success, failure, running, pending, or cancelled.

## Releases

Releases are represented as `ReleaseItem`:

- Tag name.
- Release name.
- Body.
- URL.
- Published timestamp.
- Prerelease flag.

## Implementation Notes

The provider modules use `ureq` and `serde_json` directly. Timestamps are parsed
from ISO-8601 strings into Unix timestamps by the shared provider module.

Provider loading is initiated from `git/operations.rs`, which checks the current
repository remote, finds a matching account/token, calls the relevant adapter,
and updates `AppState`.



## GitHub and GitLab read and checkout behavior

Repository Settings selects the provider remote independently from the push
remote. Lists follow GitHub Link and GitLab pagination headers, bounded to
20 pages/2,000 items/90 seconds. Limits or invalid links produce a visible error.
Tokens never follow pagination to another origin or endpoint.

Closed includes merged reviews; GitLab All really requests all states. Closed
drafts retain their terminal state. Selecting a review loads details: GitHub
check runs, commit statuses and review decisions, or GitLab head-pipeline status.
Missing endpoint permissions are reported rather than presented as success.
These summaries do not calculate branch-protection merge eligibility.

Checkout Review fetches `refs/pull/<number>/head` on GitHub or
`refs/merge-requests/<number>/head` on GitLab, verifies the displayed head OID,
and safely creates a local review branch. Fork branch names are not assumed to
exist locally. Changed review heads and local branch commits are not overwritten.

## GitHub and GitLab create, review and merge

The PR/MR window includes creation, review and merge forms. Each form captures
its repository, remote and account. Review the populated action, then confirm
submission. Changing tabs does not retarget an open form. Create/review inputs
survive errors, closing and restart. Fresh commit guards are loaded on reopening.
Confirmation and merge actions are never restored automatically.

- Create a PR/MR from an already-pushed source branch; drafts are supported.
- GitHub supports comment, approve and request-changes reviews tied to the
  displayed commit. GitLab supports comments or commit-pinned approval.
- GitHub offers merge, squash and rebase; GitLab uses the project's strategy
  with optional squash. Source branches are retained. Direct merge requests
  include the expected source SHA; provider permissions and branch rules apply.

Before review/merge, Recorde fetches the current review and rejects changed
source commits, target branches, closed reviews or draft merges. It rechecks
remote/account selection before submission. Cached reviews retain their source
remote URL; a changed URL requires refreshing the list before details, checkout
or form preparation. Requests never follow redirects or
retry automatically. Permission/rate-limit failures are shown; an ambiguous
network result asks users to refresh before manually retrying.

Source/target projects can be selected from same-host configured remotes or entered
as project paths. GitHub fork creation uses the source owner and repository;
GitLab creation resolves the target project ID. Inline comments use native diff
paths/line numbers and validate the current head; GitLab also uses the latest diff
version's base/start/head SHAs.

Merge forms support GitHub merge queues and GitLab merge trains. Queue submission
is pinned to the expected source SHA, requires an accepted queue/train entry,
and never falls back to a direct merge. GitHub's repository configuration chooses
the queued merge strategy. Opt-in 30-second selected-review refresh updates checks,
reviews and merge outcome while focused; API errors pause polling. Use **Refresh
queue / train status** to load its current state and position. GitHub supplies
the position; GitLab positions are computed from the complete active train sorted
by ID for the target branch. Later selected-review refreshes also refresh that
queue status. Queue APIs are queried only after requesting them explicitly.

Local mock-server tests cover forks, inline coordinates, queue payloads, stale
heads, permission/rate-limit responses and redirect refusal. No live-provider
mutation acceptance has been run.

API references: [GitHub pull requests](https://docs.github.com/en/rest/pulls/pulls),
[GitHub checks](https://docs.github.com/en/rest/checks/runs),
[GitHub reviews](https://docs.github.com/en/rest/pulls/reviews),
[GitLab merge requests](https://docs.gitlab.com/api/merge_requests/),
[GitHub local review checkout](https://docs.github.com/en/pull-requests/how-tos/review-pull-requests/checking-out-pull-requests-locally),
and [GitLab review refs](https://docs.gitlab.com/user/project/merge_requests/merge_request_troubleshooting/#check-out-merge-requests-locally-through-the-head-ref).

Write API references: [GitLab approvals](https://docs.gitlab.com/api/merge_request_approvals/) and [GitLab notes](https://docs.gitlab.com/api/notes/).

### Saved form drafts

Create and review text is saved atomically under the worktree Git directory in
`recorde/provider-drafts/`. Drafts are scoped to the remote, account and review
number, contain no credentials, and are removed after confirmed success. Reopening
a review retains the freshly loaded head/target guards. Confirmation and merge
actions are never restored or submitted automatically. Drafts over 256 KiB show
a save error. Closing a form retains its draft.

Additional API references: [GitHub inline comments](https://docs.github.com/en/rest/pulls/comments),
[GitLab diff discussions](https://docs.gitlab.com/api/discussions/),
[GitHub queue mutations](https://docs.github.com/en/graphql/reference/mutations#enqueuepullrequest),
and [GitLab merge trains](https://docs.gitlab.com/api/merge_trains/).
