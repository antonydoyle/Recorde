//! Bitbucket Cloud 2.0. API tokens use the account's Atlassian email and Basic auth.
use super::pagination::{component, list, object};
use super::{CheckState, CiRun, CiStatus, PrItem, PrState, ReviewState};
use serde_json::Value;

pub(crate) fn basic(username: &str, token: &str) -> String {
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(format!("{username}:{token}"));
    crate::auth::redact::remember(token);
    crate::auth::redact::remember(&encoded);
    let auth = format!("Basic {encoded}");
    crate::auth::redact::remember(&auth);
    auth
}

pub(crate) fn base(owner: &str, repo: &str) -> String {
    format!(
        "https://api.bitbucket.org/2.0/repositories/{}/{}",
        component(owner),
        component(repo)
    )
}

pub(crate) fn review_head(
    owner: &str,
    repo: &str,
    number: u32,
    auth: &str,
) -> Result<String, String> {
    let body = object(
        &format!("{}/pullrequests/{number}", base(owner, repo)),
        "Authorization",
        auth,
    )?;
    body["source"]["commit"]["hash"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "Missing review source commit.".into())
}

pub fn list_prs(owner: &str, repo: &str, auth: &str, state: &str) -> Result<Vec<PrItem>, String> {
    let states = match state {
        "closed" => "state=MERGED&state=DECLINED&state=SUPERSEDED",
        "all" => "state=OPEN&state=MERGED&state=DECLINED&state=SUPERSEDED",
        _ => "state=OPEN",
    };
    let values = list(
        &format!(
            "{}/pullrequests?{states}&pagelen=50&sort=-updated_on",
            base(owner, repo)
        ),
        "Authorization",
        auth,
        Some("values"),
    )?;
    values
        .into_iter()
        .map(|v| parse_pr(&v).ok_or_else(|| "Invalid Bitbucket pull request.".into()))
        .collect()
}

pub(crate) fn parse_pr(v: &Value) -> Option<PrItem> {
    let state = match v["state"].as_str()? {
        "MERGED" => PrState::Merged,
        "DECLINED" | "SUPERSEDED" => PrState::Closed,
        "OPEN" if v["draft"].as_bool() == Some(true) => PrState::Draft,
        "OPEN" => PrState::Open,
        _ => return None,
    };
    let votes: Vec<_> = v["participants"].as_array().into_iter().flatten().collect();
    let review_state = if votes.iter().any(|v| v["state"] == "changes_requested") {
        Some(ReviewState::ChangesRequested)
    } else if votes.iter().any(|v| v["approved"] == true) {
        Some(ReviewState::Approved)
    } else {
        None
    };
    Some(PrItem {
        queue_status: None,
        head_oid: v["source"]["commit"]["hash"]
            .as_str()
            .unwrap_or_default()
            .into(),
        number: u32::try_from(v["id"].as_u64()?).ok()?,
        title: v["title"].as_str()?.into(),
        author: v["author"]["display_name"]
            .as_str()
            .unwrap_or("unknown")
            .into(),
        state,
        head_ref: v["source"]["branch"]["name"]
            .as_str()
            .unwrap_or_default()
            .into(),
        base_ref: v["destination"]["branch"]["name"]
            .as_str()
            .unwrap_or_default()
            .into(),
        url: v["links"]["html"]["href"]
            .as_str()
            .unwrap_or_default()
            .into(),
        updated_ts: super::parse_iso8601(v["updated_on"].as_str().unwrap_or_default()),
        label_names: vec![],
        check_state: None,
        review_state,
        body: v["description"].as_str().unwrap_or_default().into(),
        comments: v["comment_count"].as_u64().unwrap_or_default() as u32,
        changed_files: None,
    })
}

pub fn details(
    owner: &str,
    repo: &str,
    number: u32,
    auth: &str,
) -> Result<(PrItem, Vec<String>), String> {
    let root = format!("{}/pullrequests/{number}", base(owner, repo));
    let body = object(&root, "Authorization", auth)?;
    let mut item = parse_pr(&body).ok_or("Invalid Bitbucket pull request details.")?;
    let mut warnings = vec![];
    match list(
        &format!("{root}/statuses?pagelen=100"),
        "Authorization",
        auth,
        Some("values"),
    ) {
        Ok(values) => item.check_state = checks(&values),
        Err(e) => warnings.push(format!("Checks unavailable: {e}")),
    }
    warnings.push("Bitbucket approvals and change requests apply to the pull request; the API cannot bind a vote atomically to a commit. Merge in Bitbucket to enforce its current merge checks.".into());
    Ok((item, warnings))
}

fn checks(values: &[Value]) -> Option<CheckState> {
    if values.is_empty() {
        return None;
    }
    if values
        .iter()
        .any(|v| matches!(v["state"].as_str(), Some("FAILED" | "STOPPED")))
    {
        Some(CheckState::Failure)
    } else if values.iter().any(|v| v["state"] != "SUCCESSFUL") {
        Some(CheckState::Pending)
    } else {
        Some(CheckState::Success)
    }
}

pub fn list_ci_runs(
    owner: &str,
    repo: &str,
    branch: &str,
    auth: &str,
) -> Result<Vec<CiRun>, String> {
    let values = list(
        &format!(
            "{}/pipelines/?pagelen=50&sort=-created_on&target.ref_name={}",
            base(owner, repo),
            component(branch)
        ),
        "Authorization",
        auth,
        Some("values"),
    )?;
    values
        .into_iter()
        .map(|v| {
            let id = v["build_number"]
                .as_u64()
                .ok_or("Invalid Bitbucket pipeline.")?
                .to_string();
            let status = match v["state"]["result"]["name"].as_str() {
                Some("SUCCESSFUL") => CiStatus::Success,
                Some("FAILED" | "ERROR" | "EXPIRED") => CiStatus::Failure,
                Some("STOPPED") => CiStatus::Cancelled,
                _ if v["state"]["name"] == "IN_PROGRESS" => CiStatus::Running,
                _ => CiStatus::Pending,
            };
            Ok(CiRun {
                name: format!("Pipeline #{id}"),
                url: format!(
                    "https://bitbucket.org/{}/{}/pipelines/results/{id}",
                    component(owner),
                    component(repo)
                ),
                id,
                status,
                branch: v["target"]["ref_name"].as_str().unwrap_or_default().into(),
                updated_ts: super::parse_iso8601(
                    v["completed_on"]
                        .as_str()
                        .or(v["created_on"].as_str())
                        .unwrap_or_default(),
                ),
            })
        })
        .collect()
}

pub(crate) fn execute(
    base: &str,
    auth: &str,
    action: &super::write::Action,
) -> Result<String, String> {
    use super::write::{Action, Decision};
    super::write::validate(action, &crate::git::capabilities::ProviderType::Bitbucket)?;
    use serde_json::json;
    let send = |method, url: &str, body| {
        super::write::send(method, url, "Authorization", auth, auth, body)
    };
    let root = format!("{base}/pullrequests");
    let (url, body) = match action {
        Action::Create { source_project, title, body, source, target, draft, .. } => {
            let mut value = json!({"title":title,"description":body,"source":{"branch":{"name":source}},"destination":{"branch":{"name":target}},"draft":draft,"close_source_branch":false});
            if !source_project.is_empty() { value["source"]["repository"] = json!({"full_name":source_project}); }
            (root.clone(), value)
        }
        Action::Review { number, head, target, body, decision, inline } => {
            let current = send("GET", &format!("{root}/{number}"), None)?;
            verify(&current, head, target)?;
            match decision {
                Decision::Comment => {
                    let mut value = json!({"content":{"raw":body}});
                    if let Some(line) = inline {
                        value["inline"] = if line.new_line > 0 { json!({"path":line.path,"to":line.new_line}) }
                            else { json!({"path":line.path,"from":line.old_line}) };
                    }
                    (format!("{root}/{number}/comments"), value)
                }
                Decision::Approve => (format!("{root}/{number}/approve"), json!({})),
                Decision::RequestChanges => (format!("{root}/{number}/request-changes"), json!({})),
            }
        }
        Action::Merge { .. } => return Err("Bitbucket Cloud does not offer an atomic expected-head guard for merging. Open the pull request in Bitbucket to merge.".into()),
    };
    let result = send("POST", &url, Some(body))?;
    match action {
        Action::Create { .. } => Ok(format!(
            "Created pull request #{}.",
            result["id"]
                .as_u64()
                .ok_or("No confirmed pull request number. Refresh before retrying.")?
        )),
        Action::Review {
            number,
            head,
            target,
            decision,
            ..
        } => {
            let current = send("GET", &format!("{root}/{number}"), None)
                .map_err(|_| "Review submitted, but its current head could not be verified. Refresh before further actions.")?;
            verify(&current, head, target).map_err(|_| "Review was submitted, but the pull request changed during submission. Inspect the current review in Bitbucket.")?;
            let confirmed = match decision {
                Decision::Comment => result["id"].is_number(),
                Decision::Approve => result["approved"] == true,
                Decision::RequestChanges => result["state"] == "changes_requested",
            };
            if !confirmed {
                return Err("No confirmed review result. Refresh before retrying.".into());
            }
            Ok("Review submitted; displayed source commit verified after submission.".into())
        }
        _ => unreachable!(),
    }
}

fn verify(current: &Value, head: &str, target: &str) -> Result<(), String> {
    if current["state"] != "OPEN"
        || current["source"]["commit"]["hash"] != head
        || current["destination"]["branch"]["name"] != target
    {
        return Err(
            "Pull request head, target or state changed. Refresh before submitting.".into(),
        );
    }
    Ok(())
}
