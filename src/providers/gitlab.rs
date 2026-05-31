use super::{PrItem, PrState};

pub fn list_mrs(host: &str, namespace: &str, project: &str, token: &str, state: &str) -> Result<Vec<PrItem>, String> {
    // GitLab accepts URL-encoded "namespace/project" as the project ID
    let encoded = format!("{}/{}", namespace, project).replace('/', "%2F");
    let gl_state = match state { "closed" => "closed", "merged" => "merged", _ => "opened" };
    let url = format!("https://{}/api/v4/projects/{}/merge_requests?state={}&per_page=50&order_by=updated_at",
        host, encoded, gl_state);

    let resp = ureq::get(&url)
        .set("PRIVATE-TOKEN", token)
        .set("User-Agent", "Recorde/0.1")
        .call()
        .map_err(|e| format!("GitLab API error: {}", e))?;

    let mrs: Vec<serde_json::Value> = resp.into_json()
        .map_err(|e| format!("JSON parse error: {}", e))?;

    Ok(mrs.into_iter().filter_map(parse_mr).collect())
}

fn parse_mr(v: serde_json::Value) -> Option<PrItem> {
    let number    = v["iid"].as_u64()? as u32;
    let title     = v["title"].as_str()?.to_string();
    let author    = v["author"]["username"].as_str().unwrap_or("unknown").to_string();
    let state_str = v["state"].as_str().unwrap_or("opened");
    let draft     = v["draft"].as_bool().unwrap_or(false)
        || title.to_lowercase().starts_with("draft:")
        || title.to_lowercase().starts_with("wip:");
    let state     = PrState::from_str_gitlab(state_str, draft);
    let head_ref  = v["source_branch"].as_str().unwrap_or("").to_string();
    let base_ref  = v["target_branch"].as_str().unwrap_or("").to_string();
    let url       = v["web_url"].as_str().unwrap_or("").to_string();
    let body      = v["description"].as_str().unwrap_or("").to_string();
    let comments  = v["user_notes_count"].as_u64().unwrap_or(0) as u32;

    let updated_ts = super::parse_iso8601(v["updated_at"].as_str().unwrap_or(""));

    let labels: Vec<String> = v["labels"].as_array()
        .map(|arr| arr.iter().filter_map(|l| l.as_str().map(str::to_string)).collect())
        .unwrap_or_default();

    Some(PrItem {
        number, title, author, state, head_ref, base_ref, url, updated_ts,
        label_names: labels, check_state: None, review_state: None, body, comments, changed_files: None,
    })
}
