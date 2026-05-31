use super::{CheckState, PrItem, PrState, ReviewState};

const API: &str = "https://api.github.com";

fn api_base(host: &str) -> String {
    if host == "github.com" {
        API.to_string()
    } else {
        format!("https://{}/api/v3", host)
    }
}

pub fn list_prs(host: &str, owner: &str, repo: &str, token: &str, state: &str) -> Result<Vec<PrItem>, String> {
    let url = format!("{}/repos/{}/{}/pulls?state={}&per_page=50&sort=updated",
        api_base(host), owner, repo, state);

    let resp = ureq::get(&url)
        .set("Authorization", &format!("token {}", token))
        .set("Accept", "application/vnd.github.v3+json")
        .set("User-Agent", "Recorde/0.1")
        .call()
        .map_err(|e| format!("GitHub API error: {}", e))?;

    let prs: Vec<serde_json::Value> = resp.into_json()
        .map_err(|e| format!("JSON parse error: {}", e))?;

    Ok(prs.into_iter().filter_map(parse_pr).collect())
}

fn parse_pr(v: serde_json::Value) -> Option<PrItem> {
    let number = v["number"].as_u64()? as u32;
    let title  = v["title"].as_str()?.to_string();
    let author = v["user"]["login"].as_str().unwrap_or("unknown").to_string();
    let state_str = v["state"].as_str().unwrap_or("open");
    let draft  = v["draft"].as_bool().unwrap_or(false);
    let state  = PrState::from_str_github(state_str, draft);
    let head_ref = v["head"]["ref"].as_str().unwrap_or("").to_string();
    let base_ref = v["base"]["ref"].as_str().unwrap_or("").to_string();
    let url    = v["html_url"].as_str().unwrap_or("").to_string();
    let body   = v["body"].as_str().unwrap_or("").to_string();
    let comments = v["comments"].as_u64().unwrap_or(0) as u32;
    let changed_files = v["changed_files"].as_u64().map(|n| n as u32);

    let updated_ts = super::parse_iso8601(v["updated_at"].as_str().unwrap_or(""));

    let labels: Vec<String> = v["labels"].as_array()
        .map(|arr| arr.iter().filter_map(|l| l["name"].as_str().map(str::to_string)).collect())
        .unwrap_or_default();

    // GitHub returns review state via a separate endpoint; we use a combined field if present
    let review_state = v["review_decision"].as_str().and_then(|s| match s {
        "APPROVED"           => Some(ReviewState::Approved),
        "CHANGES_REQUESTED"  => Some(ReviewState::ChangesRequested),
        _                    => Some(ReviewState::Pending),
    });

    Some(PrItem {
        number, title, author, state, head_ref, base_ref, url, updated_ts,
        label_names: labels, check_state: None, review_state, body, comments, changed_files,
    })
}


/// Fetch CI status for a specific commit SHA.
pub fn check_state(host: &str, owner: &str, repo: &str, sha: &str, token: &str) -> Option<CheckState> {
    let url = format!("{}/repos/{}/{}/commits/{}/check-runs?per_page=10",
        api_base(host), owner, repo, sha);
    let resp = ureq::get(&url)
        .set("Authorization", &format!("token {}", token))
        .set("Accept", "application/vnd.github.v3+json")
        .set("User-Agent", "Recorde/0.1")
        .call().ok()?;
    let body: serde_json::Value = resp.into_json().ok()?;
    let runs = body["check_runs"].as_array()?;
    if runs.is_empty() { return None; }
    let mut any_failure = false;
    let mut any_pending = false;
    for run in runs {
        match run["conclusion"].as_str().unwrap_or("") {
            "failure" | "timed_out" | "cancelled" => any_failure = true,
            "" => any_pending = true,
            _  => {}
        }
    }
    if any_failure { Some(CheckState::Failure) }
    else if any_pending { Some(CheckState::Pending) }
    else { Some(CheckState::Success) }
}
