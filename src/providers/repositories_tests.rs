use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

fn account(github: bool) -> StoredAccount {
    StoredAccount {
        provider: if github {
            ProviderKind::GitHubCom
        } else {
            ProviderKind::GitLabCom
        },
        host: if github { "github.com" } else { "gitlab.com" }.into(),
        username: "alice".into(),
    }
}

fn record(github: bool, path: &str) -> Value {
    if github {
        json!({"full_name": path, "description": "A fixture repository", "private": true,
        "clone_url": format!("https://github.com/{path}.git"), "ssh_url": format!("git@github.com:{path}.git"),
        "html_url": format!("https://github.com/{path}"), "archived": false, "fork": true})
    } else {
        json!({"path_with_namespace": path, "description": null, "visibility": "private",
        "http_url_to_repo": format!("https://gitlab.com/{path}.git"), "ssh_url_to_repo": format!("git@gitlab.com:{path}.git"),
        "web_url": format!("https://gitlab.com/{path}"), "archived": true, "forked_from_project": {"id": 1}})
    }
}

/// Local-only API fixture with bounded accept/read time and no helper processes.
fn server(
    status: u16,
    extra_headers: &str,
    body: Value,
) -> (String, std::thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let extra_headers = extra_headers.to_string();
    let handle = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(5))
                }
                Err(error) => panic!("fixture accept: {error}"),
            }
        };
        // Accepted sockets inherit the listener's non-blocking mode on Windows and macOS.
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut bytes = Vec::new();
        let end = loop {
            let mut chunk = [0; 1024];
            let read = stream.read(&mut chunk).unwrap();
            assert!(read > 0 && bytes.len() < 16384);
            bytes.extend_from_slice(&chunk[..read]);
            if let Some(index) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                break index + 4;
            }
        };
        let header = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
        let length = header
            .lines()
            .find_map(|line| {
                line.strip_prefix("content-length:")
                    .map(|length| length.trim().parse::<usize>().unwrap())
            })
            .unwrap_or(0);
        while bytes.len() < end + length {
            let mut chunk = [0; 1024];
            let read = stream.read(&mut chunk).unwrap();
            assert!(read > 0 && bytes.len() < 16384);
            bytes.extend_from_slice(&chunk[..read]);
        }
        let body = body.to_string();
        write!(stream, "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\n{extra_headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        String::from_utf8(bytes).unwrap()
    });
    (format!("http://{address}"), handle)
}

#[test]
fn repository_search_encodes_inputs_and_bounds_pages() {
    assert!(
        list_url("https://api.github.com", true, "hello&token=bad", 1)
            .unwrap()
            .contains("q=hello%26token%3Dbad")
    );
    assert!(list_url("https://api.github.com", true, "", 0).is_err());
    assert!(list_url("https://api.github.com", true, "", 21).is_err());
    assert!(list_url("https://api.github.com", true, "line\nbreak", 1).is_err());
}

#[test]
fn account_repository_lists_use_authenticated_endpoints_and_map_clone_metadata() {
    for gh in [true, false] {
        let owner = if gh {
            "team/project"
        } else {
            "team/subgroup/project"
        };
        let (base, fixture) = server(200, "", json!([record(gh, owner)]));
        let page = list_at(&base, &account(gh), "fixture-token", "", 1).unwrap();
        let request = fixture.join().unwrap();
        assert!(request
            .to_lowercase()
            .contains("authorization: bearer fixture-token"));
        assert!(request.starts_with(if gh {
            "GET /user/repos?"
        } else {
            "GET /projects?membership=true&"
        }));
        assert_eq!(page.next_page, None);
        assert!(!page.limited);
        let repository = &page.repositories[0];
        assert_eq!(repository.path, owner);
        assert!(repository.private && repository.fork);
        assert_eq!(repository.archived, !gh);
        assert!(clone_url(repository, &account(gh), false)
            .unwrap()
            .starts_with("https://alice@"));
        assert!(clone_url(repository, &account(gh), true)
            .unwrap()
            .starts_with("git@"));
    }
}

#[test]
fn search_pagination_and_incomplete_results_are_explicit() {
    let records: Vec<_> = (0..PAGE_SIZE)
        .map(|i| record(true, &format!("team/project{i}")))
        .collect();
    for page in [1, MAX_PAGES] {
        let (base, fixture) = server(
            200,
            "",
            json!({"items": records, "incomplete_results": false}),
        );
        let result = list_at(
            &base,
            &account(true),
            "fixture-token",
            "owner:team & text",
            page,
        )
        .unwrap();
        let request = fixture.join().unwrap();
        assert!(request.starts_with("GET /search/repositories?q=owner%3Ateam%20%26%20text&"));
        assert_eq!(result.next_page, (page < MAX_PAGES).then_some(page + 1));
        assert_eq!(result.limited, page == MAX_PAGES);
    }
    let (base, fixture) = server(200, "", json!({"items": [], "incomplete_results": true}));
    assert!(list_at(&base, &account(true), "fixture-token", "query", 1)
        .unwrap_err()
        .contains("incomplete"));
    fixture.join().unwrap();
}

#[test]
fn personal_forks_send_one_post_with_no_organization_override() {
    for gh in [true, false] {
        let source = parse(&record(gh, "team/project"), &account(gh)).unwrap();
        let (base, fixture) = server(if gh { 202 } else { 201 }, "", record(gh, "alice/project"));
        let result = fork_at(&base, &account(gh), "fixture-token", &source).unwrap();
        let request = fixture.join().unwrap();
        assert!(request.starts_with(if gh {
            "POST /repos/team/project/forks "
        } else {
            "POST /projects/team%2Fproject/fork "
        }));
        assert!(request
            .to_lowercase()
            .contains("authorization: bearer fixture-token"));
        assert_eq!(request.split("\r\n\r\n").nth(1), Some("{}"));
        assert_eq!(result.path, "alice/project");
    }
}

#[test]
fn unsafe_provider_destinations_and_mismatched_identity_are_rejected() {
    let mut bad = account(true);
    bad.host = "github.com.evil.invalid".into();
    assert!(api_base(&bad).is_err());
    bad.provider = ProviderKind::GitHubEnterprise;
    bad.host = "user:secret@host.invalid".into();
    assert!(api_base(&bad).is_err());
    bad.provider = ProviderKind::BitbucketCloud;
    assert!(api_base(&bad)
        .unwrap_err()
        .contains("available for GitHub and GitLab"));
    for url in [
        "https://evil.invalid/team/project.git",
        "https://user:secret@github.com/team/project.git",
        "https://github.com/team/project.git?token=secret",
        "https://github.com/other/project.git",
    ] {
        let mut value = record(true, "team/project");
        value["clone_url"] = url.into();
        value["ssh_url"] = Value::Null;
        assert!(parse(&value, &account(true)).is_err(), "{url}");
    }
    assert!(validate_path("team/../project", false).is_err());
    assert!(validate_path("team/subgroup/project", true).is_err());
}

#[test]
fn redirects_and_fork_permission_errors_do_not_leak_credentials_or_retry() {
    let (base, fixture) = server(302, "Location: http://127.0.0.1:1/stolen\r\n", json!({}));
    assert!(list_at(&base, &account(true), "fixture-token", "", 1).is_err());
    fixture.join().unwrap();
    let repository = parse(&record(true, "team/project"), &account(true)).unwrap();
    let (base, fixture) = server(403, "", json!({"message": "denied fixture-token"}));
    let error = fork_at(&base, &account(true), "fixture-token", &repository).unwrap_err();
    assert!(error.contains("403") && !error.contains("fixture-token"));
    fixture.join().unwrap();
}
