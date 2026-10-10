//! Explicit provider mutations. No retries: a lost response can hide a successful write.
use crate::git::capabilities::ProviderType;
use serde_json::{json, Value};
use std::io::Read;
use std::path::PathBuf;

#[cfg(test)]
#[path = "write_tests.rs"]
mod tests;

#[derive(Clone)]
pub struct Context {
    pub projects: Vec<String>,
    pub path: PathBuf,
    pub remote: String,
    pub remote_url: String,
    pub host: String,
    pub owner: String,
    pub project: String,
    pub provider: ProviderType,
    pub account: String,
}

#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Decision {
    Comment,
    Approve,
    RequestChanges,
}
#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MergeMethod {
    Merge,
    Squash,
    Rebase,
}

#[derive(Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InlineComment {
    pub path: String,
    pub old_path: String,
    pub new_line: u32,
    pub old_line: u32,
}
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Action {
    Create {
        #[serde(default)]
        source_project: String,
        #[serde(default)]
        target_project: String,
        title: String,
        body: String,
        source: String,
        target: String,
        draft: bool,
    },
    Review {
        number: u32,
        head: String,
        target: String,
        body: String,
        decision: Decision,
        #[serde(default)]
        inline: Option<InlineComment>,
    },
    Merge {
        number: u32,
        head: String,
        target: String,
        method: MergeMethod,
        #[serde(default)]
        queue: bool,
    },
}

pub fn prepare(
    path: PathBuf,
    requested_remote: Option<(String, String)>,
) -> Result<Context, String> {
    let repo = git2::Repository::open(&path).map_err(|e| e.to_string())?;
    let remote = match requested_remote {
        Some((remote, expected_url)) => {
            crate::git::reviews::verify_remote(&path, &remote, &expected_url)?;
            remote
        }
        None => crate::git::remote::provider_remote(&repo)?,
    };
    let remote_url = repo
        .find_remote(&remote)
        .map_err(|e| e.to_string())?
        .url()
        .ok()
        .ok_or("Remote has no URL.")?
        .to_string();
    let (owner, project, host, _, provider) =
        crate::git::operations::resolve_repo_info_for_remote(&path, &remote)?;
    let (account, _) = crate::auth::store::lookup_for_api(&remote_url, &path)?
        .ok_or("No account selected for this provider.")?;
    if !matches!(
        provider,
        ProviderType::GitHub
            | ProviderType::GitLab
            | ProviderType::Bitbucket
            | ProviderType::AzureDevOps
    ) {
        return Err("This provider does not support in-app writes.".into());
    }
    let mut projects = vec![format!("{owner}/{project}")];
    if let Ok(remotes) = repo.remotes() {
        for name in remotes.iter().flatten().flatten() {
            if let Some(endpoint) = repo.find_remote(name).ok().and_then(|r| {
                r.url()
                    .ok()
                    .and_then(|url| crate::auth::endpoint::parse(url, true).ok())
            }) {
                if endpoint.host == host {
                    projects.push(format!("{}/{}", endpoint.namespace, endpoint.project));
                }
            }
        }
    }
    projects.sort();
    projects.dedup();
    Ok(Context {
        projects,
        path,
        remote,
        remote_url,
        host,
        owner,
        project,
        provider,
        account,
    })
}

pub fn execute(context: &Context, action: &Action) -> Result<String, String> {
    let repo = git2::Repository::open(&context.path).map_err(|e| e.to_string())?;
    if repo
        .find_remote(&context.remote)
        .ok()
        .and_then(|r| r.url().ok().map(str::to_owned))
        .as_deref()
        != Some(&context.remote_url)
    {
        return Err("Remote changed. Reopen this form before submitting.".into());
    }
    let (account, token) = crate::auth::store::lookup_for_api(&context.remote_url, &context.path)?
        .ok_or("No account selected for this provider.")?;
    if account != context.account {
        return Err("Selected account changed. Reopen this form before submitting.".into());
    }
    let (owner_now, project_now, host_now, _, provider_now) =
        crate::git::operations::resolve_repo_info_for_remote(&context.path, &context.remote)?;
    if provider_now != context.provider
        || host_now != context.host
        || owner_now != context.owner
        || project_now != context.project
    {
        return Err(
            "Provider destination or account type changed. Reopen this form before submitting."
                .into(),
        );
    }
    let component = super::pagination::component;
    validate(action, &context.provider)?;
    let project_path = match action {
        Action::Create {
            source_project,
            target_project,
            ..
        } => {
            let selected = if matches!(
                context.provider,
                ProviderType::GitHub | ProviderType::Bitbucket | ProviderType::AzureDevOps
            ) {
                target_project
            } else {
                source_project
            };
            if selected.is_empty() {
                format!("{}/{}", context.owner, context.project)
            } else {
                selected.clone()
            }
        }
        _ => format!("{}/{}", context.owner, context.project),
    };
    let (owner, project) = project_path
        .rsplit_once('/')
        .ok_or("Select a project in owner/repository form.")?;
    let base = match context.provider {
        ProviderType::GitHub => {
            let api = if context.host == "github.com" {
                "https://api.github.com".into()
            } else {
                format!("https://{}/api/v3", context.host)
            };
            format!("{api}/repos/{}/{}", component(owner), component(project))
        }
        ProviderType::GitLab => format!(
            "https://{}/api/v4/projects/{}",
            context.host,
            component(&project_path)
        ),
        ProviderType::Bitbucket => super::bitbucket::base(owner, project),
        ProviderType::AzureDevOps => super::azure::base(owner, project)?,
        _ => return Err("Unsupported provider.".into()),
    };
    match context.provider {
        ProviderType::Bitbucket => {
            super::bitbucket::execute(&base, &super::bitbucket::basic(&account, &token), action)
        }
        ProviderType::AzureDevOps => {
            super::azure::execute(&base, &super::bitbucket::basic("", &token), action)
        }
        _ => execute_at(&base, &context.provider, &token, action),
    }
}

pub(crate) fn validate(action: &Action, provider: &ProviderType) -> Result<(), String> {
    if matches!(
        provider,
        ProviderType::Bitbucket | ProviderType::AzureDevOps
    ) {
        match action {
            Action::Merge { .. } if *provider == ProviderType::Bitbucket => return Err("Bitbucket Cloud cannot atomically guard the source commit on merge. Open the pull request in Bitbucket to merge.".into()),
            Action::Merge { queue: true, .. } => return Err("Merge queues are unavailable for this provider.".into()),
            Action::Review { decision, body, .. } if *decision != Decision::Comment && !body.trim().is_empty() => return Err("Submit comments separately from review votes.".into()),
            Action::Create { source_project, target_project, body, .. } if *provider == ProviderType::AzureDevOps => {
                if source_project.split('/').next() != target_project.split('/').next() { return Err("Azure fork pull requests must stay within one organization.".into()); }
                if body.chars().count() > 4000 { return Err("Azure descriptions must not exceed 4000 characters.".into()); }
            }
            _ => {}
        }
    }
    match action {
        Action::Create {
            source_project,
            target_project,
            title,
            body,
            source,
            target,
            ..
        } => {
            if title.trim().is_empty() || source.trim().is_empty() || target.trim().is_empty() {
                return Err("Title, source branch and target branch are required.".into());
            }
            if source == target && source_project == target_project {
                return Err("Choose different source and target branches.".into());
            }
            if title.len() > 255 || body.len() > 60_000 {
                return Err("Title or description is too long.".into());
            }
            if !source_project.is_empty() || !target_project.is_empty() {
                for project in [source_project, target_project] {
                    let parts: Vec<_> = project.split('/').collect();
                    if project.len() > 512
                        || parts.len() < 2
                        || (matches!(provider, ProviderType::GitHub | ProviderType::Bitbucket)
                            && parts.len() != 2)
                        || (*provider == ProviderType::AzureDevOps && parts.len() != 3)
                        || parts.iter().any(|p| {
                            p.is_empty()
                                || *p == "."
                                || *p == ".."
                                || p.chars().any(|c| {
                                    c.is_control()
                                        || (*provider != ProviderType::AzureDevOps
                                            && c.is_whitespace())
                                        || matches!(c, ':' | '\\' | '?' | '#')
                                })
                        })
                    {
                        return Err(
                            "Use owner/repository project names on the selected provider host."
                                .into(),
                        );
                    }
                }
                if source.contains(':') {
                    return Err("Enter the branch name without an owner prefix; the source project is selected separately.".into());
                }
            }
        }
        Action::Review {
            number,
            head,
            target,
            body,
            decision,
            inline,
        } => {
            validate_identity(*number, head, target)?;
            if (*decision == Decision::Comment
                || (*provider == ProviderType::GitHub && *decision == Decision::RequestChanges))
                && body.trim().is_empty()
            {
                return Err("Enter the review text.".into());
            }
            if body.len() > 60_000 {
                return Err("Review text is too long.".into());
            }
            if let Some(comment) = inline {
                if *decision != Decision::Comment
                    || body.trim().is_empty()
                    || (comment.new_line == 0 && comment.old_line == 0)
                {
                    return Err(
                        "Inline comments need text and a line number; submit approvals separately."
                            .into(),
                    );
                }
                crate::git::safe_path::relative(&comment.path)?;
                if !comment.old_path.is_empty() {
                    crate::git::safe_path::relative(&comment.old_path)?;
                }
            }
            if *provider == ProviderType::GitLab
                && (*decision == Decision::RequestChanges
                    || (*decision == Decision::Approve && !body.trim().is_empty()))
            {
                return Err("For GitLab, submit a comment separately from approval. Request-changes reviews are not supported here.".into());
            }
        }
        Action::Merge {
            number,
            head,
            target,
            method,
            ..
        } => {
            validate_identity(*number, head, target)?;
            if *provider == ProviderType::GitLab && *method == MergeMethod::Rebase {
                return Err(
                    "GitLab uses the project's merge strategy; choose Merge or Squash.".into(),
                );
            }
        }
    }
    Ok(())
}

fn validate_identity(number: u32, head: &str, target: &str) -> Result<(), String> {
    if number == 0
        || target.is_empty()
        || head.len() != 40
        || !head.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(
            "Refresh review details to obtain its current commit and target branch.".into(),
        );
    }
    Ok(())
}

fn execute_at(
    base: &str,
    provider: &ProviderType,
    token: &str,
    action: &Action,
) -> Result<String, String> {
    validate(action, provider)?;
    let github = *provider == ProviderType::GitHub;
    let collection = if github { "pulls" } else { "merge_requests" };
    let root = format!("{base}/{collection}");
    let auth = format!("Bearer {token}");
    let header = "Authorization";
    let mut inline_position = None;
    if let Action::Review {
        number,
        head,
        target,
        ..
    }
    | Action::Merge {
        number,
        head,
        target,
        ..
    } = action
    {
        let current = send(
            "GET",
            &format!("{root}/{number}"),
            header,
            &auth,
            token,
            None,
        )?;
        let current_head = if github {
            current["head"]["sha"].as_str()
        } else {
            current["sha"].as_str()
        };
        let current_target = if github {
            current["base"]["ref"].as_str()
        } else {
            current["target_branch"].as_str()
        };
        if current_head != Some(head.as_str()) || current_target != Some(target.as_str()) {
            return Err("Review head or target branch changed. Refresh and review the updated changes before submitting.".into());
        }
        if !matches!(current["state"].as_str(), Some("open" | "opened")) {
            return Err("This review is no longer open.".into());
        }
        if let Action::Review {
            inline: Some(comment),
            ..
        } = action
        {
            if !github {
                let versions = send(
                    "GET",
                    &format!("{root}/{number}/versions?per_page=1"),
                    header,
                    &auth,
                    token,
                    None,
                )?;
                let version = &versions[0];
                if version["head_commit_sha"].as_str() != Some(head.as_str()) {
                    return Err(
                        "Diff version changed or is unavailable. Refresh before commenting.".into(),
                    );
                }
                let base = version["base_commit_sha"]
                    .as_str()
                    .ok_or("Diff base is unavailable.")?;
                let start = version["start_commit_sha"]
                    .as_str()
                    .ok_or("Diff target is unavailable.")?;
                validate_identity(*number, base, target)?;
                validate_identity(*number, start, target)?;
                let mut position = json!({"position_type":"text", "base_sha":base, "start_sha":start, "head_sha":head,
                    "new_path":comment.path, "old_path":if comment.old_path.is_empty() { &comment.path } else { &comment.old_path }});
                if comment.new_line > 0 {
                    position["new_line"] = json!(comment.new_line);
                }
                if comment.old_line > 0 {
                    position["old_line"] = json!(comment.old_line);
                }
                inline_position = Some(position);
            }
        }
        if matches!(action, Action::Merge { .. })
            && (current["draft"].as_bool() == Some(true)
                || current["work_in_progress"].as_bool() == Some(true))
        {
            return Err("Mark this draft ready for review before merging.".into());
        }
        if let Action::Merge {
            queue: true,
            method,
            ..
        } = action
        {
            if github {
                let node = current["node_id"]
                    .as_str()
                    .ok_or("Provider did not return the pull request identifier.")?;
                let api = base
                    .split_once("/repos/")
                    .map(|(api, _)| api)
                    .unwrap_or(base);
                let graphql = format!("{}/graphql", api.strip_suffix("/v3").unwrap_or(api));
                let response = send(
                    "POST",
                    &graphql,
                    header,
                    &auth,
                    token,
                    Some(json!({
                        "query":"mutation($input: EnqueuePullRequestInput!) { enqueuePullRequest(input: $input) { mergeQueueEntry { id } } }",
                        "variables":{"input":{"pullRequestId":node,"expectedHeadOid":head}}
                    })),
                )?;
                if response["errors"]
                    .as_array()
                    .is_some_and(|errors| !errors.is_empty())
                {
                    return Err("Provider rejected the queue request. Check queue availability and permissions, then refresh before retrying.".into());
                }
                if response["data"]["enqueuePullRequest"]["mergeQueueEntry"]["id"]
                    .as_str()
                    .is_none()
                {
                    return Err(
                        "No confirmed queue entry. Refresh the provider before retrying.".into(),
                    );
                }
            } else {
                let response = send(
                    "POST",
                    &format!("{base}/merge_trains/merge_requests/{number}"),
                    header,
                    &auth,
                    token,
                    Some(
                        json!({"sha":head,"squash":*method == MergeMethod::Squash,"auto_merge":false}),
                    ),
                )?;
                if response["id"].as_u64().is_none() {
                    return Err(
                        "No confirmed merge-train entry. Refresh the provider before retrying."
                            .into(),
                    );
                }
            }
            return Ok("Added to the provider merge queue. Refresh review details to follow checks and completion.".into());
        }
    }
    let (method, url, body) = match action {
        Action::Review {
            number,
            head,
            body,
            inline: Some(comment),
            ..
        } if github => (
            "POST",
            format!("{root}/{number}/comments"),
            json!({"commit_id":head,"body":body,"path":comment.path,
                "line":if comment.new_line > 0 { comment.new_line } else { comment.old_line },
                "side":if comment.new_line > 0 { "RIGHT" } else { "LEFT" }}),
        ),
        Action::Review {
            number,
            body,
            inline: Some(_),
            ..
        } => (
            "POST",
            format!("{root}/{number}/discussions"),
            json!({"body":body,"position":inline_position}),
        ),
        Action::Create {
            source_project,
            title,
            body,
            source,
            target,
            draft,
            ..
        } if github => (
            "POST",
            root,
            if let Some((owner, project)) = source_project.split_once('/') {
                json!({"title":title,"body":body,"head":format!("{owner}:{source}"),"head_repo":project,"base":target,"draft":draft})
            } else {
                json!({"title":title,"body":body,"head":source,"base":target,"draft":draft})
            },
        ),
        Action::Create {
            source_project,
            target_project,
            title,
            body,
            source,
            target,
            draft,
        } => {
            let title = if *draft && !title.to_lowercase().starts_with("draft:") {
                format!("Draft: {title}")
            } else {
                title.clone()
            };
            let mut body = json!({"title":title,"description":body,"source_branch":source,"target_branch":target});
            if !target_project.is_empty() && target_project != source_project {
                let projects = base
                    .rsplit_once('/')
                    .ok_or("Invalid provider project endpoint.")?
                    .0;
                let project = send(
                    "GET",
                    &format!(
                        "{projects}/{}",
                        super::pagination::component(target_project)
                    ),
                    header,
                    &auth,
                    token,
                    None,
                )?;
                let id = project["id"]
                    .as_u64()
                    .filter(|id| *id > 0)
                    .ok_or("The selected target project is unavailable.")?;
                body["target_project_id"] = json!(id);
            }
            ("POST", root, body)
        }
        Action::Review {
            number,
            head,
            body,
            decision,
            ..
        } if github => {
            let event = match decision {
                Decision::Comment => "COMMENT",
                Decision::Approve => "APPROVE",
                Decision::RequestChanges => "REQUEST_CHANGES",
            };
            (
                "POST",
                format!("{root}/{number}/reviews"),
                json!({"commit_id":head,"body":body,"event":event}),
            )
        }
        Action::Review {
            number,
            head,
            decision: Decision::Approve,
            ..
        } => (
            "POST",
            format!("{root}/{number}/approve"),
            json!({"sha":head}),
        ),
        Action::Review { number, body, .. } => (
            "POST",
            format!("{root}/{number}/notes"),
            json!({"body":body}),
        ),
        Action::Merge {
            number,
            head,
            method,
            ..
        } if github => {
            let method = match method {
                MergeMethod::Merge => "merge",
                MergeMethod::Squash => "squash",
                MergeMethod::Rebase => "rebase",
            };
            (
                "PUT",
                format!("{root}/{number}/merge"),
                json!({"sha":head,"merge_method":method}),
            )
        }
        Action::Merge {
            number,
            head,
            method,
            ..
        } => (
            "PUT",
            format!("{root}/{number}/merge"),
            json!({"sha":head,"squash":*method == MergeMethod::Squash,"auto_merge":false,"should_remove_source_branch":false}),
        ),
    };
    let response = send(method, &url, header, &auth, token, Some(body))?;
    match action {
        Action::Create { .. } => {
            let number = response[if github { "number" } else { "iid" }].as_u64().ok_or("Provider accepted the request but returned no review number. Refresh before retrying.")?;
            Ok(format!("Created review #{number}."))
        }
        Action::Review { .. } => Ok("Review submitted.".into()),
        Action::Merge { .. } if github && response["merged"].as_bool() != Some(true) => {
            Err("Provider did not confirm a merge. Refresh to inspect its current state.".into())
        }
        Action::Merge { .. } if !github && response["state"].as_str() != Some("merged") => {
            Ok("Merge request accepted. Refresh to check whether it has merged.".into())
        }
        Action::Merge { .. } => Ok("Merged successfully.".into()),
    }
}

pub(super) fn send(
    method: &str,
    url: &str,
    header: &str,
    auth: &str,
    token: &str,
    body: Option<Value>,
) -> Result<Value, String> {
    let agent = ureq::AgentBuilder::new()
        .redirects(0)
        .timeout(std::time::Duration::from_secs(30))
        .build();
    let request = agent
        .request(method, url)
        .set(header, auth)
        .set("User-Agent", "Recorde")
        .set("Accept", "application/json");
    let result = match body {
        Some(body) => request.send_json(body),
        None => request.call(),
    };
    let response = match result {
        Ok(response) | Err(ureq::Error::Status(_, response)) => response,
        Err(_) => return Err("Connection ended without a confirmed result. Refresh the provider before retrying; the action may have succeeded.".into()),
    };
    let status = response.status();
    let retry = response
        .header("Retry-After")
        .unwrap_or_default()
        .to_string();
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| {
            "Response interrupted. Refresh before retrying; the action may have succeeded."
        })?;
    if bytes.len() > 1024 * 1024 {
        return Err(
            "Response too large. Refresh before retrying; the action may have succeeded.".into(),
        );
    }
    let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    if !(200..300).contains(&status) {
        let detail = value["message"]
            .as_str()
            .or_else(|| value["error"].as_str())
            .unwrap_or("");
        let hint = match status {
            401 => "Authentication failed; check the selected account.",
            403 => "Permission or rate limit denied the action; check token scopes and repository rules.",
            404 => "Resource unavailable; check repository access and the review number.",
            405 | 409 => "Review changed or cannot be merged; refresh and check branch rules.",
            422 => "Provider rejected the inputs or repository rules.",
            429 => "Rate limited; wait before submitting again.",
            300..=399 => "Provider redirected the request; no redirect was followed.",
            _ => "Provider could not complete the action. Refresh before retrying.",
        };
        let detail: String = detail.chars().take(500).collect();
        let message = format!(
            "HTTP {status}: {hint} {detail} {}",
            if retry.is_empty() {
                String::new()
            } else {
                format!("Retry-After: {retry}")
            }
        );
        return Err(crate::auth::redact::text(
            &message.replace(token, "[redacted]"),
        ));
    }
    if value.is_null() {
        return Err("Provider response was incomplete. Refresh before retrying; the action may have succeeded.".into());
    }
    Ok(value)
}
