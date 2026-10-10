//! Optional queue reads. Never infer a successful enqueue from these responses.
use serde_json::{json, Value};
use std::io::Read;

pub(crate) fn github(
    host: &str,
    owner: &str,
    repo: &str,
    number: u32,
    head: &str,
    token: &str,
) -> Result<String, String> {
    let url = if host == "github.com" {
        "https://api.github.com/graphql".into()
    } else {
        format!("https://{host}/api/graphql")
    };
    let body = json!({
        "query": "query($owner: String!, $repo: String!, $number: Int!) { repository(owner: $owner, name: $repo) { pullRequest(number: $number) { headRefOid mergeQueueEntry { position state } } } }",
        "variables": {"owner":owner,"repo":repo,"number":number}
    });
    let response = ureq::AgentBuilder::new()
        .redirects(0)
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .post(&url)
        .set("Authorization", &format!("Bearer {token}"))
        .set("User-Agent", "Recorde")
        .send_json(body)
        .map_err(|_| "Queue status unavailable; check account permissions and provider support.")?;
    if response.status() != 200 {
        return Err("Queue status request was redirected or rejected.".into());
    }
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(1_048_577)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read queue status.")?;
    if bytes.len() > 1_048_576 {
        return Err("Queue response exceeded the size limit.".into());
    }
    let body: Value = serde_json::from_slice(&bytes).map_err(|_| "Invalid queue response.")?;
    github_status(&body, head)
}

fn github_status(body: &Value, head: &str) -> Result<String, String> {
    if body["errors"]
        .as_array()
        .is_some_and(|errors| !errors.is_empty())
    {
        return Err(
            "Queue status unavailable; check account permissions and provider support.".into(),
        );
    }
    let review = &body["data"]["repository"]["pullRequest"];
    if review["headRefOid"].as_str() != Some(head) {
        return Err("Review head changed; refresh before reading queue status.".into());
    }
    let entry = review
        .get("mergeQueueEntry")
        .ok_or("Queue status missing from response.")?;
    if entry.is_null() {
        return Ok("Not in the merge queue".into());
    }
    let position = entry["position"]
        .as_u64()
        .ok_or("Invalid queue position.")?;
    let state = status_text(&entry["state"])?;
    Ok(format!("Merge queue: {state}, position {position}"))
}

pub(crate) fn gitlab(
    host: &str,
    owner: &str,
    project: &str,
    number: u32,
    branch: &str,
    token: &str,
) -> Result<String, String> {
    let component = super::pagination::component;
    let url = format!(
        "https://{host}/api/v4/projects/{}/merge_trains/{}?scope=active&sort=asc&per_page=100",
        component(&format!("{owner}/{project}")),
        component(branch)
    );
    let entries = super::pagination::list(&url, "Authorization", &format!("Bearer {token}"), None)?;
    gitlab_status(entries, number, branch)
}

fn gitlab_status(mut entries: Vec<Value>, number: u32, branch: &str) -> Result<String, String> {
    let mut ids = std::collections::HashSet::new();
    for entry in &entries {
        let id = entry["id"].as_u64().ok_or("Missing merge-train ID.")?;
        if !ids.insert(id) || entry["target_branch"].as_str() != Some(branch) {
            return Err("Merge train changed during pagination; refresh its status.".into());
        }
    }
    entries.sort_by_key(|entry| entry["id"].as_u64());
    for (index, entry) in entries.iter().enumerate() {
        if entry["merge_request"]["iid"].as_u64() == Some(u64::from(number)) {
            return Ok(format!(
                "Merge train: {}, position {}",
                status_text(&entry["status"])?,
                index + 1
            ));
        }
    }
    Ok("Not in the active merge train".into())
}

fn status_text(value: &Value) -> Result<String, String> {
    value
        .as_str()
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 64
                && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        })
        .map(|s| s.to_lowercase().replace('_', " "))
        .ok_or_else(|| "Invalid queue state.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn github_queue_checks_head_and_does_not_treat_errors_as_empty_queue() {
        let mut body = json!({"data":{"repository":{"pullRequest":{"headRefOid":"head","mergeQueueEntry":{"position":2,"state":"AWAITING_CHECKS"}}}}});
        assert_eq!(
            github_status(&body, "head").unwrap(),
            "Merge queue: awaiting checks, position 2"
        );
        assert!(github_status(&body, "other").is_err());
        body["data"]["repository"]["pullRequest"]["mergeQueueEntry"] = Value::Null;
        assert_eq!(
            github_status(&body, "head").unwrap(),
            "Not in the merge queue"
        );
        body["errors"] = json!([{"message":"credential must not leak"}]);
        assert!(!github_status(&body, "head")
            .unwrap_err()
            .contains("credential"));
    }
    #[test]
    fn train_position_uses_ids_within_the_requested_target_and_rejects_repeated_pages() {
        let first =
            json!({"id":10,"target_branch":"main","merge_request":{"iid":3},"status":"fresh"});
        let second =
            json!({"id":20,"target_branch":"main","merge_request":{"iid":7},"status":"idle"});
        assert_eq!(
            gitlab_status(vec![second.clone(), first.clone()], 7, "main").unwrap(),
            "Merge train: idle, position 2"
        );
        assert!(gitlab_status(vec![first.clone(), first.clone()], 3, "main").is_err());
        assert!(gitlab_status(vec![first], 3, "different").is_err());
    }
}
