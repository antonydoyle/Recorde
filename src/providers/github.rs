use super::pagination::{component, list};
use super::{
    CheckState, CiRun, CiStatus, IssueItem, IssueState, PrItem, PrState, ReleaseItem, ReviewState,
};

const API: &str = "https://api.github.com";

fn api_base(host: &str) -> String {
    if host == "github.com" {
        API.to_string()
    } else {
        format!("https://{}/api/v3", host)
    }
}

pub fn list_prs(
    host: &str,
    owner: &str,
    repo: &str,
    token: &str,
    state: &str,
) -> Result<Vec<PrItem>, String> {
    let url = format!(
        "{}/repos/{}/{}/pulls?state={}&per_page=100&sort=updated&direction=desc",
        api_base(host),
        component(owner),
        component(repo),
        component(state)
    );

    let prs = list(&url, "Authorization", &format!("token {token}"), None)?;
    Ok(prs.into_iter().filter_map(parse_pr).collect())
}

fn parse_pr(v: serde_json::Value) -> Option<PrItem> {
    let number = v["number"].as_u64()? as u32;
    let title = v["title"].as_str()?.to_string();
    let author = v["user"]["login"].as_str().unwrap_or("unknown").to_string();
    let state_str = v["state"].as_str().unwrap_or("open");
    let draft = v["draft"].as_bool().unwrap_or(false);
    let state = if !v["merged_at"].is_null() || v["merged"].as_bool() == Some(true) {
        PrState::Merged
    } else {
        PrState::from_str_github(state_str, draft)
    };
    let head_ref = v["head"]["ref"].as_str().unwrap_or("").to_string();
    let base_ref = v["base"]["ref"].as_str().unwrap_or("").to_string();
    let url = v["html_url"].as_str().unwrap_or("").to_string();
    let body = v["body"].as_str().unwrap_or("").to_string();
    let comments = v["comments"].as_u64().unwrap_or(0) as u32;
    let changed_files = v["changed_files"].as_u64().map(|n| n as u32);

    let updated_ts = super::parse_iso8601(v["updated_at"].as_str().unwrap_or(""));

    let labels: Vec<String> = v["labels"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|l| l["name"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    // GitHub returns review state via a separate endpoint; we use a combined field if present
    let review_state = v["review_decision"].as_str().and_then(|s| match s {
        "APPROVED" => Some(ReviewState::Approved),
        "CHANGES_REQUESTED" => Some(ReviewState::ChangesRequested),
        _ => Some(ReviewState::Pending),
    });

    Some(PrItem {
        queue_status: None,
        head_oid: v["head"]["sha"].as_str().unwrap_or("").to_string(),
        number,
        title,
        author,
        state,
        head_ref,
        base_ref,
        url,
        updated_ts,
        label_names: labels,
        check_state: None,
        review_state,
        body,
        comments,
        changed_files,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn check_and_review_rollups_keep_failures_pending_and_latest_decisions() {
        use serde_json::json;
        let runs = vec![
            json!({"conclusion": "success"}),
            json!({"conclusion": null}),
        ];
        assert_eq!(aggregate_checks(&runs, &[]), Some(CheckState::Pending));
        assert_eq!(
            aggregate_checks(&runs, &[json!({"context":"ci","state":"error"})]),
            Some(CheckState::Failure)
        );
        assert_eq!(
            aggregate_checks(
                &[],
                &[
                    json!({"context":"ci","state":"success"}),
                    json!({"context":"ci","state":"failure"})
                ]
            ),
            Some(CheckState::Success)
        );
        let reviews = vec![
            json!({"id":1,"user":{"login":"a"},"state":"CHANGES_REQUESTED"}),
            json!({"id":2,"user":{"login":"a"},"state":"APPROVED"}),
        ];
        assert_eq!(
            aggregate_reviews(&reviews, false),
            Some(ReviewState::Approved)
        );
        assert_eq!(
            aggregate_reviews(&reviews, true),
            Some(ReviewState::Pending)
        );
    }
    #[test]
    fn merged_and_closed_drafts_are_not_shown_as_open_drafts() {
        let mut fixture = serde_json::json!({"number": 7, "title": "Change", "state": "closed", "draft": true, "merged_at": "2026-01-01T00:00:00Z"});
        assert_eq!(parse_pr(fixture.clone()).unwrap().state, PrState::Merged);
        fixture["merged_at"] = serde_json::Value::Null;
        assert_eq!(parse_pr(fixture).unwrap().state, PrState::Closed);
    }
}

/// Fetch details only for the selected PR, avoiding one API fan-out per list row.
pub fn details(
    host: &str,
    owner: &str,
    repo: &str,
    number: u32,
    token: &str,
) -> Result<(PrItem, Vec<String>), String> {
    let base = format!(
        "{}/repos/{}/{}",
        api_base(host),
        component(owner),
        component(repo)
    );
    let auth = format!("token {token}");
    let body =
        super::pagination::object(&format!("{base}/pulls/{number}"), "Authorization", &auth)?;
    let mut item = parse_pr(body.clone()).ok_or("Invalid pull request details.")?;
    let mut warnings = Vec::new();
    // Fork checks belong to the repository containing the head commit.
    let head_owner = body["head"]["repo"]["owner"]["login"]
        .as_str()
        .unwrap_or(owner);
    let head_repo = body["head"]["repo"]["name"].as_str().unwrap_or(repo);
    let commit_base = format!(
        "{}/repos/{}/{}/commits/{}",
        api_base(host),
        component(head_owner),
        component(head_repo),
        component(&item.head_oid)
    );
    match (
        list(
            &format!("{commit_base}/check-runs?per_page=100&filter=latest"),
            "Authorization",
            &auth,
            Some("check_runs"),
        ),
        list(
            &format!("{commit_base}/statuses?per_page=100"),
            "Authorization",
            &auth,
            None,
        ),
    ) {
        (Ok(runs), Ok(statuses)) => item.check_state = aggregate_checks(&runs, &statuses),
        (runs, statuses) => {
            for error in [runs.err(), statuses.err()].into_iter().flatten() {
                warnings.push(format!("Checks unavailable: {error}"));
            }
        }
    }
    match list(
        &format!("{base}/pulls/{number}/reviews?per_page=100"),
        "Authorization",
        &auth,
        None,
    ) {
        Ok(reviews) => {
            let pending = body["requested_reviewers"]
                .as_array()
                .is_some_and(|a| !a.is_empty())
                || body["requested_teams"]
                    .as_array()
                    .is_some_and(|a| !a.is_empty());
            item.review_state = aggregate_reviews(&reviews, pending);
        }
        Err(error) => warnings.push(format!("Reviews unavailable: {error}")),
    }
    Ok((item, warnings))
}

fn aggregate_checks(
    runs: &[serde_json::Value],
    statuses: &[serde_json::Value],
) -> Option<CheckState> {
    let mut states = Vec::new();
    for run in runs {
        states.push(match run["conclusion"].as_str().unwrap_or("") {
            "success" | "neutral" | "skipped" => CheckState::Success,
            "failure" | "timed_out" | "cancelled" | "action_required" | "startup_failure"
            | "stale" => CheckState::Failure,
            _ => CheckState::Pending,
        });
    }
    let mut contexts = std::collections::HashSet::new();
    for status in statuses {
        if !contexts.insert(status["context"].as_str().unwrap_or("")) {
            continue;
        }
        states.push(match status["state"].as_str().unwrap_or("") {
            "success" => CheckState::Success,
            "failure" | "error" => CheckState::Failure,
            _ => CheckState::Pending,
        });
    }
    if states.is_empty() {
        None
    } else if states.contains(&CheckState::Failure) {
        Some(CheckState::Failure)
    } else if states.contains(&CheckState::Pending) {
        Some(CheckState::Pending)
    } else {
        Some(CheckState::Success)
    }
}

fn aggregate_reviews(reviews: &[serde_json::Value], pending: bool) -> Option<ReviewState> {
    let mut latest = std::collections::HashMap::new();
    for review in reviews {
        let state = review["state"].as_str().unwrap_or("");
        if !matches!(state, "APPROVED" | "CHANGES_REQUESTED" | "DISMISSED") {
            continue;
        }
        let user = review["user"]["login"].as_str().unwrap_or("");
        let id = review["id"].as_u64().unwrap_or(0);
        let entry = latest.entry(user).or_insert((0, ""));
        if id >= entry.0 {
            *entry = (id, state);
        }
    }
    if latest.values().any(|(_, s)| *s == "CHANGES_REQUESTED") {
        Some(ReviewState::ChangesRequested)
    } else if pending {
        Some(ReviewState::Pending)
    } else if latest.values().any(|(_, s)| *s == "APPROVED") {
        Some(ReviewState::Approved)
    } else {
        None
    }
}

// ── Phase 13: Issues, CI, Releases ───────────────────────────────────────────

pub fn list_issues(
    host: &str,
    owner: &str,
    repo: &str,
    token: &str,
    state: &str,
) -> Result<Vec<IssueItem>, String> {
    let url = format!(
        "{}/repos/{}/{}/issues?state={}&per_page=100&sort=updated&direction=desc&filter=all",
        api_base(host),
        component(owner),
        component(repo),
        component(state)
    );
    let items = list(&url, "Authorization", &format!("token {token}"), None)?;
    // GitHub returns both issues and PRs from this endpoint; filter out PRs
    Ok(items
        .into_iter()
        .filter(|v| v["pull_request"].is_null())
        .filter_map(parse_issue)
        .collect())
}

fn parse_issue(v: serde_json::Value) -> Option<IssueItem> {
    let number = v["number"].as_u64()? as u32;
    let title = v["title"].as_str()?.to_string();
    let author = v["user"]["login"].as_str().unwrap_or("unknown").to_string();
    let state = if v["state"].as_str().unwrap_or("open") == "closed" {
        IssueState::Closed
    } else {
        IssueState::Open
    };
    let url = v["html_url"].as_str().unwrap_or("").to_string();
    let body = v["body"].as_str().unwrap_or("").to_string();
    let comments = v["comments"].as_u64().unwrap_or(0) as u32;
    let updated = super::parse_iso8601(v["updated_at"].as_str().unwrap_or(""));
    let labels: Vec<String> = v["labels"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|l| l["name"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    Some(IssueItem {
        number,
        title,
        author,
        state,
        url,
        label_names: labels,
        body,
        comments,
        updated_ts: updated,
    })
}

pub fn list_ci_runs(
    host: &str,
    owner: &str,
    repo: &str,
    branch: &str,
    token: &str,
) -> Result<Vec<CiRun>, String> {
    let url = format!(
        "{}/repos/{}/{}/actions/runs?branch={}&per_page=15",
        api_base(host),
        component(owner),
        component(repo),
        component(branch)
    );
    let runs = list(
        &url,
        "Authorization",
        &format!("token {token}"),
        Some("workflow_runs"),
    )?;
    Ok(runs
        .into_iter()
        .filter_map(|r| {
            let id = r["id"].as_u64()?.to_string();
            let name = r["name"].as_str().unwrap_or("Workflow").to_string();
            let status = CiStatus::from_github(
                r["conclusion"].as_str().unwrap_or(""),
                r["status"].as_str().unwrap_or(""),
            );
            let url = r["html_url"].as_str().unwrap_or("").to_string();
            let branch = r["head_branch"].as_str().unwrap_or("").to_string();
            let updated = super::parse_iso8601(r["updated_at"].as_str().unwrap_or(""));
            Some(CiRun {
                id,
                name,
                status,
                url,
                branch,
                updated_ts: updated,
            })
        })
        .collect())
}

pub fn list_releases(
    host: &str,
    owner: &str,
    repo: &str,
    token: &str,
) -> Result<Vec<ReleaseItem>, String> {
    let url = format!(
        "{}/repos/{}/{}/releases?per_page=20",
        api_base(host),
        component(owner),
        component(repo)
    );
    let items = list(&url, "Authorization", &format!("token {token}"), None)?;
    Ok(items
        .into_iter()
        .filter_map(|r| {
            let tag_name = r["tag_name"].as_str()?.to_string();
            let name = r["name"].as_str().unwrap_or(&tag_name).to_string();
            let body = r["body"].as_str().unwrap_or("").to_string();
            let url = r["html_url"].as_str().unwrap_or("").to_string();
            let published = super::parse_iso8601(r["published_at"].as_str().unwrap_or(""));
            let prerelease = r["prerelease"].as_bool().unwrap_or(false);
            Some(ReleaseItem {
                tag_name,
                name,
                body,
                url,
                published_ts: published,
                prerelease,
            })
        })
        .collect())
}
