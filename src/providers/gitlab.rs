use super::pagination::{component, list};
use super::{PrItem, PrState};

pub fn list_mrs(host: &str, namespace: &str, project: &str, token: &str, state: &str) -> Result<Vec<PrItem>, String> {
    // GitLab accepts URL-encoded "namespace/project" as the project ID
    let encoded = component(&format!("{namespace}/{project}"));
    let gl_state = match state { "closed" | "all" => "all", "merged" => "merged", _ => "opened" };
    let url = format!("https://{}/api/v4/projects/{}/merge_requests?state={}&per_page=50&order_by=updated_at",
        host, encoded, gl_state);

    let mrs = list(&url, "PRIVATE-TOKEN", token, None)?;
    Ok(mrs.into_iter().filter_map(parse_mr).filter(|mr| state != "closed" || matches!(mr.state, PrState::Closed | PrState::Merged)).collect())
}

pub fn details(host: &str, namespace: &str, project: &str, number: u32, token: &str) -> Result<(PrItem, Vec<String>), String> {
    let encoded = component(&format!("{namespace}/{project}"));
    let url = format!("https://{host}/api/v4/projects/{encoded}/merge_requests/{number}");
    let body = super::pagination::object(&url, "PRIVATE-TOKEN", token)?;
    let mut item = parse_mr(body.clone()).ok_or("Invalid merge request details.")?;
    item.check_state = body["head_pipeline"]["status"].as_str().map(|status| match status {
        "success" | "skipped" => super::CheckState::Success,
        "failed" | "canceled" => super::CheckState::Failure,
        _ => super::CheckState::Pending,
    });
    Ok((item, Vec::new()))
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
        queue_status: None,
        head_oid: v["sha"].as_str().unwrap_or("").to_string(),
        number, title, author, state, head_ref, base_ref, url, updated_ts,
        label_names: labels, check_state: None, review_state: None, body, comments, changed_files: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn merged_and_closed_drafts_keep_terminal_state() {
        let mut fixture = serde_json::json!({"iid": 9, "title": "Draft: Change", "state": "merged", "draft": true});
        assert_eq!(parse_mr(fixture.clone()).unwrap().state, PrState::Merged);
        fixture["state"] = "closed".into();
        assert_eq!(parse_mr(fixture).unwrap().state, PrState::Closed);
    }
}

// ── Phase 13: Issues, CI, Releases ───────────────────────────────────────────

pub fn list_issues(host: &str, namespace: &str, project: &str, token: &str, state: &str) -> Result<Vec<super::IssueItem>, String> {
    let encoded = component(&format!("{namespace}/{project}"));
    let gl_state = match state { "closed" => "closed", "all" => "all", _ => "opened" };
    let url = format!("https://{}/api/v4/projects/{}/issues?state={}&per_page=50&order_by=updated_at",
        host, encoded, gl_state);
    let items = list(&url, "PRIVATE-TOKEN", token, None)?;
    Ok(items.into_iter().filter_map(|v| {
        let number   = v["iid"].as_u64()? as u32;
        let title    = v["title"].as_str()?.to_string();
        let author   = v["author"]["username"].as_str().unwrap_or("unknown").to_string();
        let state    = if v["state"].as_str().unwrap_or("opened") == "closed" { super::IssueState::Closed } else { super::IssueState::Open };
        let url      = v["web_url"].as_str().unwrap_or("").to_string();
        let body     = v["description"].as_str().unwrap_or("").to_string();
        let comments = v["user_notes_count"].as_u64().unwrap_or(0) as u32;
        let updated  = super::parse_iso8601(v["updated_at"].as_str().unwrap_or(""));
        let labels: Vec<String> = v["labels"].as_array()
            .map(|a| a.iter().filter_map(|l| l.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        Some(super::IssueItem { number, title, author, state, url, label_names: labels, body, comments, updated_ts: updated })
    }).collect())
}

pub fn list_pipelines(host: &str, namespace: &str, project: &str, branch: &str, token: &str) -> Result<Vec<super::CiRun>, String> {
    let encoded = component(&format!("{namespace}/{project}"));
    let url = format!("https://{}/api/v4/projects/{}/pipelines?ref={}&per_page=15&order_by=updated_at",
        host, encoded, component(branch));
    let items = list(&url, "PRIVATE-TOKEN", token, None)?;
    Ok(items.into_iter().filter_map(|p| {
        let id      = p["id"].as_u64()?.to_string();
        let name    = format!("Pipeline #{}", p["id"].as_u64().unwrap_or(0));
        let status  = super::CiStatus::from_gitlab(p["status"].as_str().unwrap_or(""));
        let url     = p["web_url"].as_str().unwrap_or("").to_string();
        let branch  = p["ref"].as_str().unwrap_or("").to_string();
        let updated = super::parse_iso8601(p["updated_at"].as_str().unwrap_or(""));
        Some(super::CiRun { id, name, status, url, branch, updated_ts: updated })
    }).collect())
}

pub fn list_releases(host: &str, namespace: &str, project: &str, token: &str) -> Result<Vec<super::ReleaseItem>, String> {
    let encoded = component(&format!("{namespace}/{project}"));
    let url = format!("https://{}/api/v4/projects/{}/releases?per_page=20", host, encoded);
    let items = list(&url, "PRIVATE-TOKEN", token, None)?;
    Ok(items.into_iter().filter_map(|r| {
        let tag_name  = r["tag_name"].as_str()?.to_string();
        let name      = r["name"].as_str().unwrap_or(&tag_name).to_string();
        let body      = r["description"].as_str().unwrap_or("").to_string();
        let url       = r["_links"]["self"].as_str().unwrap_or("").to_string();
        let published = super::parse_iso8601(r["released_at"].as_str().unwrap_or(""));
        Some(super::ReleaseItem { tag_name, name, body, url, published_ts: published, prerelease: false })
    }).collect())
}
