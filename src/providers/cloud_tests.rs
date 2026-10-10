use super::*;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

fn fixture(
    replies: Vec<(u16, Value, String)>,
) -> (String, std::thread::JoinHandle<Vec<(String, Value)>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!(
        "http://{}/org/project/_apis/git/repositories/repo",
        listener.local_addr().unwrap()
    );
    let handle = std::thread::spawn(move || {
        let mut requests = vec![];
        for (status, body, extra) in replies {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(e) => panic!("Missing fixture request: {e}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = vec![];
            let mut buffer = [0; 4096];
            let (headers, start, length) = loop {
                let count = stream.read(&mut buffer).unwrap();
                assert!(count > 0 && bytes.len() < 100_000);
                bytes.extend_from_slice(&buffer[..count]);
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8(bytes[..end].to_vec()).unwrap();
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|s| s.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    break (headers, end + 4, length);
                }
            };
            while bytes.len() < start + length {
                let count = stream.read(&mut buffer).unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&buffer[..count]);
            }
            assert!(headers
                .to_ascii_lowercase()
                .contains(&format!("authorization: {}", auth().to_ascii_lowercase())));
            assert!(!headers.lines().next().unwrap().contains("fixture-secret"));
            let payload = if length == 0 {
                Value::Null
            } else {
                serde_json::from_slice(&bytes[start..start + length]).unwrap()
            };
            requests.push((headers.lines().next().unwrap().into(), payload));
            let body = body.to_string();
            write!(stream, "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n{body}", body.len()).unwrap();
        }
        requests
    });
    (url, handle)
}
fn auth() -> String {
    bitbucket::basic("fixture@example.test", "fixture-secret")
}
fn head() -> String {
    "a".repeat(40)
}
fn current(azure: bool) -> Value {
    if azure {
        json!({"status":"active","lastMergeSourceCommit":{"commitId":head()},"targetRefName":"refs/heads/main"})
    } else {
        json!({"state":"OPEN","source":{"commit":{"hash":head()}},"destination":{"branch":{"name":"main"}}})
    }
}
fn reply(body: Value) -> (u16, Value, String) {
    (200, body, String::new())
}
fn review() -> write::Action {
    write::Action::Review {
        number: 7,
        head: head(),
        target: "main".into(),
        body: "Review comment".into(),
        decision: write::Decision::Comment,
        inline: None,
    }
}
fn execute(azure_provider: bool, base: &str, action: &write::Action) -> Result<String, String> {
    if azure_provider {
        azure::execute(base, &auth(), action)
    } else {
        bitbucket::execute(base, &auth(), action)
    }
}

#[test]
fn cloud_writes_stop_before_mutation_on_stale_head_target_or_state() {
    for azure_provider in [false, true] {
        for change in ["head", "target", "state"] {
            let mut value = current(azure_provider);
            match (azure_provider, change) {
                (true, "head") => {
                    value["lastMergeSourceCommit"]["commitId"] = json!("b".repeat(40))
                }
                (true, "target") => value["targetRefName"] = json!("refs/heads/other"),
                (true, _) => value["status"] = json!("completed"),
                (false, "head") => value["source"]["commit"]["hash"] = json!("b".repeat(40)),
                (false, "target") => value["destination"]["branch"]["name"] = json!("other"),
                (false, _) => value["state"] = json!("MERGED"),
            }
            let (base, server) = fixture(vec![reply(value)]);
            assert!(execute(azure_provider, &base, &review())
                .unwrap_err()
                .contains("changed"));
            assert_eq!(server.join().unwrap().len(), 1);
        }
    }
}

#[test]
fn cloud_comments_confirm_submission_and_detect_post_write_head_changes() {
    for azure_provider in [false, true] {
        for changed in [false, true] {
            let mut after = current(azure_provider);
            if changed {
                if azure_provider {
                    after["lastMergeSourceCommit"]["commitId"] = json!("b".repeat(40));
                } else {
                    after["source"]["commit"]["hash"] = json!("b".repeat(40));
                }
            }
            let (base, server) = fixture(vec![
                reply(current(azure_provider)),
                reply(json!({"id":10})),
                reply(after),
            ]);
            let result = execute(azure_provider, &base, &review());
            if changed {
                assert!(result.unwrap_err().contains("submitted"));
            } else {
                assert!(result.is_ok());
            }
            let requests = server.join().unwrap();
            assert_eq!(requests.len(), 3);
            assert!(requests[1].0.starts_with("POST "));
            assert_eq!(
                if azure_provider {
                    &requests[1].1["comments"][0]["content"]
                } else {
                    &requests[1].1["content"]["raw"]
                },
                "Review comment"
            );
        }
    }
}

#[test]
fn azure_merge_sends_atomic_head_guard_and_never_bypasses_policies() {
    let (base, server) = fixture(vec![
        reply(current(true)),
        reply(json!({"status":"completed"})),
    ]);
    let action = write::Action::Merge {
        number: 7,
        head: head(),
        target: "main".into(),
        method: write::MergeMethod::Squash,
        queue: false,
    };
    assert!(azure::execute(&base, &auth(), &action).is_ok());
    let requests = server.join().unwrap();
    assert!(requests[1].0.starts_with("PATCH "));
    assert_eq!(requests[1].1["lastMergeSourceCommit"]["commitId"], head());
    assert_eq!(requests[1].1["completionOptions"]["bypassPolicy"], false);
    assert_eq!(
        requests[1].1["completionOptions"]["deleteSourceBranch"],
        false
    );
    assert_eq!(
        requests[1].1["completionOptions"]["mergeStrategy"],
        "squash"
    );
    assert!(
        write::validate(&action, &crate::git::capabilities::ProviderType::Bitbucket)
            .unwrap_err()
            .contains("atomically")
    );
    assert!(bitbucket::execute("http://127.0.0.1:1", &auth(), &action)
        .unwrap_err()
        .contains("atomic"));
}

#[test]
fn cloud_create_preserves_drafts_and_source_destination() {
    for azure_provider in [false, true] {
        let (base, server) = fixture(vec![reply(if azure_provider {
            json!({"pullRequestId":9})
        } else {
            json!({"id":9})
        })]);
        let action = write::Action::Create {
            source_project: if azure_provider {
                "org/project/repo"
            } else {
                "workspace/repo"
            }
            .into(),
            target_project: if azure_provider {
                "org/project/repo"
            } else {
                "workspace/repo"
            }
            .into(),
            title: "Title".into(),
            body: "Body".into(),
            source: "feature/a".into(),
            target: "main".into(),
            draft: true,
        };
        assert!(execute(azure_provider, &base, &action)
            .unwrap()
            .contains("#9"));
        let requests = server.join().unwrap();
        let body = &requests[0].1;
        if azure_provider {
            assert_eq!(body["sourceRefName"], "refs/heads/feature/a");
            assert_eq!(body["isDraft"], true);
        } else {
            assert_eq!(body["source"]["branch"]["name"], "feature/a");
            assert_eq!(body["draft"], true);
            assert_eq!(body["close_source_branch"], false);
        }
    }
}

#[test]
fn cloud_auth_redirects_and_provider_errors_never_leak_credentials() {
    let destination = TcpListener::bind("127.0.0.1:0").unwrap();
    destination.set_nonblocking(true).unwrap();
    for status in [302, 401, 403, 429] {
        let (base, server) = fixture(vec![(
            status,
            json!({"message":auth()}),
            format!(
                "Location: http://{}/stolen\r\n",
                destination.local_addr().unwrap()
            ),
        )]);
        let error = execute(true, &base, &review()).unwrap_err();
        assert!(!error.contains(&auth()));
        assert!(!error.contains("fixture-secret"));
        server.join().unwrap();
    }
    assert_eq!(
        destination.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[test]
fn cloud_pagination_follows_next_continuation_and_skip_without_leaking_auth() {
    for style in ["next", "continuation", "skip", "foreign", "repeat"] {
        let first = if matches!(style, "next" | "foreign" | "repeat") {
            let next = match style {
                "foreign" => "http://127.0.0.1:1/stolen",
                "repeat" => "?pagelen=1",
                _ => "?page=2",
            };
            json!({"values":[{"id":1}],"next":next})
        } else {
            json!({"value":[{"id":1}]})
        };
        let extra = if style == "continuation" {
            "x-ms-continuationtoken: opaque/+ token\r\n".into()
        } else {
            String::new()
        };
        let mut replies = vec![(200, first, extra)];
        if !matches!(style, "foreign" | "repeat") {
            replies.push(reply(if style == "next" {
                json!({"values":[{"id":2}]})
            } else if style == "skip" {
                json!({"value":[]})
            } else {
                json!({"value":[{"id":2}]})
            }));
        }
        let (base, server) = fixture(replies);
        let query = if style == "skip" {
            "$top=1"
        } else {
            "pagelen=1"
        };
        let key = if matches!(style, "next" | "foreign" | "repeat") {
            "values"
        } else {
            "value"
        };
        let result = pagination::list(
            &format!("{base}/pullrequests?{query}"),
            "Authorization",
            &auth(),
            Some(key),
        );
        if matches!(style, "foreign" | "repeat") {
            assert!(result.is_err());
        } else {
            assert_eq!(result.unwrap().len(), if style == "skip" { 1 } else { 2 });
        }
        let requests = server.join().unwrap();
        if style == "skip" {
            assert!(requests[1].0.contains("%24skip=1"));
        }
        if style == "continuation" {
            assert!(requests[1]
                .0
                .contains("continuationToken=opaque%2F%2B+token"));
        }
    }
}

#[test]
fn cloud_parsers_keep_terminal_drafts_and_review_rejections() {
    let bb = json!({"id":1,"title":"Draft","draft":true,"state":"MERGED","participants":[{"approved":true},{"state":"changes_requested"}]});
    let az = json!({"pullRequestId":1,"title":"Draft","isDraft":true,"status":"abandoned","reviewers":[{"vote":10},{"vote":-5}]});
    assert_eq!(bitbucket::parse_pr(&bb).unwrap().state, PrState::Merged);
    assert_eq!(
        bitbucket::parse_pr(&bb).unwrap().review_state,
        Some(ReviewState::ChangesRequested)
    );
    assert_eq!(azure::parse_pr(&az).unwrap().state, PrState::Closed);
    assert_eq!(
        azure::parse_pr(&az).unwrap().review_state,
        Some(ReviewState::ChangesRequested)
    );
    assert_eq!(bitbucket::basic("email", "token"), "Basic ZW1haWw6dG9rZW4=");
    assert_eq!(bitbucket::basic("", "token"), "Basic OnRva2Vu");
}

#[test]
fn azure_fork_creation_resolves_source_on_the_same_origin_and_organization() {
    let (base, server) = fixture(vec![
        reply(json!({"id":"fork-id"})),
        reply(json!({"pullRequestId":11})),
    ]);
    let action = write::Action::Create {
        source_project: "org/Other Project/fork".into(),
        target_project: "org/project/repo".into(),
        title: "Fork change".into(),
        body: String::new(),
        source: "feature".into(),
        target: "main".into(),
        draft: false,
    };
    write::validate(
        &action,
        &crate::git::capabilities::ProviderType::AzureDevOps,
    )
    .unwrap();
    azure::execute(&base, &auth(), &action).unwrap();
    let requests = server.join().unwrap();
    assert!(requests[0]
        .0
        .contains("/org/Other%20Project/_apis/git/repositories/fork?api-version=7.1"));
    assert_eq!(requests[1].1["forkSource"]["repository"]["id"], "fork-id");
    assert_eq!(requests[1].1["forkSource"]["name"], "refs/heads/feature");
}

#[test]
fn azure_inline_comments_use_verified_iteration_and_decline_stale_iterations() {
    for changed in [false, true] {
        let iteration = json!({"value":[{"id":3,"sourceRefCommit":{"commitId":if changed { "b".repeat(40) } else { head() }}}]});
        let mut replies = vec![reply(current(true)), reply(iteration)];
        if !changed {
            replies.extend([reply(json!({"changeEntries":[{"changeTrackingId":5,"item":{"path":"/src/code.rs"}}],"nextSkip":0})), reply(json!({"id":12})), reply(current(true))]);
        }
        let (base, server) = fixture(replies);
        let mut action = review();
        if let write::Action::Review { inline, .. } = &mut action {
            *inline = Some(write::InlineComment {
                path: "src/code.rs".into(),
                new_line: 8,
                ..Default::default()
            });
        }
        let result = azure::execute(&base, &auth(), &action);
        if changed {
            assert!(result.unwrap_err().contains("iteration changed"));
        } else {
            assert!(result.is_ok());
        }
        let requests = server.join().unwrap();
        if !changed {
            assert_eq!(requests[3].1["threadContext"]["rightFileStart"]["line"], 8);
            assert_eq!(
                requests[3].1["pullRequestThreadContext"]["iterationContext"]
                    ["secondComparingIteration"],
                3
            );
        }
    }
}

#[test]
fn cloud_review_votes_use_selected_authenticated_identity_and_confirm_outcome() {
    for azure_provider in [false, true] {
        for approve in [false, true] {
            let vote = if approve { 10 } else { -10 };
            let mut replies = vec![reply(current(azure_provider))];
            if azure_provider {
                replies.push(reply(json!({"authenticatedUser":{"id":"selected-user"}})));
            }
            replies.push(reply(if azure_provider {
                json!({"vote":vote})
            } else if approve {
                json!({"approved":true})
            } else {
                json!({"state":"changes_requested"})
            }));
            replies.push(reply(current(azure_provider)));
            let (base, server) = fixture(replies);
            let mut action = review();
            if let write::Action::Review { body, decision, .. } = &mut action {
                body.clear();
                *decision = if approve {
                    write::Decision::Approve
                } else {
                    write::Decision::RequestChanges
                };
            }
            execute(azure_provider, &base, &action).unwrap();
            let requests = server.join().unwrap();
            if azure_provider {
                assert!(requests[1].0.contains("/org/_apis/connectionData"));
                assert!(requests[2].0.contains("/reviewers/selected%2Duser?"));
                assert_eq!(requests[2].1["vote"], vote);
            } else {
                assert!(requests[1].0.contains(if approve {
                    "/approve "
                } else {
                    "/request-changes "
                }));
            }
        }
    }
}
