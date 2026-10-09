//! Bounded pagination that never forwards credentials to another endpoint.
use serde_json::Value;
use std::collections::HashSet;
use std::io::Read;
use std::time::{Duration, Instant};

pub(super) fn object(url: &str, header: &str, credential: &str) -> Result<Value, String> {
    let response = super::request(url)
        .set(header, credential)
        .set("Accept", "application/json")
        .set("User-Agent", "Recorde")
        .call()
        .map_err(|e| crate::auth::redact::text(&format!("Provider request failed: {e}")))?;
    if response.status() != 200 {
        return Err(format!("Provider returned HTTP {}.", response.status()));
    }
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read provider response.")?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("Provider response exceeded the size limit.".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "Invalid provider JSON response.".into())
}

pub(super) fn component(value: &str) -> String {
    percent_encoding::utf8_percent_encode(value, percent_encoding::NON_ALPHANUMERIC).to_string()
}

pub(super) fn list(
    url: &str,
    header: &str,
    credential: &str,
    array_key: Option<&str>,
) -> Result<Vec<Value>, String> {
    let origin = url::Url::parse(url).map_err(|_| "Invalid API URL.")?;
    let mut next = Some(origin.clone());
    let mut visited = HashSet::new();
    let mut items = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(90);
    while let Some(page) = next.take() {
        if visited.len() >= 20 || Instant::now() >= deadline {
            return Err(
                "Provider list exceeded its page or time limit. Narrow the filter and refresh."
                    .into(),
            );
        }
        if page.origin() != origin.origin()
            || page.path() != origin.path()
            || !page.username().is_empty()
            || page.password().is_some()
        {
            return Err(
                "Provider returned a pagination link outside the requested endpoint.".into(),
            );
        }
        if !visited.insert(page.to_string()) {
            return Err("Provider returned a repeated pagination link.".into());
        }
        let response = super::request(page.as_str())
            .timeout(
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_secs(30)),
            )
            .set(header, credential)
            .set("Accept", "application/json")
            .set("User-Agent", "Recorde")
            .call()
            .map_err(|e| crate::auth::redact::text(&format!("Provider request failed: {e}")))?;
        if response.status() != 200 {
            return Err(format!("Provider returned HTTP {}.", response.status()));
        }
        if let Some(link) = response.header("Link") {
            for entry in link.split(',') {
                let mut fields = entry.trim().split(';');
                let target = fields.next().unwrap_or_default().trim();
                if fields.any(|field| field.trim() == "rel=\"next\"" || field.trim() == "rel=next")
                {
                    let target = target
                        .strip_prefix('<')
                        .and_then(|s| s.strip_suffix('>'))
                        .ok_or("Invalid pagination link.")?;
                    next = Some(page.join(target).map_err(|_| "Invalid pagination link.")?);
                }
            }
        }
        if next.is_none() {
            if let Some(number) = response.header("X-Next-Page").filter(|s| !s.is_empty()) {
                let number = number
                    .parse::<u32>()
                    .map_err(|_| "Invalid next page number.")?;
                if number == 0 {
                    return Err("Invalid next page number.".into());
                }
                let mut target = page.clone();
                let mut pairs: Vec<(String, String)> = page
                    .query_pairs()
                    .filter(|(key, _)| key != "page")
                    .map(|(k, v)| (k.into(), v.into()))
                    .collect();
                pairs.push(("page".into(), number.to_string()));
                target.query_pairs_mut().clear().extend_pairs(pairs);
                next = Some(target);
            }
        }
        const MAX_PAGE: u64 = 8 * 1024 * 1024;
        let mut bytes = Vec::new();
        response
            .into_reader()
            .take(MAX_PAGE + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Cannot read provider response.")?;
        if bytes.len() as u64 > MAX_PAGE {
            return Err("Provider response exceeded the size limit.".into());
        }
        let mut body: Value =
            serde_json::from_slice(&bytes).map_err(|_| "Invalid provider JSON response.")?;
        let array = match array_key {
            Some(key) => body.get_mut(key),
            None => Some(&mut body),
        }
        .and_then(Value::as_array_mut)
        .ok_or("Provider did not return an item list.")?;
        items.append(array);
        if items.len() > 2000 {
            return Err("Provider list is too large. Narrow the filter and refresh.".into());
        }
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    #[test]
    fn follows_both_provider_paging_styles_and_rejects_cross_host_links() {
        for style in ["link", "number", "foreign"] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                for page in 1..=if style == "foreign" { 1 } else { 2 } {
                    let (mut stream, _) = listener.accept().unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    let mut request = Vec::new();
                    let mut buffer = [0; 1024];
                    while !request.ends_with(b"\r\n\r\n") {
                        let count = stream.read(&mut buffer).unwrap();
                        assert!(count > 0 && request.len() < 16384);
                        request.extend_from_slice(&buffer[..count]);
                    }
                    let request = String::from_utf8(request).unwrap();
                    assert!(request.to_lowercase().contains("private-token: fixture"));
                    if page == 2 {
                        assert!(request.contains("page=2"));
                    }
                    let paging = if page == 2 {
                        String::new()
                    } else {
                        match style {
                            "link" => {
                                format!("Link: <http://{address}/items?page=2>; rel=\"next\"\r\n")
                            }
                            "number" => "X-Next-Page: 2\r\n".into(),
                            _ => "Link: <http://127.0.0.1:1/stolen>; rel=\"next\"\r\n".into(),
                        }
                    };
                    let body = format!("[{{\"id\":{page}}}]");
                    write!(stream, "HTTP/1.1 200 OK\r\n{paging}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                }
            });
            let result = list(
                &format!("http://{address}/items"),
                "PRIVATE-TOKEN",
                "fixture",
                None,
            );
            if style == "foreign" {
                assert!(result.unwrap_err().contains("outside"));
            } else {
                assert_eq!(
                    result.unwrap(),
                    vec![serde_json::json!({"id":1}), serde_json::json!({"id":2})]
                );
            }
            server.join().unwrap();
        }
        assert_eq!(component("feature/a&b?#"), "feature%2Fa%26b%3F%23");
    }
}
