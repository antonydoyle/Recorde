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
        json!({"status":"active","lastMergeSourceCommit":{"commitId":head()},"targetRefName":"refs/heads/main","sourceRefName":"refs/heads/topic"})
    } else {
        json!({"state":"OPEN","source":{"commit":{"hash":head()}},"destination":{"branch":{"name":"main"}}})
    }
}
fn source_ref() -> (u16, Value, String) {
    reply(json!({"value":[{"name":"refs/heads/topic","objectId":head()}]}))
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
            let mut replies = vec![reply(current(azure_provider))];
            if azure_provider {
                replies.push(source_ref());
            }
            replies.extend([reply(json!({"id":10})), reply(after)]);
            if azure_provider && !changed {
                replies.push(source_ref());
            }
            let (base, server) = fixture(replies);
            let result = execute(azure_provider, &base, &review());
            if changed {
                assert!(result.unwrap_err().contains("submitted"));
            } else {
                assert!(result.is_ok());
            }
            let requests = server.join().unwrap();
            assert_eq!(
                requests.len(),
                if azure_provider {
                    if changed {
                        4
                    } else {
                        5
                    }
                } else {
                    3
                }
            );
            let post = requests.iter().find(|r| r.0.starts_with("POST ")).unwrap();
            assert_eq!(
                if azure_provider {
                    &post.1["comments"][0]["content"]
                } else {
                    &post.1["content"]["raw"]
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
        source_ref(),
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
    assert!(requests[2].0.starts_with("PATCH "));
    assert_eq!(requests[2].1["lastMergeSourceCommit"]["commitId"], head());
    assert_eq!(requests[2].1["completionOptions"]["bypassPolicy"], false);
    assert_eq!(
        requests[2].1["completionOptions"]["deleteSourceBranch"],
        false
    );
    assert_eq!(
        requests[2].1["completionOptions"]["mergeStrategy"],
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
        let mut replies = vec![reply(current(true)), source_ref(), reply(iteration)];
        if !changed {
            replies.extend([reply(json!({"changeEntries":[{"changeTrackingId":5,"item":{"path":"/src/code.rs"}}],"nextSkip":0})), reply(json!({"id":12})), reply(current(true)), source_ref()]);
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
            assert_eq!(requests[4].1["threadContext"]["rightFileStart"]["line"], 8);
            assert_eq!(
                requests[4].1["pullRequestThreadContext"]["iterationContext"]
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
                replies.push(source_ref());
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
            if azure_provider {
                replies.push(source_ref());
            }
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
                assert!(requests[2]
                    .0
                    .contains("/org/_apis/connectionData?api-version=7.2-preview.1"));
                assert!(requests[3].0.contains("/reviewers/selected%2Duser?"));
                assert_eq!(requests[3].1["vote"], vote);
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

#[test]
fn bitbucket_short_heads_resolve_on_trusted_source_repository_before_review() {
    for full in [head(), "b".repeat(40), "a".repeat(12)] {
        let mut short = current(false);
        short["source"]["commit"]["hash"] = json!("a".repeat(12));
        short["source"]["repository"] = json!({"full_name":"fork/source","links":{"self":{"href":"https://attacker.invalid/steal"}}});
        let mut replies = vec![reply(short.clone()), reply(json!({"hash":full}))];
        if full == head() {
            replies.extend([
                reply(json!({"id":5})),
                reply(short),
                reply(json!({"hash":full})),
            ]);
        }
        let (base, server) = fixture(replies);
        let result = bitbucket::execute(&base, &auth(), &review());
        assert_eq!(result.is_ok(), full == head());
        let requests = server.join().unwrap();
        assert!(requests[1]
            .0
            .contains("/repositories/fork/source/commit/aaaaaaaaaaaa "));
        assert_eq!(
            requests.iter().filter(|r| r.0.starts_with("POST ")).count(),
            usize::from(full == head())
        );
    }
}

#[test]
fn azure_live_ref_blocks_review_when_last_test_merge_lags_a_push() {
    let (base, server) = fixture(vec![
        reply(current(true)),
        reply(json!({"value":[{"name":"refs/heads/topic","objectId":"b".repeat(40)}]})),
    ]);
    assert!(azure::execute(&base, &auth(), &review())
        .unwrap_err()
        .contains("Source branch changed"));
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[1]
        .0
        .contains("/refs?filter=heads%2Ftopic&api-version=7.1"));
}

#[test]
fn azure_fork_live_ref_uses_repository_id_and_exact_branch() {
    let mut body = current(true);
    body["forkSource"] = json!({"repository":{"id":"fork-id","url":"https://attacker.invalid"}});
    let (base, server) = fixture(vec![reply(
        json!({"value":[{"name":"refs/heads/topic-other","objectId":"b".repeat(40)},{"name":"refs/heads/topic","objectId":head()}]}),
    )]);
    assert_eq!(
        azure::live_source_head(&base, &body, &auth()).unwrap(),
        head()
    );
    assert!(server.join().unwrap()[0]
        .0
        .contains("/org/_apis/git/repositories/fork%2Did/refs?"));
}

#[test]
fn azure_merge_reports_rejected_and_accepted_unconfirmed_results() {
    for (status, body, accepted) in [
        (409, json!({"message":"stale head"}), false),
        (400, json!({"message":"policy rejection"}), false),
        (200, json!({"status":"active"}), true),
        (200, json!({"mergeStatus":"queued"}), true),
    ] {
        let (base, server) = fixture(vec![
            reply(current(true)),
            source_ref(),
            (status, body, String::new()),
        ]);
        let action = write::Action::Merge {
            number: 7,
            head: head(),
            target: "main".into(),
            method: write::MergeMethod::Squash,
            queue: false,
        };
        let result = azure::execute(&base, &auth(), &action);
        assert_eq!(result.is_ok(), accepted);
        if accepted {
            assert!(result.unwrap().contains("unconfirmed"));
        }
        assert_eq!(server.join().unwrap().len(), 3);
    }
}

#[test]
fn bitbucket_statuses_dedupe_latest_context_and_unknown_remains_pending() {
    assert_eq!(
        bitbucket::checks(&[
            json!({"key":"ci","updated_on":"2026-01-01","state":"FAILED"}),
            json!({"key":"ci","updated_on":"2026-01-02","state":"SUCCESSFUL"})
        ]),
        Some(CheckState::Success)
    );
    assert_eq!(
        bitbucket::checks(&[json!({"key":"ci","state":"NEW_STATE"})]),
        Some(CheckState::Pending)
    );
}

#[test]
fn bitbucket_nested_error_is_visible_and_redacted() {
    let (base, server) = fixture(vec![(
        400,
        json!({"error":{"message":format!("Denied {}",auth())}}),
        String::new(),
    )]);
    let error = bitbucket::execute(&base, &auth(), &review()).unwrap_err();
    assert!(error.contains("Denied"));
    assert!(!error.contains(&auth()));
    server.join().unwrap();
}

#[test]
fn bitbucket_inline_comments_preserve_old_and_new_line_sides() {
    for old in [false, true] {
        let (base, server) = fixture(vec![
            reply(current(false)),
            reply(json!({"id":2})),
            reply(current(false)),
        ]);
        let mut action = review();
        if let write::Action::Review { inline, .. } = &mut action {
            *inline = Some(write::InlineComment {
                path: "src/code.rs".into(),
                old_line: if old { 5 } else { 0 },
                new_line: if old { 0 } else { 6 },
                ..Default::default()
            });
        }
        bitbucket::execute(&base, &auth(), &action).unwrap();
        let requests = server.join().unwrap();
        assert_eq!(
            requests[1].1["inline"][if old { "from" } else { "to" }],
            if old { 5 } else { 6 }
        );
        assert_eq!(requests[1].1["inline"]["path"], "src/code.rs");
    }
}

#[test]
fn cloud_ci_lists_use_branch_filters_and_map_terminal_states() {
    let (base, server) = fixture(vec![reply(
        json!({"values":[{"build_number":12,"state":{"result":{"name":"FAILED"}},"target":{"ref_name":"topic/a"}}]}),
    )]);
    let runs = bitbucket::list_ci_from(&base, "workspace", "repo", "topic/a", &auth()).unwrap();
    assert_eq!(runs[0].status, CiStatus::Failure);
    assert!(server.join().unwrap()[0]
        .0
        .contains("target.ref_name=topic%2Fa"));
    let (base, server) = fixture(vec![
        reply(json!({"id":"repo-id"})),
        reply(
            json!({"value":[{"id":13,"definition":{"name":"Build"},"result":"canceled","sourceBranch":"refs/heads/topic/a"}]}),
        ),
    ]);
    let runs = azure::list_ci_from(&base, &base, "topic/a", &auth()).unwrap();
    assert_eq!(runs[0].status, CiStatus::Cancelled);
    assert_eq!(runs[0].branch, "topic/a");
    assert!(server.join().unwrap()[1].0.contains(
        "repositoryId=repo%2Did&repositoryType=TfsGit&branchName=refs%2Fheads%2Ftopic%2Fa"
    ));
}

#[test]
fn azure_boards_bounds_queries_and_requires_known_process_categories() {
    let (base, server) = fixture(vec![reply(
        json!({"workItems":(0..2001).map(|id|json!({"id":id})).collect::<Vec<_>>()}),
    )]);
    assert!(azure::list_issues_from(&base, &auth(), "all")
        .unwrap_err()
        .contains("2000"));
    server.join().unwrap();
    for category in ["Completed", "Proposed", "Unexpected"] {
        let (base, server) = fixture(vec![
            reply(json!({"workItems":[{"id":1}]})),
            reply(
                json!({"value":[{"id":1,"fields":{"System.Title":"Item","System.State":"Custom","System.WorkItemType":"User Story"}}]}),
            ),
            reply(json!({"value":[{"name":"Custom","category":category}]})),
        ]);
        let items = azure::list_issues_from(&base, &auth(), "all");
        if category == "Unexpected" {
            assert!(items.unwrap_err().contains("Unknown"));
        } else {
            assert_eq!(
                items.unwrap()[0].state,
                if category == "Completed" {
                    IssueState::Closed
                } else {
                    IssueState::Open
                }
            );
        }
        let requests = server.join().unwrap();
        assert_eq!(requests[1].1["ids"], json!([1]));
        assert!(requests[2].0.contains("workitemtypes/User%20Story/states?"));
    }
}

#[test]
fn azure_classic_releases_use_separate_trusted_host_and_parse_drafts() {
    assert_eq!(
        azure::release_base("org/My Project").unwrap(),
        "https://vsrm.dev.azure.com/org/My%20Project"
    );
    let (base, server) = fixture(vec![reply(
        json!({"value":[{"id":2,"name":"Release","status":"draft","description":"Notes","_links":{"web":{"href":"https://dev.azure.com/org/project/_release?releaseId=2"}}}]}),
    )]);
    let releases = azure::list_releases_from(&base, &auth()).unwrap();
    assert!(releases[0].prerelease);
    assert_eq!(releases[0].name, "Release");
    assert!(server.join().unwrap()[0]
        .0
        .contains("/_apis/release/releases?api-version=7.1"));
}

#[test]
fn azure_cross_organization_fork_is_rejected_before_network() {
    let action = write::Action::Create {
        source_project: "other/project/repo".into(),
        target_project: "org/project/repo".into(),
        title: "Title".into(),
        body: String::new(),
        source: "topic".into(),
        target: "main".into(),
        draft: false,
    };
    assert!(azure::execute(
        "http://127.0.0.1:1/org/project/_apis/git/repositories/repo",
        &auth(),
        &action
    )
    .unwrap_err()
    .contains("organization"));
}

#[test]
fn azure_old_side_renamed_file_comment_keeps_tracking_identity() {
    let (base, server) = fixture(vec![
        reply(current(true)),
        source_ref(),
        reply(json!({"value":[{"id":4,"sourceRefCommit":{"commitId":head()}}]})),
        reply(
            json!({"changeEntries":[{"changeTrackingId":8,"item":{"path":"/new.rs"},"originalPath":"/old.rs"}],"nextSkip":0}),
        ),
        reply(json!({"id":14})),
        reply(current(true)),
        source_ref(),
    ]);
    let mut action = review();
    if let write::Action::Review { inline, .. } = &mut action {
        *inline = Some(write::InlineComment {
            path: "old.rs".into(),
            old_line: 5,
            ..Default::default()
        });
    }
    azure::execute(&base, &auth(), &action).unwrap();
    let requests = server.join().unwrap();
    assert_eq!(requests[4].1["threadContext"]["leftFileStart"]["line"], 5);
    assert_eq!(requests[4].1["threadContext"]["filePath"], "/old.rs");
    assert_eq!(
        requests[4].1["pullRequestThreadContext"]["changeTrackingId"],
        8
    );
}

#[test]
fn bitbucket_details_query_statuses_for_the_resolved_source_commit_only() {
    let mut pr = current(false);
    pr["id"] = json!(7);
    pr["title"] = json!("Short head");
    pr["source"]["commit"]["hash"] = json!("a".repeat(12));
    pr["source"]["repository"] = json!({"full_name":"fork/repo"});
    let (base, server) = fixture(vec![
        reply(pr),
        reply(json!({"hash":head()})),
        reply(json!({"values":[{"key":"ci","state":"SUCCESSFUL"}]})),
    ]);
    let (item, _) = bitbucket::details_from(&base, 7, &auth()).unwrap();
    assert_eq!(item.head_oid, head());
    assert_eq!(item.check_state, Some(CheckState::Success));
    let requests = server.join().unwrap();
    assert!(requests[2].0.contains(&format!(
        "/repositories/fork/repo/commit/{}/statuses?",
        head()
    )));
    assert!(!requests[2].0.contains("/pullrequests/"));
}
