//! Azure DevOps Services REST 7.1. `owner` is organization/project.
use super::pagination::{component, list, object};
use super::{
    CheckState, CiRun, CiStatus, IssueItem, IssueState, PrItem, PrState, ReleaseItem, ReviewState,
};
use serde_json::{json, Value};

pub(crate) fn project_base(owner: &str) -> Result<String, String> {
    let (org, project) = owner
        .split_once('/')
        .ok_or("Azure remote has no project.")?;
    if org.is_empty() || project.is_empty() || project.contains('/') {
        return Err("Invalid Azure organization/project.".into());
    }
    Ok(format!(
        "https://dev.azure.com/{}/{}",
        component(org),
        component(project)
    ))
}

pub(crate) fn base(owner: &str, repo: &str) -> Result<String, String> {
    Ok(format!(
        "{}/_apis/git/repositories/{}",
        project_base(owner)?,
        component(repo)
    ))
}

pub(crate) fn review_head(
    owner: &str,
    repo: &str,
    number: u32,
    auth: &str,
) -> Result<String, String> {
    let body = object(
        &format!(
            "{}/pullrequests/{number}?api-version=7.1",
            base(owner, repo)?
        ),
        "Authorization",
        auth,
    )?;
    live_source_head(&base(owner, repo)?, &body, auth)
}

// lastMergeSourceCommit describes the last test merge and may lag a push.
// Resolve the actual branch tip; never follow repository URLs supplied by a PR.
pub(super) fn live_source_head(base: &str, body: &Value, auth: &str) -> Result<String, String> {
    if body["status"] != "active" {
        if !matches!(body["status"].as_str(), Some("completed" | "abandoned")) {
            return Err("Invalid review state.".into());
        }
        let head = body["lastMergeSourceCommit"]["commitId"]
            .as_str()
            .ok_or("Missing historical source commit.")?;
        return full_oid(head);
    }
    let name = body["sourceRefName"]
        .as_str()
        .filter(|n| n.starts_with("refs/heads/") && n.len() > 11)
        .ok_or("Missing source branch.")?;
    let source = if !body["forkSource"].is_null() {
        let id = body["forkSource"]["repository"]["id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or("Missing source fork repository ID.")?;
        let project = base
            .split("/_apis/")
            .next()
            .ok_or("Invalid Azure API URL.")?;
        let organization = project
            .rsplit_once('/')
            .ok_or("Invalid Azure organization URL.")?
            .0;
        format!("{organization}/_apis/git/repositories/{}", component(id))
    } else {
        base.to_owned()
    };
    let refs = list(
        &format!(
            "{source}/refs?filter={}&api-version=7.1",
            component(&name[5..])
        ),
        "Authorization",
        auth,
        Some("value"),
    )?;
    let matching: Vec<_> = refs.iter().filter(|r| r["name"] == name).collect();
    if matching.len() != 1 {
        return Err("Source branch changed or is unavailable. Refresh details.".into());
    }
    full_oid(matching[0]["objectId"].as_str().unwrap_or_default())
}
fn full_oid(head: &str) -> Result<String, String> {
    if head.len() != 40 || !head.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Invalid full source commit.".into());
    }
    Ok(head.into())
}
fn verify_live(base: &str, body: &Value, auth: &str, head: &str) -> Result<(), String> {
    if live_source_head(base, body, auth)? != head {
        return Err(
            "Source branch changed since the displayed review. Refresh before submitting.".into(),
        );
    }
    Ok(())
}
fn canonical_pr_url(owner: &str, repo: &str, number: u32) -> Result<String, String> {
    Ok(format!(
        "{}/_git/{}/pullrequest/{number}",
        project_base(owner)?,
        component(repo)
    ))
}

pub fn list_prs(owner: &str, repo: &str, auth: &str, state: &str) -> Result<Vec<PrItem>, String> {
    let status = if state == "open" { "active" } else { "all" };
    let values = list(
        &format!(
            "{}/pullrequests?api-version=7.1&searchCriteria.status={status}&$top=100",
            base(owner, repo)?
        ),
        "Authorization",
        auth,
        Some("value"),
    )?;
    let items: Vec<_> = values
        .iter()
        .map(|v| {
            let mut item = parse_pr(v).ok_or("Invalid Azure pull request.".to_owned())?;
            item.url = canonical_pr_url(owner, repo, item.number)?;
            Ok::<_, String>(item)
        })
        .collect::<Result<_, _>>()?;
    Ok(items
        .into_iter()
        .filter(|pr| state != "closed" || matches!(pr.state, PrState::Closed | PrState::Merged))
        .collect())
}

pub(crate) fn parse_pr(v: &Value) -> Option<PrItem> {
    let state = match v["status"].as_str()? {
        "completed" => PrState::Merged,
        "abandoned" => PrState::Closed,
        "active" if v["isDraft"] == true => PrState::Draft,
        "active" => PrState::Open,
        _ => return None,
    };
    let votes: Vec<_> = v["reviewers"].as_array().into_iter().flatten().collect();
    let review_state = if votes.iter().any(|v| v["vote"].as_i64().unwrap_or(0) < 0) {
        Some(ReviewState::ChangesRequested)
    } else if votes
        .iter()
        .any(|v| v["isRequired"] == true && v["vote"].as_i64().unwrap_or(0) == 0)
    {
        Some(ReviewState::Pending)
    } else if votes.iter().any(|v| v["vote"].as_i64().unwrap_or(0) > 0) {
        Some(ReviewState::Approved)
    } else {
        None
    };
    let number = u32::try_from(v["pullRequestId"].as_u64()?).ok()?;
    Some(PrItem {
        queue_status: None,
        head_oid: v["lastMergeSourceCommit"]["commitId"]
            .as_str()
            .unwrap_or_default()
            .into(),
        number,
        title: v["title"].as_str()?.into(),
        author: v["createdBy"]["displayName"]
            .as_str()
            .unwrap_or("unknown")
            .into(),
        state,
        head_ref: branch(&v["sourceRefName"]),
        base_ref: branch(&v["targetRefName"]),
        url: format!(
            "{}/pullrequest/{number}",
            v["repository"]["webUrl"].as_str().unwrap_or_default()
        ),
        updated_ts: super::parse_iso8601(
            v["closedDate"]
                .as_str()
                .or(v["creationDate"].as_str())
                .unwrap_or_default(),
        ),
        label_names: v["labels"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|l| l["name"].as_str().map(str::to_owned))
            .collect(),
        check_state: None,
        review_state,
        body: v["description"].as_str().unwrap_or_default().into(),
        comments: 0,
        changed_files: None,
    })
}

fn branch(v: &Value) -> String {
    v.as_str()
        .unwrap_or_default()
        .strip_prefix("refs/heads/")
        .unwrap_or(v.as_str().unwrap_or_default())
        .into()
}

pub fn details(
    owner: &str,
    repo: &str,
    number: u32,
    auth: &str,
) -> Result<(PrItem, Vec<String>), String> {
    let root = format!("{}/pullrequests/{number}", base(owner, repo)?);
    let body = object(&format!("{root}?api-version=7.1"), "Authorization", auth)?;
    let mut item = parse_pr(&body).ok_or("Invalid Azure pull request details.")?;
    item.url = canonical_pr_url(owner, repo, number)?;
    item.head_oid = live_source_head(&base(owner, repo)?, &body, auth)?;
    let mut warnings = vec![];
    let merge_snapshot_current = body["lastMergeSourceCommit"]["commitId"] == item.head_oid;
    if !merge_snapshot_current {
        warnings.push(
            "Azure is still updating its test merge. Refresh before reviewing or merging.".into(),
        );
    }
    // Policy evaluations include required build validation and approval policies.
    let project_id = body["repository"]["project"]["id"]
        .as_str()
        .ok_or("Missing Azure project ID.")?;
    let artifact = component(&format!(
        "vstfs:///CodeReview/CodeReviewId/{project_id}/{number}"
    ));
    let policies = list(
        &format!(
            "{}/_apis/policy/evaluations?artifactId={artifact}&api-version=7.1-preview.1",
            project_base(owner)?
        ),
        "Authorization",
        auth,
        Some("value"),
    );
    let statuses = (|| -> Result<Vec<Value>, String> {
        let latest = if body["supportsIterations"] == true {
            let iterations = list(
                &format!("{root}/iterations?api-version=7.1"),
                "Authorization",
                auth,
                Some("value"),
            )?;
            let latest = iterations
                .iter()
                .max_by_key(|v| v["id"].as_u64().unwrap_or(0))
                .ok_or("Missing Azure review iteration.")?;
            if latest["sourceRefCommit"]["commitId"] != item.head_oid {
                return Err("Review head changed while loading checks. Refresh details.".into());
            }
            Some(latest["id"].as_u64().ok_or("Invalid Azure iteration ID.")?)
        } else {
            None
        };
        let statuses = list(
            &format!("{root}/statuses?api-version=7.1"),
            "Authorization",
            auth,
            Some("value"),
        )?;
        Ok(statuses
            .into_iter()
            .filter(|s| {
                s["iterationId"]
                    .as_u64()
                    .is_none_or(|id| id == 0 || Some(id) == latest)
            })
            .collect())
    })();
    match (policies, statuses) {
        (Ok(policies), Ok(statuses)) => {
            let mut states = vec![];
            for p in policies
                .iter()
                .filter(|p| p["configuration"]["isEnabled"] != false)
            {
                states.push(match p["status"].as_str() {
                    Some("approved" | "notApplicable") => CheckState::Success,
                    Some("rejected" | "broken") => CheckState::Failure,
                    _ => CheckState::Pending,
                });
            }
            // Status APIs can include earlier iterations. Only the newest value for each context counts.
            let mut latest = std::collections::HashMap::new();
            for s in &statuses {
                let key = (
                    s["context"]["genre"].as_str().unwrap_or_default(),
                    s["context"]["name"].as_str().unwrap_or_default(),
                );
                let id = s["id"].as_u64().unwrap_or(0);
                let entry = latest.entry(key).or_insert((0, s));
                if id >= entry.0 {
                    *entry = (id, s);
                }
            }
            for (_, s) in latest.values() {
                states.push(match s["state"].as_str() {
                    Some("succeeded" | "notApplicable") => CheckState::Success,
                    Some("failed" | "error") => CheckState::Failure,
                    _ => CheckState::Pending,
                });
            }
            item.check_state = if states.is_empty() {
                None
            } else if states.contains(&CheckState::Failure) {
                Some(CheckState::Failure)
            } else if states.contains(&CheckState::Pending) {
                Some(CheckState::Pending)
            } else {
                Some(CheckState::Success)
            };
        }
        (policies, statuses) => {
            for e in [policies.err(), statuses.err()].into_iter().flatten() {
                warnings.push(format!("Checks unavailable: {e}"));
            }
        }
    }
    warnings.push("Azure review votes apply to the pull request. Enable branch policies that reset votes after new pushes; source commits are verified before and after submission.".into());
    if !merge_snapshot_current {
        item.check_state = Some(CheckState::Pending);
    }
    Ok((item, warnings))
}

pub fn list_ci_runs(
    owner: &str,
    repo: &str,
    branch_name: &str,
    auth: &str,
) -> Result<Vec<CiRun>, String> {
    list_ci_from(
        &base(owner, repo)?,
        &project_base(owner)?,
        branch_name,
        auth,
    )
}
pub(super) fn list_ci_from(
    api: &str,
    project: &str,
    branch_name: &str,
    auth: &str,
) -> Result<Vec<CiRun>, String> {
    let repository = object(&format!("{}?api-version=7.1", api), "Authorization", auth)?;
    let id = repository["id"]
        .as_str()
        .ok_or("Missing Azure repository ID.")?;
    let values = list(&format!("{}/_apis/build/builds?api-version=7.1&repositoryId={}&repositoryType=TfsGit&branchName={}&$top=100", project, component(id), component(&format!("refs/heads/{branch_name}"))), "Authorization", auth, Some("value"))?;
    values
        .into_iter()
        .map(|v| {
            let status = match v["result"].as_str() {
                Some("succeeded") => CiStatus::Success,
                Some("failed" | "partiallySucceeded") => CiStatus::Failure,
                Some("canceled") => CiStatus::Cancelled,
                _ if v["status"] == "inProgress" => CiStatus::Running,
                _ => CiStatus::Pending,
            };
            Ok(CiRun {
                id: v["id"].as_u64().ok_or("Invalid Azure build.")?.to_string(),
                name: v["definition"]["name"].as_str().unwrap_or("Build").into(),
                status,
                url: v["_links"]["web"]["href"]
                    .as_str()
                    .unwrap_or_default()
                    .into(),
                branch: branch(&v["sourceBranch"]),
                updated_ts: super::parse_iso8601(
                    v["finishTime"]
                        .as_str()
                        .or(v["startTime"].as_str())
                        .or(v["queueTime"].as_str())
                        .unwrap_or_default(),
                ),
            })
        })
        .collect()
}

/// Azure Boards belongs to the project, rather than an individual Git repository.
pub fn list_issues(owner: &str, auth: &str, state: &str) -> Result<Vec<IssueItem>, String> {
    list_issues_from(&project_base(owner)?, auth, state)
}
pub(super) fn list_issues_from(
    root: &str,
    auth: &str,
    state: &str,
) -> Result<Vec<IssueItem>, String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(90);
    let send =
        |url: &str, body| super::write::send("POST", url, "Authorization", auth, auth, Some(body));
    let query = send(
        &format!("{root}/_apis/wit/wiql?api-version=7.1&$top=2001"),
        json!({"query":"SELECT [System.Id] FROM WorkItems WHERE [System.TeamProject] = @project ORDER BY [System.ChangedDate] DESC"}),
    )?;
    let refs = query["workItems"]
        .as_array()
        .ok_or("Invalid Azure Boards query response.")?;
    if refs.len() > 2000 {
        return Err(
            "Azure Boards project exceeds the 2000 item limit. Open Boards to narrow the query."
                .into(),
        );
    }
    let mut items = vec![];
    let mut categories = std::collections::HashMap::new();
    for chunk in refs.chunks(200) {
        if std::time::Instant::now() >= deadline {
            return Err(
                "Azure Boards query exceeded its time limit. Refresh or open Boards.".into(),
            );
        }
        let ids: Vec<_> = chunk.iter().filter_map(|v| v["id"].as_u64()).collect();
        let batch = send(
            &format!("{root}/_apis/wit/workitemsbatch?api-version=7.1"),
            json!({"ids":ids,"fields":["System.Title","System.State","System.WorkItemType","System.CreatedBy","System.ChangedDate","System.Description","System.Tags","System.CommentCount"]}),
        )?;
        for v in batch["value"]
            .as_array()
            .ok_or("Invalid Azure Boards batch response.")?
        {
            let fields = &v["fields"];
            let kind = fields["System.WorkItemType"]
                .as_str()
                .ok_or("Missing work item type.")?;
            if !categories.contains_key(kind) {
                if std::time::Instant::now() >= deadline {
                    return Err(
                        "Azure Boards query exceeded its time limit. Refresh or open Boards."
                            .into(),
                    );
                }
                if categories.len() >= 30 {
                    return Err(
                        "Too many Azure Boards work item types. Open Boards to narrow the query."
                            .into(),
                    );
                }
                let states = list(
                    &format!(
                        "{root}/_apis/wit/workitemtypes/{}/states?api-version=7.1",
                        component(kind)
                    ),
                    "Authorization",
                    auth,
                    Some("value"),
                )?;
                categories.insert(kind.to_string(), states);
            }
            let name = fields["System.State"]
                .as_str()
                .ok_or("Missing work item state.")?;
            let category = categories[kind]
                .iter()
                .find(|s| s["name"] == name)
                .and_then(|s| s["category"].as_str())
                .ok_or("Unknown Azure Boards state category.")?;
            let item_state = match category {
                "Completed" | "Removed" => IssueState::Closed,
                "Proposed" | "InProgress" | "Resolved" => IssueState::Open,
                _ => return Err("Unknown Azure Boards state category.".into()),
            };
            if (state == "open" && item_state != IssueState::Open)
                || (state == "closed" && item_state != IssueState::Closed)
            {
                continue;
            }
            let number = u32::try_from(v["id"].as_u64().ok_or("Missing work item ID.")?)
                .map_err(|_| "Invalid work item ID.")?;
            items.push(IssueItem {
                number,
                title: fields["System.Title"].as_str().unwrap_or_default().into(),
                author: fields["System.CreatedBy"]["displayName"]
                    .as_str()
                    .unwrap_or("unknown")
                    .into(),
                state: item_state,
                url: format!("{root}/_workitems/edit/{number}"),
                label_names: fields["System.Tags"]
                    .as_str()
                    .unwrap_or_default()
                    .split(';')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect(),
                body: fields["System.Description"]
                    .as_str()
                    .unwrap_or_default()
                    .into(),
                comments: fields["System.CommentCount"].as_u64().unwrap_or(0) as u32,
                updated_ts: super::parse_iso8601(
                    fields["System.ChangedDate"].as_str().unwrap_or_default(),
                ),
            });
        }
    }
    Ok(items)
}

pub fn list_releases(owner: &str, auth: &str) -> Result<Vec<ReleaseItem>, String> {
    list_releases_from(&release_base(owner)?, auth)
}
pub(super) fn release_base(owner: &str) -> Result<String, String> {
    Ok(project_base(owner)?.replacen("https://dev.azure.com/", "https://vsrm.dev.azure.com/", 1))
}
pub(super) fn list_releases_from(root: &str, auth: &str) -> Result<Vec<ReleaseItem>, String> {
    let values = list(
        &format!("{root}/_apis/release/releases?api-version=7.1&$top=100"),
        "Authorization",
        auth,
        Some("value"),
    )?;
    values
        .into_iter()
        .map(|v| {
            Ok(ReleaseItem {
                tag_name: format!(
                    "Release #{}",
                    v["id"].as_u64().ok_or("Invalid Azure release.")?
                ),
                name: v["name"].as_str().unwrap_or_default().into(),
                body: v["description"].as_str().unwrap_or_default().into(),
                url: v["_links"]["web"]["href"]
                    .as_str()
                    .unwrap_or_default()
                    .into(),
                published_ts: super::parse_iso8601(v["createdOn"].as_str().unwrap_or_default()),
                prerelease: v["status"] == "draft",
            })
        })
        .collect()
}

pub(crate) fn execute(
    base: &str,
    auth: &str,
    action: &super::write::Action,
) -> Result<String, String> {
    use super::write::{Action, Decision, MergeMethod};
    super::write::validate(action, &crate::git::capabilities::ProviderType::AzureDevOps)?;
    let send = |method, url: &str, body| {
        super::write::send(method, url, "Authorization", auth, auth, body)
    };
    let root = format!("{base}/pullrequests");
    if let Action::Create {
        source_project,
        target_project,
        title,
        body,
        source,
        target,
        draft,
    } = action
    {
        let mut payload = json!({"sourceRefName":format!("refs/heads/{source}"),"targetRefName":format!("refs/heads/{target}"),"title":title,"description":body,"isDraft":draft});
        if source_project != target_project {
            let source_url = fork_repository_url(base, source_project, target_project)?;
            let repository = send("GET", &format!("{source_url}?api-version=7.1"), None)?;
            let id = repository["id"]
                .as_str()
                .ok_or("Missing Azure fork repository ID.")?;
            payload["forkSource"] =
                json!({"name":format!("refs/heads/{source}"),"repository":{"id":id}});
        }
        let result = send("POST", &format!("{root}?api-version=7.1"), Some(payload))?;
        return Ok(format!(
            "Created pull request #{}.",
            result["pullRequestId"]
                .as_u64()
                .ok_or("No confirmed pull request number. Refresh before retrying.")?
        ));
    }
    let (number, head, target) = match action {
        Action::Review {
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
        } => (number, head, target),
        _ => unreachable!(),
    };
    let url = format!("{root}/{number}?api-version=7.1");
    let current = send("GET", &url, None)?;
    verify(&current, head, target)?;
    verify_live(base, &current, auth, head)?;
    match action {
        Action::Merge { method, queue, .. } => {
            if *queue {
                return Err("Azure merge queues are not supported in this form.".into());
            }
            let strategy = match method {
                MergeMethod::Merge => "noFastForward",
                MergeMethod::Squash => "squash",
                MergeMethod::Rebase => "rebase",
            };
            let result = send(
                "PATCH",
                &url,
                Some(
                    json!({"status":"completed","lastMergeSourceCommit":{"commitId":head},"completionOptions":{"mergeStrategy":strategy,"deleteSourceBranch":false,"bypassPolicy":false}}),
                ),
            )?;
            if result["status"] == "completed" {
                Ok("Merged successfully.".into())
            } else if result["mergeStatus"] == "queued" || result["status"] == "active" {
                Ok("Azure accepted the completion request; completion is unconfirmed. Refresh to inspect merge status and branch policies.".into())
            } else {
                Err("Azure did not confirm a merge. Refresh to inspect branch policies and merge status.".into())
            }
        }
        Action::Review {
            body,
            decision,
            inline,
            ..
        } => {
            let result = if *decision == Decision::Comment {
                let mut payload = json!({"comments":[{"parentCommentId":0,"content":body,"commentType":1}],"status":1});
                if let Some(line) = inline {
                    let iterations = list(
                        &format!("{root}/{number}/iterations?api-version=7.1"),
                        "Authorization",
                        auth,
                        Some("value"),
                    )?;
                    let iteration = iterations
                        .iter()
                        .max_by_key(|v| v["id"].as_u64().unwrap_or(0))
                        .ok_or("Missing Azure review iteration.")?;
                    if iteration["sourceRefCommit"]["commitId"] != head.as_str() {
                        return Err("Review iteration changed. Refresh before commenting.".into());
                    }
                    let id = iteration["id"]
                        .as_u64()
                        .ok_or("Missing Azure review iteration ID.")?;
                    let changes = object(&format!("{root}/{number}/iterations/{id}/changes?$compareTo=0&$top=2000&api-version=7.1"), "Authorization", auth)?;
                    if changes["nextSkip"].as_u64().unwrap_or(0) > 0 {
                        return Err(
                            "Review diff exceeds the 2000 file limit. Comment in Azure DevOps."
                                .into(),
                        );
                    }
                    let file_path = format!("/{}", line.path);
                    let tracking = changes["changeEntries"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .find(|change| {
                            change["item"]["path"] == file_path
                                || change["originalPath"] == file_path
                        })
                        .and_then(|change| change["changeTrackingId"].as_u64())
                        .ok_or("File is not present in this review iteration's diff.")?;
                    let mut context = json!({"filePath":file_path});
                    let side = if line.new_line > 0 {
                        "rightFile"
                    } else {
                        "leftFile"
                    };
                    let number = if line.new_line > 0 {
                        line.new_line
                    } else {
                        line.old_line
                    };
                    context[format!("{side}Start")] = json!({"line":number,"offset":1});
                    context[format!("{side}End")] = json!({"line":number,"offset":1});
                    payload["threadContext"] = context;
                    payload["pullRequestThreadContext"] = json!({"changeTrackingId":tracking,"iterationContext":{"firstComparingIteration":id,"secondComparingIteration":id}});
                }
                send(
                    "POST",
                    &format!("{root}/{number}/threads?api-version=7.1"),
                    Some(payload),
                )?
            } else {
                let project = base
                    .split("/_apis/")
                    .next()
                    .ok_or("Invalid Azure API URL.")?;
                let organization = project
                    .rsplit_once('/')
                    .ok_or("Invalid Azure organization URL.")?
                    .0;
                let identity = send(
                    "GET",
                    &format!("{organization}/_apis/connectionData?api-version=7.2-preview.1"),
                    None,
                )?;
                let id = identity["authenticatedUser"]["id"]
                    .as_str()
                    .ok_or("Azure did not identify the selected account.")?;
                let vote = if *decision == Decision::Approve {
                    10
                } else {
                    -10
                };
                send(
                    "PUT",
                    &format!(
                        "{root}/{number}/reviewers/{}?api-version=7.1",
                        component(id)
                    ),
                    Some(json!({"vote":vote,"id":id})),
                )?
            };
            let confirmed = if *decision == Decision::Comment {
                result["id"].is_number()
            } else {
                result["vote"].as_i64()
                    == Some(if *decision == Decision::Approve {
                        10
                    } else {
                        -10
                    })
            };
            if !confirmed {
                return Err("Azure did not confirm the review. Refresh before retrying.".into());
            }
            let after = send("GET", &url, None).map_err(|_| "Review submitted, but its current source commit could not be verified. Refresh before further actions.")?;
            verify(&after, head, target).map_err(|_| "Review submitted, but the pull request changed during submission. Inspect its current votes in Azure DevOps.")?;
            verify_live(base, &after, auth, head).map_err(|_| "Review submitted, but the source branch changed or could not be verified. Inspect its current votes in Azure DevOps.")?;
            Ok("Review submitted; displayed source commit verified after submission.".into())
        }
        _ => unreachable!(),
    }
}

fn verify(current: &Value, head: &str, target: &str) -> Result<(), String> {
    if current["status"] != "active"
        || current["lastMergeSourceCommit"]["commitId"] != head
        || branch(&current["targetRefName"]) != target
    {
        return Err(
            "Pull request head, target or state changed. Refresh before submitting.".into(),
        );
    }
    Ok(())
}

fn fork_repository_url(base: &str, source: &str, target: &str) -> Result<String, String> {
    let source: Vec<_> = source.split('/').collect();
    let target: Vec<_> = target.split('/').collect();
    if source.len() != 3 || target.len() != 3 || source[0] != target[0] {
        return Err("Azure fork pull requests must stay within the selected organization.".into());
    }
    let mut url = url::Url::parse(base).map_err(|_| "Invalid Azure API URL.")?;
    url.path_segments_mut()
        .map_err(|_| "Invalid Azure API URL.")?
        .clear()
        .extend([
            source[0],
            source[1],
            "_apis",
            "git",
            "repositories",
            source[2],
        ]);
    Ok(url.into())
}
