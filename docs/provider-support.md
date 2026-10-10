# Provider support

Recorde supports GitHub, GitLab, Bitbucket Cloud and Azure DevOps Services. Select the provider account and remote for the repository before opening a review form. A submitted form checks that the remote URL, account name and provider destination still match.

## Cloud provider capabilities

| Feature | Bitbucket Cloud | Azure DevOps Services |
| --- | --- | --- |
| Authentication | Atlassian email + scoped API token, HTTP Basic | Personal access token, HTTP Basic |
| Pull requests | Open/closed/all, drafts, details, participant decisions and commit statuses | Open/closed/all, drafts, details, votes, policy evaluations and PR statuses |
| Checkout | Fetch exact source commit; recheck PR head before fetch and checkout | Fetch exact source commit; recheck PR head before fetch and checkout |
| Create | Same repository or compatible fork, including drafts | Same repository or compatible fork in the same organization, including drafts |
| Review | Comments, inline comments, approve, request changes | Comment threads, iteration-bound inline comments, approve, reject |
| Merge | Complete in Bitbucket's browser UI | Merge commit, squash or rebase, with expected source commit; policies enforced |
| CI | Bitbucket Pipelines for the current branch | Azure Builds for this repository and branch |
| Issues | Native issue tracker retired upstream in August 2026 | Azure Boards work items for the whole project, using process state categories |
| Releases | No native releases API; use tags or Downloads in Bitbucket | Classic releases for the whole project |
| Merge queues | Not exposed | Not exposed |

Bitbucket's merge endpoint has no documented atomic expected-head parameter. Recorde disables in-app merge instead of allowing the source to change between a preflight read and a merge request. Approvals and rejection votes on both providers apply to the pull request, rather than an immutable commit. Recorde verifies the displayed source and target immediately before and after sending a review and reports a race if either changes. Enable provider policies to reset approvals after new pushes. No automatic write retries are performed.

Checkout requires the server to allow fetching the review's source commit from the selected remote. If a fork commit is inaccessible from that remote, fetching fails without switching the working tree. Add an authorized source remote or use the provider's checkout instructions in that case.

Azure HTTPS (`dev.azure.com/org/project/_git/repository`), legacy `org.visualstudio.com`, and `git@ssh.dev.azure.com:v3/org/project/repository` URLs resolve to the canonical account host `dev.azure.com`. Project names containing spaces are supported. The organization placeholder in Azure clone URL user information is not treated as an account name. Other hosts and lookalike suffixes are not mapped to cloud accounts. Azure DevOps Server and Bitbucket Data Center are not covered by these cloud adapters.

All HTTP requests disable redirects. Pagination validates the exact origin and path before sending credentials, detects loops and is limited to 20 pages, 2000 items, 8 MiB per response and 90 seconds per list. Azure Boards work item queries cap their result at 2000 items and fail explicitly if the project exceeds that bound. Secrets and encoded Basic authentication headers are registered for log redaction.

## Official API references

- [Bitbucket authentication](https://developer.atlassian.com/cloud/bitbucket/rest/): scoped API tokens use Atlassian email and Basic authentication.
- [Bitbucket pull requests](https://developer.atlassian.com/cloud/bitbucket/rest/api-group-pullrequests/): create, drafts, comments, votes, statuses and merge parameters.
- [Bitbucket Pipelines](https://developer.atlassian.com/cloud/bitbucket/rest/api-group-pipelines/).
- [Bitbucket changelog](https://developer.atlassian.com/cloud/bitbucket/changelog/): issue tracker endpoints removed on 24 August 2026.
- [Azure pull requests](https://learn.microsoft.com/en-us/rest/api/azure/devops/git/pull-requests?view=azure-devops-rest-7.1), [completion](https://learn.microsoft.com/en-us/rest/api/azure/devops/git/pull-requests/update?view=azure-devops-rest-7.1), [review votes](https://learn.microsoft.com/en-us/rest/api/azure/devops/git/pull-request-reviewers/create-pull-request-reviewer?view=azure-devops-rest-7.1) and [comment threads](https://learn.microsoft.com/en-us/rest/api/azure/devops/git/pull-request-threads/create?view=azure-devops-rest-7.1).
- [Azure Builds](https://learn.microsoft.com/en-us/rest/api/azure/devops/build/builds/list?view=azure-devops-rest-7.1), [Boards WIQL](https://learn.microsoft.com/en-us/rest/api/azure/devops/wit/wiql/query-by-wiql?view=azure-devops-rest-7.1), [work item states](https://learn.microsoft.com/en-us/rest/api/azure/devops/wit/work-item-type-states/list?view=azure-devops-rest-7.1) and [classic releases](https://learn.microsoft.com/en-us/rest/api/azure/devops/release/releases/list?view=azure-devops-rest-7.1).
- [GitLab authentication](https://docs.gitlab.com/api/rest/authentication/): OAuth and personal access tokens both support `Authorization: Bearer`.

Validation uses local mock HTTP servers and disposable Git repositories. It does not send writes to hosted projects or access real accounts.
