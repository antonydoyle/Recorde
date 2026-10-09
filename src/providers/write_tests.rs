use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;

struct Reply {
    status: u16,
    body: Value,
    headers: String,
}
fn reply(status: u16, body: Value) -> Reply {
    Reply {
        status,
        body,
        headers: String::new(),
    }
}
fn fixture(replies: Vec<Reply>) -> (String, std::thread::JoinHandle<Vec<(String, Value)>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/project", listener.local_addr().unwrap());
    let handle = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for reply in replies {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && std::time::Instant::now() < deadline =>
                    {
                        std::thread::sleep(std::time::Duration::from_millis(5))
                    }
                    Err(error) => panic!("Missing fixture request: {error}"),
                }
            };
            // Windows inherits the listener's nonblocking mode on accepted sockets.
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 4096];
            let (headers, body_start, length) = loop {
                let count = stream.read(&mut buffer).unwrap();
                assert!(count > 0 && bytes.len() < 100_000);
                bytes.extend_from_slice(&buffer[..count]);
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8(bytes[..end].to_vec()).unwrap();
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            line.to_lowercase()
                                .strip_prefix("content-length:")
                                .map(|value| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    break (headers, end + 4, length);
                }
            };
            while bytes.len() < body_start + length {
                let count = stream.read(&mut buffer).unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&buffer[..count]);
            }
            assert!(headers.contains("fixture-only-token"));
            let body = if length == 0 {
                Value::Null
            } else {
                serde_json::from_slice(&bytes[body_start..body_start + length]).unwrap()
            };
            requests.push((headers.lines().next().unwrap().into(), body));
            let body = reply.body.to_string();
            write!(stream, "HTTP/1.1 {} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{}\r\n{}", reply.status, body.len(), reply.headers, body).unwrap();
        }
        requests
    });
    (url, handle)
}
fn head() -> String {
    "a".repeat(40)
}
fn current(github: bool) -> Value {
    if github {
        json!({"state":"open","head":{"sha":head()},"base":{"ref":"main"},"draft":false})
    } else {
        json!({"state":"opened","sha":head(),"target_branch":"main","draft":false})
    }
}
fn merge() -> Action {
    Action::Merge {
        number: 17,
        head: head(),
        target: "main".into(),
        method: MergeMethod::Squash,
        queue: false,
    }
}
fn create() -> Action {
    Action::Create {
        source_project: String::new(),
        target_project: String::new(),
        title: "Title \"quoted\"".into(),
        body: "first\nsecond".into(),
        source: "feature/topic".into(),
        target: "main".into(),
        draft: true,
    }
}

#[test]
fn fork_creation_names_source_repo_and_resolves_gitlab_target_on_same_api() {
    for github in [true, false] {
        let mut replies = Vec::new();
        if !github {
            replies.push(reply(200, json!({"id":123})));
        }
        replies.push(reply(
            201,
            if github {
                json!({"number":4})
            } else {
                json!({"iid":4})
            },
        ));
        let (base, server) = fixture(replies);
        let mut action = create();
        if let Action::Create {
            source_project,
            target_project,
            ..
        } = &mut action
        {
            *source_project = "contributor/fork".into();
            *target_project = "upstream/project".into();
        }
        execute_at(
            &base,
            &if github {
                ProviderType::GitHub
            } else {
                ProviderType::GitLab
            },
            "fixture-only-token",
            &action,
        )
        .unwrap();
        let requests = server.join().unwrap();
        let body = &requests.last().unwrap().1;
        if github {
            assert_eq!(body["head"], "contributor:feature/topic");
            assert_eq!(body["head_repo"], "fork");
        } else {
            assert!(requests[0].0.contains("upstream%2Fproject"));
            assert_eq!(body["target_project_id"], 123);
        }
    }
}

#[test]
fn merge_queues_pin_the_reviewed_commit_and_do_not_call_direct_merge() {
    for github in [true, false] {
        let mut current = current(github);
        current["node_id"] = json!("PR_fixture");
        let response = if github {
            json!({"data":{"enqueuePullRequest":{"mergeQueueEntry":{"id":"queue-entry"}}}})
        } else {
            json!({"id":42})
        };
        let (base, server) = fixture(vec![reply(200, current), reply(200, response)]);
        let mut action = merge();
        if let Action::Merge { queue, .. } = &mut action {
            *queue = true;
        }
        execute_at(
            &base,
            &if github {
                ProviderType::GitHub
            } else {
                ProviderType::GitLab
            },
            "fixture-only-token",
            &action,
        )
        .unwrap();
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 2);
        let (request, body) = &requests[1];
        if github {
            assert!(request.contains("/graphql"));
            assert_eq!(body["variables"]["input"]["expectedHeadOid"], head());
        } else {
            assert!(request.contains("/merge_trains/merge_requests/17"));
            assert_eq!(body["sha"], head());
            assert_eq!(body["auto_merge"], false);
        }
    }
}

#[test]
fn inline_comments_use_exact_commit_and_native_diff_coordinates() {
    for github in [true, false] {
        let mut replies = vec![reply(200, current(github))];
        if !github {
            replies.push(reply(200, json!([{"head_commit_sha":head(),"base_commit_sha":"b".repeat(40),"start_commit_sha":"c".repeat(40)}])));
        }
        replies.push(reply(201, json!({"id":123})));
        let (base, server) = fixture(replies);
        let action = Action::Review {
            number: 17,
            head: head(),
            target: "main".into(),
            body: "line comment".into(),
            decision: Decision::Comment,
            inline: Some(InlineComment {
                path: "new name.rs".into(),
                old_path: "old name.rs".into(),
                new_line: 17,
                old_line: 0,
            }),
        };
        execute_at(
            &base,
            &if github {
                ProviderType::GitHub
            } else {
                ProviderType::GitLab
            },
            "fixture-only-token",
            &action,
        )
        .unwrap();
        let requests = server.join().unwrap();
        let (request, body) = requests.last().unwrap();
        if github {
            assert!(request.contains("/pulls/17/comments"));
            assert_eq!(body["commit_id"], head());
            assert_eq!(body["line"], 17);
            assert_eq!(body["side"], "RIGHT");
        } else {
            assert!(request.contains("/merge_requests/17/discussions"));
            assert_eq!(body["position"]["head_sha"], head());
            assert_eq!(body["position"]["new_line"], 17);
            assert!(body["position"].get("old_line").is_none());
            assert_eq!(body["position"]["old_path"], "old name.rs");
        }
    }
}

#[test]
fn github_merge_and_review_bind_to_expected_commit() {
    let (base, server) = fixture(vec![
        reply(200, current(true)),
        reply(200, json!({"merged":true})),
    ]);
    assert!(
        execute_at(&base, &ProviderType::GitHub, "fixture-only-token", &merge())
            .unwrap()
            .contains("Merged")
    );
    let requests = server.join().unwrap();
    assert_eq!(requests[1].0, "PUT /project/pulls/17/merge HTTP/1.1");
    assert_eq!(requests[1].1, json!({"sha":head(),"merge_method":"squash"}));
    let (base, server) = fixture(vec![reply(200, current(true)), reply(200, json!({"id":9}))]);
    let review = Action::Review {
        number: 17,
        head: head(),
        target: "main".into(),
        body: "Please fix\nthis".into(),
        decision: Decision::RequestChanges,
        inline: None,
    };
    execute_at(&base, &ProviderType::GitHub, "fixture-only-token", &review).unwrap();
    let requests = server.join().unwrap();
    assert_eq!(
        requests[1].1,
        json!({"commit_id":head(),"body":"Please fix\nthis","event":"REQUEST_CHANGES"})
    );
}

#[test]
fn changed_head_target_and_draft_refuse_merge_before_any_write() {
    for change in [
        json!({"state":"open","head":{"sha":"b".repeat(40)},"base":{"ref":"main"}}),
        json!({"state":"open","head":{"sha":head()},"base":{"ref":"other"}}),
        json!({"state":"open","head":{"sha":head()},"base":{"ref":"main"},"draft":true}),
    ] {
        let (base, server) = fixture(vec![reply(200, change)]);
        assert!(execute_at(&base, &ProviderType::GitHub, "fixture-only-token", &merge()).is_err());
        assert_eq!(server.join().unwrap().len(), 1);
    }
}

#[test]
fn create_payloads_preserve_text_and_draft_intent_for_both_providers() {
    for (provider, result) in [
        (ProviderType::GitHub, json!({"number":7})),
        (ProviderType::GitLab, json!({"iid":7})),
    ] {
        let (base, server) = fixture(vec![reply(201, result)]);
        assert_eq!(
            execute_at(&base, &provider, "fixture-only-token", &create()).unwrap(),
            "Created review #7."
        );
        let requests = server.join().unwrap();
        let body = &requests[0].1;
        if provider == ProviderType::GitHub {
            assert_eq!(body["head"], "feature/topic");
            assert_eq!(body["draft"], true);
            assert_eq!(body["body"], "first\nsecond");
        } else {
            assert_eq!(body["source_branch"], "feature/topic");
            assert_eq!(body["title"], "Draft: Title \"quoted\"");
            assert_eq!(body["description"], "first\nsecond");
        }
    }
}

#[test]
fn gitlab_approval_and_merge_send_sha_without_deleting_source() {
    let (base, server) = fixture(vec![
        reply(200, current(false)),
        reply(200, json!({"approved":true})),
    ]);
    let approval = Action::Review {
        number: 17,
        head: head(),
        target: "main".into(),
        body: String::new(),
        decision: Decision::Approve,
        inline: None,
    };
    execute_at(
        &base,
        &ProviderType::GitLab,
        "fixture-only-token",
        &approval,
    )
    .unwrap();
    let requests = server.join().unwrap();
    assert_eq!(
        requests[1].0,
        "POST /project/merge_requests/17/approve HTTP/1.1"
    );
    assert_eq!(requests[1].1, json!({"sha":head()}));
    let (base, server) = fixture(vec![
        reply(200, current(false)),
        reply(200, json!({"state":"merged"})),
    ]);
    execute_at(&base, &ProviderType::GitLab, "fixture-only-token", &merge()).unwrap();
    let requests = server.join().unwrap();
    assert_eq!(
        requests[1].1,
        json!({"sha":head(),"squash":true,"auto_merge":false,"should_remove_source_branch":false})
    );
}

#[test]
fn permissions_rate_limits_and_redirects_are_not_retried_and_redact_tokens() {
    let destination = TcpListener::bind("127.0.0.1:0").unwrap();
    destination.set_nonblocking(true).unwrap();
    for status in [403, 429, 302] {
        let mut response = reply(
            status,
            json!({"message":"fixture-only-token must not leak"}),
        );
        response.headers = format!(
            "Retry-After: 60\r\nLocation: http://{}/stolen\r\n",
            destination.local_addr().unwrap()
        );
        let (base, server) = fixture(vec![response]);
        let error = execute_at(
            &base,
            &ProviderType::GitHub,
            "fixture-only-token",
            &create(),
        )
        .unwrap_err();
        assert!(error.contains(&format!("HTTP {status}")));
        assert!(!error.contains("fixture-only-token"));
        assert_eq!(server.join().unwrap().len(), 1);
    }
    assert_eq!(
        destination.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}
