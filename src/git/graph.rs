use std::collections::HashMap;

#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GraphFilter {
    /// Fully qualified refs keep branches and tags with the same name independent.
    pub hidden_refs: std::collections::BTreeSet<String>,
    pub solo_ref: Option<String>,
    /// Reachable from end, excluding everything reachable from start (start..end).
    pub range: Option<(String, String)>,
}

/// A single commit node in the graph.
#[derive(Debug, Clone)]
pub struct CommitNode {
    pub oid: String,
    pub summary: String,
    pub body: Option<String>,
    pub author: String,
    pub author_email: String,
    pub time: i64,
    pub parents: Vec<String>,
    pub lane: usize,
    pub refs: Vec<String>,
}

/// A rendered edge between two commits.
/// Always points downward (from_row < to_row).
#[derive(Debug, Clone)]
pub struct GraphEdge {
    pub from_row: usize,
    pub from_lane: usize,
    pub to_row: usize,
    pub to_lane: usize,
}

/// Full computed commit graph ready for rendering.
#[derive(Debug, Default, Clone)]
pub struct CommitGraph {
    cursor: Option<std::sync::Arc<super::history_cursor::Cursor>>,
    /// Effective history follows Git replacement refs.
    pub replacement_refs: bool,
    pub order_may_refine: bool,
    pub filter: GraphFilter,
    pub ref_targets: std::collections::BTreeMap<String, String>,
    pub local_names: std::collections::HashSet<String>,
    pub remote_names: std::collections::HashSet<String>,
    pub head_oid: Option<String>,
    pub has_more: bool,
    pub limit: usize,
    pub max_ref_chars: usize,
    edge_max: Vec<usize>,
    pub nodes: Vec<CommitNode>,
    pub edges: Vec<GraphEdge>,
    pub oid_to_row: HashMap<String, usize>,
    pub max_lane: usize,
    /// Shorthand names of all tag refs (used by the sidebar for correct classification).
    pub tag_names: std::collections::HashSet<String>,
}

impl CommitGraph {
    pub fn from_repo(repo: &git2::Repository) -> Result<Self, git2::Error> {
        Self::from_repo_limit(repo, usize::MAX)
    }

    pub fn from_repo_limit(repo: &git2::Repository, limit: usize) -> Result<Self, git2::Error> {
        Self::from_repo_filtered(repo, limit, &GraphFilter::default(), false)
    }

    pub fn from_repo_filtered(
        repo: &git2::Repository,
        limit: usize,
        filter: &GraphFilter,
        head_only: bool,
    ) -> Result<Self, git2::Error> {
        Self::from_repo_page(repo, limit, filter, head_only, None)
    }

    pub fn from_repo_page(
        repo: &git2::Repository,
        limit: usize,
        filter: &GraphFilter,
        head_only: bool,
        previous: Option<&Self>,
    ) -> Result<Self, git2::Error> {
        let limit = limit.max(1);
        let mut roots = Vec::new();
        let mut hidden = Vec::new();
        let replacements = super::replacement::refs(repo)?;
        let replacement_refs = !replacements.is_empty();
        let mut ref_targets = std::collections::BTreeMap::new();
        for reference in repo.references()? {
            let reference = reference?;
            let Some(name) = reference.name().ok() else {
                continue;
            };
            if reference.symbolic_target().ok().flatten().is_some() {
                continue;
            }
            if name.starts_with("refs/recorde/") || super::replacement::is_ref(name) {
                continue;
            }
            if let Ok(commit) = reference.peel_to_commit() {
                ref_targets.insert(name.to_string(), commit.id().to_string());
                if filter.range.is_none()
                    && filter.solo_ref.is_none()
                    && !head_only
                    && !filter.hidden_refs.contains(name)
                {
                    roots.push(commit.id());
                }
            }
        }
        let head_oid = repo
            .head()
            .ok()
            .and_then(|h| h.peel_to_commit().ok())
            .map(|c| c.id());
        if let Some((start, end)) = &filter.range {
            roots.push(repo.revparse_single(end)?.peel_to_commit()?.id());
            hidden.push(repo.revparse_single(start)?.peel_to_commit()?.id());
        } else if let Some(solo) = &filter.solo_ref {
            roots.push(repo.find_reference(solo)?.peel_to_commit()?.id());
        } else if head_only || repo.head_detached().unwrap_or(false) {
            if let Some(head) = head_oid {
                roots.push(head);
            }
        }
        roots.sort();
        roots.dedup();
        let cursor = previous
            .filter(|g| {
                g.ref_targets == ref_targets && g.head_oid == head_oid.map(|oid| oid.to_string())
            })
            .and_then(|g| g.cursor.as_ref())
            .filter(|cursor| {
                cursor.path == repo.path()
                    && cursor.roots == roots
                    && cursor.hidden == hidden
                    && cursor.replacements == replacements
                    && !repo.is_shallow()
                    && cursor.accelerated == super::history_cursor::Cursor::can_accelerate(repo)
            })
            .cloned()
            .unwrap_or_else(|| {
                std::sync::Arc::new(super::history_cursor::Cursor::new(
                    repo.path().to_path_buf(),
                    roots,
                    hidden,
                ))
            });
        let ids = cursor.read(limit.saturating_add(1))?;

        // Collect tag shorthand names so the sidebar can classify them correctly.
        let tag_names: std::collections::HashSet<String> = repo
            .tag_names(None)?
            .iter()
            .flatten()
            .flatten()
            .map(str::to_string)
            .collect();

        // Map OID → ref names for badge display.
        // For annotated tags reference.target() returns the tag-object OID, not the
        // commit OID, so we peel each reference to its underlying commit before
        // inserting into the map.
        let mut local_names = std::collections::HashSet::new();
        let mut remote_names = std::collections::HashSet::new();
        let mut ref_map: HashMap<String, Vec<String>> = HashMap::new();
        for reference in repo.references()? {
            let reference = reference?;
            if reference.name().ok().is_some_and(|name| {
                name.starts_with("refs/recorde/") || super::replacement::is_ref(name)
            }) {
                continue;
            }
            let Some(shorthand) = reference.shorthand().ok() else {
                continue;
            };
            if reference.is_branch() {
                local_names.insert(shorthand.to_string());
            }
            if reference.is_remote() && reference.symbolic_target().ok().flatten().is_none() {
                remote_names.insert(shorthand.to_string());
            }
            // peel_to_commit handles both lightweight tags (direct) and annotated tags
            let commit_oid = reference
                .peel_to_commit()
                .map(|c| c.id().to_string())
                .or_else(|_| {
                    // fallback for refs that aren't commits (e.g. blobs, trees)
                    reference
                        .target()
                        .map(|o| o.to_string())
                        .ok_or_else(|| git2::Error::from_str("no target"))
                });
            if let Ok(oid) = commit_oid {
                ref_map.entry(oid).or_default().push(shorthand.to_string());
            }
        }

        let mut nodes: Vec<CommitNode> = Vec::new();
        let mut active: Vec<Option<String>> = Vec::new(); // lane slot → expected next OID
        let mut max_lane = 0usize;

        let mut has_more = false;
        for (index, oid) in ids.into_iter().enumerate() {
            if index == limit {
                has_more = true;
                break;
            }
            let oid_str = oid.to_string();
            let cached = previous
                .filter(|g| {
                    g.cursor
                        .as_ref()
                        .is_some_and(|old| std::sync::Arc::ptr_eq(old, &cursor))
                })
                .and_then(|g| g.nodes.get(index))
                .filter(|node| node.oid == oid_str);
            let mut node = if let Some(cached) = cached {
                cached.clone()
            } else {
                let commit = repo.find_commit(super::replacement::resolve(oid, &replacements)?)?;
                let summary = commit.summary().ok().flatten().unwrap_or("").to_string();
                let body = commit
                    .message()
                    .unwrap_or("")
                    .trim_start_matches(&summary)
                    .trim()
                    .to_string();
                let loaded = CommitNode {
                    oid: oid_str.clone(),
                    summary,
                    body: if body.is_empty() { None } else { Some(body) },
                    author: commit.author().name().unwrap_or("Unknown").to_string(),
                    author_email: commit.author().email().unwrap_or("").to_string(),
                    time: commit.time().seconds(),
                    refs: ref_map.remove(&oid_str).unwrap_or_default(),
                    parents: commit.parent_ids().map(|p| p.to_string()).collect(),
                    lane: 0,
                };
                loaded
            };
            node.lane = assign_lane(&node.oid, &node.parents, &mut active);
            max_lane = max_lane.max(node.lane);
            nodes.push(node);
        }

        // Sort each node's refs so tags come first, ensuring they appear
        // within the 3-badge limit in the graph column even on busy commits.
        for node in nodes.iter_mut() {
            node.refs.sort_by(|a, b| {
                let a_tag = tag_names.contains(a.as_str());
                let b_tag = tag_names.contains(b.as_str());
                match (a_tag, b_tag) {
                    (true, false) => std::cmp::Ordering::Less,
                    (false, true) => std::cmp::Ordering::Greater,
                    _ => a.cmp(b),
                }
            });
        }

        // Build OID→row index.
        let oid_to_row: HashMap<String, usize> = nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.oid.clone(), i))
            .collect();

        // Precompute edges.
        let mut edges: Vec<GraphEdge> = Vec::new();
        for (row, node) in nodes.iter().enumerate() {
            for parent_oid in &node.parents {
                if let Some(&parent_row) = oid_to_row.get(parent_oid) {
                    edges.push(GraphEdge {
                        from_row: row,
                        from_lane: node.lane,
                        to_row: parent_row,
                        to_lane: nodes[parent_row].lane,
                    });
                }
            }
        }

        let edge_max = index_edges(&edges);
        let max_ref_chars = local_names
            .iter()
            .chain(remote_names.iter())
            .chain(tag_names.iter())
            .map(|s| s.chars().count())
            .max()
            .unwrap_or(0);
        Ok(Self {
            order_may_refine: !cursor.accelerated,
            cursor: Some(cursor),
            replacement_refs,
            filter: filter.clone(),
            ref_targets,
            has_more,
            limit,
            max_ref_chars,
            edge_max,
            local_names,
            remote_names,
            head_oid: head_oid.map(|oid| oid.to_string()),
            nodes,
            edges,
            oid_to_row,
            max_lane,
            tag_names,
        })
    }

    /// Traverse parent OIDs, then rebuild row/lane geometry for the retained DAG.
    pub fn head_ancestry(&self) -> Self {
        let mut keep = std::collections::HashSet::new();
        let mut pending: Vec<_> = self.head_oid.iter().cloned().collect();
        while let Some(oid) = pending.pop() {
            if !keep.insert(oid.clone()) {
                continue;
            }
            if let Some(&row) = self.oid_to_row.get(&oid) {
                pending.extend(self.nodes[row].parents.iter().cloned());
            }
        }
        let mut nodes: Vec<_> = self
            .nodes
            .iter()
            .filter(|node| keep.contains(&node.oid))
            .cloned()
            .collect();
        let mut active = Vec::new();
        let mut max_lane = 0;
        for node in &mut nodes {
            node.lane = assign_lane(&node.oid, &node.parents, &mut active);
            max_lane = max_lane.max(node.lane);
        }
        let oid_to_row: HashMap<_, _> = nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.oid.clone(), i))
            .collect();
        let mut edges = Vec::new();
        for (row, node) in nodes.iter().enumerate() {
            for parent in &node.parents {
                if let Some(&to_row) = oid_to_row.get(parent) {
                    edges.push(GraphEdge {
                        from_row: row,
                        from_lane: node.lane,
                        to_row,
                        to_lane: nodes[to_row].lane,
                    });
                }
            }
        }
        let edge_max = index_edges(&edges);
        Self {
            cursor: None,
            replacement_refs: self.replacement_refs,
            order_may_refine: self.order_may_refine,
            filter: self.filter.clone(),
            ref_targets: self.ref_targets.clone(),
            has_more: self.has_more,
            limit: self.limit,
            max_ref_chars: self.max_ref_chars,
            edge_max,
            local_names: self.local_names.clone(),
            remote_names: self.remote_names.clone(),
            head_oid: self.head_oid.clone(),
            nodes,
            edges,
            oid_to_row,
            max_lane,
            tag_names: self.tag_names.clone(),
        }
    }
    /// Interval index skips entire groups of offscreen edges, including long edges.
    pub fn visible_edges(&self, first: usize, last: usize) -> Vec<&GraphEdge> {
        let mut result = Vec::new();
        if self.edges.is_empty() || first >= last {
            return result;
        }
        let mut pending = vec![(1, 0, self.edges.len().next_power_of_two())];
        while let Some((node, start, end)) = pending.pop() {
            if start >= self.edges.len()
                || self.edges[start].from_row >= last
                || self.edge_max[node] < first
            {
                continue;
            }
            if end - start == 1 {
                result.push(&self.edges[start]);
            } else {
                let mid = start + (end - start) / 2;
                pending.push((node * 2 + 1, mid, end));
                pending.push((node * 2, start, mid));
            }
        }
        result
    }
}

fn index_edges(edges: &[GraphEdge]) -> Vec<usize> {
    let size = edges.len().max(1).next_power_of_two();
    let mut max = vec![0; size * 2];
    for (index, edge) in edges.iter().enumerate() {
        max[size + index] = edge.to_row;
    }
    for index in (1..size).rev() {
        max[index] = max[index * 2].max(max[index * 2 + 1]);
    }
    max
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_support::TestRepo;

    #[test]
    fn hide_solo_and_range_use_exact_refs_and_git_reachability() {
        let fixture = TestRepo::new();
        let base = fixture.commit("base", b"base");
        fixture.git_ok(&["checkout", "-b", "topic"]);
        let topic = fixture.commit("topic", b"topic");
        fixture.git_ok(&["checkout", "main"]);
        fixture.commit("main", b"main");
        fixture.git_ok(&["tag", "topic", &topic.to_string()]);
        let repo = fixture.open();
        let mut filter = GraphFilter::default();
        filter.hidden_refs.insert("refs/heads/topic".into());
        assert!(CommitGraph::from_repo_filtered(&repo, 100, &filter, false)
            .unwrap()
            .oid_to_row
            .contains_key(&topic.to_string()));
        filter.hidden_refs.insert("refs/tags/topic".into());
        let graph = CommitGraph::from_repo_filtered(&repo, 100, &filter, false).unwrap();
        assert!(!graph.oid_to_row.contains_key(&topic.to_string()));
        assert!(graph.oid_to_row.contains_key(&base.to_string()));
        filter.solo_ref = Some("refs/tags/topic".into());
        let graph = CommitGraph::from_repo_filtered(&repo, 100, &filter, false).unwrap();
        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.nodes[0].oid, topic.to_string());
        filter.range = Some((base.to_string(), "refs/heads/topic".into()));
        let graph = CommitGraph::from_repo_filtered(&repo, 100, &filter, false).unwrap();
        let expected = fixture.git(&["rev-list", &format!("{base}..refs/heads/topic")]);
        assert!(expected.status.success());
        assert_eq!(
            graph
                .nodes
                .iter()
                .map(|node| node.oid.as_str())
                .collect::<Vec<_>>(),
            String::from_utf8_lossy(&expected.stdout)
                .lines()
                .collect::<Vec<_>>()
        );
        assert!(graph.edges.is_empty());
    }

    #[test]
    fn current_branch_is_filtered_before_page_limit() {
        let fixture = TestRepo::new();
        let head = fixture.commit("base", b"base");
        fixture.git_ok(&["checkout", "-b", "busy"]);
        for n in 0..10 {
            fixture.commit("topic", format!("{n}").as_bytes());
        }
        fixture.git_ok(&["checkout", "main"]);
        let graph =
            CommitGraph::from_repo_filtered(&fixture.open(), 1, &GraphFilter::default(), true)
                .unwrap();
        assert_eq!(graph.nodes[0].oid, head.to_string());
        assert!(!graph.has_more);
    }

    #[test]
    fn shallow_history_stops_at_native_git_boundary() {
        let source = TestRepo::new();
        for n in 0..5 {
            source.commit("file", format!("{n}").as_bytes());
        }
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("shallow");
        let url = url::Url::from_directory_path(source.path()).unwrap();
        source.git_ok(&[
            "clone",
            "--depth",
            "2",
            url.as_str(),
            destination.to_str().unwrap(),
        ]);
        let repo = git2::Repository::open(&destination).unwrap();
        assert!(repo.is_shallow());
        let graph = CommitGraph::from_repo(&repo).unwrap();
        let output = source.git(&["-C", destination.to_str().unwrap(), "rev-list", "HEAD"]);
        assert!(output.status.success());
        assert_eq!(
            graph
                .nodes
                .iter()
                .map(|node| node.oid.as_str())
                .collect::<Vec<_>>(),
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .collect::<Vec<_>>()
        );
        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.edges.len(), 1);
    }

    #[test]
    fn cold_pages_topologically_refine_skewed_octopus_without_writes_or_omissions() {
        let fixture = TestRepo::new();
        fixture.commit("base", b"base");
        let repo = fixture.open();
        let base = repo.head().unwrap().peel_to_commit().unwrap();
        let tree = base.tree().unwrap();
        let create = |parents: &[&git2::Commit], seconds| {
            let signature = git2::Signature::new(
                "Fixture",
                "fixture@example.com",
                &git2::Time::new(seconds, 0),
            )
            .unwrap();
            repo.commit(None, &signature, &signature, "skewed", &tree, parents)
                .unwrap()
        };
        let a = repo.find_commit(create(&[&base], 1000)).unwrap();
        let b = repo.find_commit(create(&[&base], 9000)).unwrap();
        let c = repo.find_commit(create(&[&a], 2000)).unwrap();
        let d = repo.find_commit(create(&[&b], 500)).unwrap();
        let head = create(&[&c, &d, &base], 400);
        repo.reference("refs/heads/main", head, true, "fixture")
            .unwrap();
        let original_index = std::fs::read(repo.path().join("index")).unwrap();
        let mut graph = CommitGraph::from_repo_limit(&repo, 2).unwrap();
        assert!(graph.order_may_refine);
        for count in [3, 4, 8] {
            graph = CommitGraph::from_repo_page(
                &repo,
                count,
                &GraphFilter::default(),
                false,
                Some(&graph),
            )
            .unwrap();
            assert!(graph.edges.iter().all(|e| e.from_row < e.to_row));
        }
        assert!(!graph.has_more);
        assert_eq!(graph.nodes.len(), 6);
        assert_eq!(
            graph
                .nodes
                .iter()
                .map(|n| &n.oid)
                .collect::<std::collections::HashSet<_>>()
                .len(),
            6
        );
        assert_eq!(
            std::fs::read(repo.path().join("index")).unwrap(),
            original_index
        );
        assert!(!repo.path().join("objects/info/commit-graph").exists());
    }

    #[test]
    fn replacement_history_uses_effective_parents_and_invalidates_cached_pages() {
        let fixture = TestRepo::new();
        let root = fixture.commit("root", b"root");
        let middle = fixture.commit("middle", b"middle");
        let head = fixture.commit("head", b"head");
        let repo = fixture.open();
        let head_commit = repo.find_commit(head).unwrap();
        let signature = repo.signature().unwrap();
        let replacement = repo
            .commit(
                None,
                &signature,
                &signature,
                "replacement message",
                &head_commit.tree().unwrap(),
                &[&repo.find_commit(root).unwrap()],
            )
            .unwrap();
        let original = CommitGraph::from_repo_limit(&repo, 2).unwrap();
        fixture.git_ok(&["replace", &head.to_string(), &replacement.to_string()]);
        let effective =
            CommitGraph::from_repo_page(&repo, 10, &GraphFilter::default(), true, Some(&original))
                .unwrap();
        assert_eq!(effective.nodes.len(), 2);
        assert_eq!(effective.nodes[0].oid, head.to_string());
        assert_eq!(effective.nodes[0].summary, "replacement message");
        assert_eq!(effective.nodes[0].parents, vec![root.to_string()]);
        assert!(!effective.oid_to_row.contains_key(&middle.to_string()));
        assert!(effective.replacement_refs);
        fixture.git_ok(&["replace", "-d", &head.to_string()]);
        let restored =
            CommitGraph::from_repo_page(&repo, 10, &GraphFilter::default(), true, Some(&effective))
                .unwrap();
        assert_eq!(restored.nodes.len(), 3);
    }

    #[test]
    #[ignore = "manual 100k-commit performance fixture"]
    fn benchmark_large_history_first_page() {
        let fixture = TestRepo::new();
        fixture.commit("base", b"base");
        let repo = fixture.open();
        let tree = repo.head().unwrap().peel_to_tree().unwrap();
        let signature = repo.signature().unwrap();
        let mut oid = repo.head().unwrap().target().unwrap();
        let count = std::env::var("RECORDE_PERF_COMMITS")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(100_000)
            .clamp(2000, 1_000_000);
        for _ in 0..count {
            let parent = repo.find_commit(oid).unwrap();
            oid = repo
                .commit(None, &signature, &signature, "history", &tree, &[&parent])
                .unwrap();
        }
        repo.reference("refs/heads/main", oid, true, "fixture")
            .unwrap();
        if std::env::var_os("RECORDE_PERF_COMMIT_GRAPH").is_some() {
            let started = std::time::Instant::now();
            super::super::lifecycle::optimize_history(fixture.path()).unwrap();
            eprintln!("Build native commit graph: {:?}", started.elapsed());
        }
        let started = std::time::Instant::now();
        let graph = CommitGraph::from_repo_limit(&repo, 2000).unwrap();
        eprintln!(
            "{} commits, first 2000 nodes: {:?}; {} edges; {} edge-index slots",
            count + 1,
            started.elapsed(),
            graph.edges.len(),
            graph.edge_max.len()
        );
        assert_eq!(graph.nodes.len(), 2000);
        assert!(graph.has_more);
        let next_started = std::time::Instant::now();
        let next_page =
            CommitGraph::from_repo_page(&repo, 4000, &GraphFilter::default(), false, Some(&graph))
                .unwrap();
        eprintln!(
            "Next 2000 nodes with retained cursor: {:?}",
            next_started.elapsed()
        );
        assert!(std::sync::Arc::ptr_eq(
            graph.cursor.as_ref().unwrap(),
            next_page.cursor.as_ref().unwrap()
        ));
        assert_eq!(next_page.nodes.len(), 4000.min(count + 1));
        if std::env::var_os("RECORDE_PERF_COMMIT_GRAPH").is_some() {
            let expanded = CommitGraph::from_repo_page(
                &repo,
                9000,
                &GraphFilter::default(),
                false,
                Some(&next_page),
            )
            .unwrap();
            assert_eq!(expanded.nodes.len(), 9000.min(count + 1));
            assert_eq!(expanded.oid_to_row.len(), expanded.nodes.len());
            assert_eq!(
                next_page.nodes.iter().map(|n| &n.oid).collect::<Vec<_>>(),
                expanded.nodes[..next_page.nodes.len()]
                    .iter()
                    .map(|n| &n.oid)
                    .collect::<Vec<_>>()
            );
            assert!(expanded.edges.iter().all(|e| e.from_row < e.to_row));
        }
        let native_started = std::time::Instant::now();
        let native = super::super::process::git(
            fixture.path(),
            &[
                "--no-replace-objects",
                "rev-list",
                "--topo-order",
                "--max-count=2001",
                &oid.to_string(),
            ],
            &std::sync::atomic::AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(
            native
                .stdout
                .split(|b| *b == b'\n')
                .filter(|line| !line.is_empty())
                .count(),
            2001
        );
        eprintln!(
            "Native Git first-page traversal: {:?}",
            native_started.elapsed()
        );
        let started = std::time::Instant::now();
        for first in 0..1900 {
            std::hint::black_box(graph.visible_edges(first, first + 40));
        }
        eprintln!("1900 visible-edge queries: {:?}", started.elapsed());
        let state = crate::state::new_shared_state();
        state.lock().unwrap().commit_graph = Some(std::sync::Arc::new(graph));
        let mut view = crate::ui::commit_graph::CommitGraphView::new(state.clone());
        let context = egui::Context::default();
        let mut frames = Vec::new();
        for frame in 0..240 {
            let begin = std::time::Instant::now();
            let output = context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 800.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| view.show(ui));
                },
            );
            std::hint::black_box(context.tessellate(output.shapes, output.pixels_per_point));
            if frame >= 20 {
                frames.push(begin.elapsed());
            }
        }
        frames.sort();
        eprintln!(
            "Graph UI + tessellation, 220 warm frames p95: {:?}",
            frames[frames.len() * 95 / 100]
        );
    }

    #[test]
    fn indexed_history_pages_preserve_topology_filters_and_dirty_work() {
        let fixture = TestRepo::new();
        let base = fixture.commit("base", b"base\n");
        fixture.git_ok(&["checkout", "-b", "other"]);
        let other = fixture.commit("other", b"other\n");
        fixture.git_ok(&["checkout", "main"]);
        fixture.commit("main", b"main\n");
        fixture.git_ok(&["merge", "--no-ff", "other", "-m", "merge"]);
        let head = fixture.head();
        fixture.write("main", b"staged\n");
        fixture.git_ok(&["add", "main"]);
        fixture.write("main", b"dirty\n");
        let repo = fixture.open();
        let before = CommitGraph::from_repo_limit(&repo, 2).unwrap();
        assert!(!before.cursor.as_ref().unwrap().accelerated);
        super::super::lifecycle::optimize_history(fixture.path()).unwrap();
        let page =
            CommitGraph::from_repo_page(&repo, 2, &GraphFilter::default(), false, Some(&before))
                .unwrap();
        assert!(page.cursor.as_ref().unwrap().accelerated);
        assert!(!std::sync::Arc::ptr_eq(
            before.cursor.as_ref().unwrap(),
            page.cursor.as_ref().unwrap()
        ));
        let all =
            CommitGraph::from_repo_page(&repo, 20, &GraphFilter::default(), false, Some(&page))
                .unwrap();
        assert!(!all.has_more);
        assert_eq!(all.nodes.len(), 4);
        assert_eq!(
            page.nodes
                .iter()
                .map(|n| (&n.oid, n.lane))
                .collect::<Vec<_>>(),
            all.nodes[..2]
                .iter()
                .map(|n| (&n.oid, n.lane))
                .collect::<Vec<_>>()
        );
        assert!(all.edges.iter().all(|e| e.from_row < e.to_row));
        let range = CommitGraph::from_repo_filtered(
            &repo,
            20,
            &GraphFilter {
                range: Some((base.to_string(), other.to_string())),
                ..Default::default()
            },
            false,
        )
        .unwrap();
        assert_eq!(range.nodes.len(), 1);
        assert_eq!(range.nodes[0].oid, other.to_string());
        assert_eq!(fixture.head(), head);
        assert_eq!(fixture.index_bytes("main"), b"staged\n");
        assert_eq!(
            std::fs::read(fixture.path().join("main")).unwrap(),
            b"dirty\n"
        );
        fixture.git_ok(&["commit-graph", "verify"]);
    }

    #[test]
    fn paging_reuses_walk_preserves_lanes_and_invalidates_changed_refs() {
        let fixture = TestRepo::new();
        fixture.commit("base", b"base");
        for number in 0..12 {
            fixture.commit("file", number.to_string().as_bytes());
        }
        let repo = fixture.open();
        let first = CommitGraph::from_repo_limit(&repo, 4).unwrap();
        let second =
            CommitGraph::from_repo_page(&repo, 8, &GraphFilter::default(), false, Some(&first))
                .unwrap();
        let fresh = CommitGraph::from_repo_limit(&repo, 8).unwrap();
        assert!(std::sync::Arc::ptr_eq(
            first.cursor.as_ref().unwrap(),
            second.cursor.as_ref().unwrap()
        ));
        assert_eq!(
            second
                .nodes
                .iter()
                .map(|n| (&n.oid, n.lane))
                .collect::<Vec<_>>(),
            fresh
                .nodes
                .iter()
                .map(|n| (&n.oid, n.lane))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            first
                .nodes
                .iter()
                .map(|n| (&n.oid, n.lane))
                .collect::<Vec<_>>(),
            second.nodes[..4]
                .iter()
                .map(|n| (&n.oid, n.lane))
                .collect::<Vec<_>>()
        );
        let new_head = fixture.commit("new", b"new").to_string();
        let changed =
            CommitGraph::from_repo_page(&repo, 8, &GraphFilter::default(), false, Some(&second))
                .unwrap();
        assert!(!std::sync::Arc::ptr_eq(
            second.cursor.as_ref().unwrap(),
            changed.cursor.as_ref().unwrap()
        ));
        assert_eq!(changed.nodes[0].oid, new_head);
    }
    #[test]
    fn branch_and_tag_namespaces_are_independent_even_with_slashes() {
        let repo = TestRepo::new();
        repo.commit("base", b"base");
        repo.git_ok(&["branch", "feature/topic"]);
        repo.git_ok(&["tag", "feature/topic"]);
        repo.git_ok(&["update-ref", "refs/remotes/upstream/topic", "HEAD"]);
        let graph = CommitGraph::from_repo(&repo.open()).unwrap();
        assert!(graph.local_names.contains("feature/topic"));
        assert!(graph.tag_names.contains("feature/topic"));
        assert!(!graph.remote_names.contains("feature/topic"));
        assert!(graph.remote_names.contains("upstream/topic"));
    }
    #[test]
    fn current_branch_filter_traverses_ancestry_and_remaps_edges() {
        let repo = TestRepo::new();
        let base = repo.commit("file.txt", b"base\n");
        repo.git_ok(&["checkout", "-b", "other"]);
        let other = repo.commit("other.txt", b"other\n");
        repo.git_ok(&["checkout", "main"]);
        let head = repo.commit("file.txt", b"main\n");
        repo.git_ok(&["tag", "main", &other.to_string()]);
        let filtered = CommitGraph::from_repo(&repo.open())
            .unwrap()
            .head_ancestry();
        assert_eq!(filtered.nodes.len(), 2);
        assert!(filtered.oid_to_row.contains_key(&base.to_string()));
        assert!(filtered.oid_to_row.contains_key(&head.to_string()));
        assert!(!filtered.oid_to_row.contains_key(&other.to_string()));
        for edge in &filtered.edges {
            assert!(filtered.nodes[edge.from_row]
                .parents
                .contains(&filtered.nodes[edge.to_row].oid));
            assert!(edge.from_row < edge.to_row);
        }
    }
    #[test]
    fn detached_only_commits_and_empty_repositories_are_visible() {
        let repo = TestRepo::new();
        assert!(CommitGraph::from_repo(&repo.open())
            .unwrap()
            .nodes
            .is_empty());
        repo.commit("file.txt", b"base\n");
        repo.git_ok(&["checkout", "--detach"]);
        let detached = repo.commit("file.txt", b"detached\n");
        let graph = CommitGraph::from_repo(&repo.open()).unwrap();
        assert_eq!(graph.head_oid, Some(detached.to_string()));
        assert!(graph.oid_to_row.contains_key(&detached.to_string()));
    }

    #[test]
    fn paging_preserves_rows_and_lanes_and_index_returns_visible_edges() {
        let fixture = TestRepo::new();
        fixture.commit("base", b"base");
        let repo = fixture.open();
        let tree = repo.head().unwrap().peel_to_tree().unwrap();
        let signature = repo.signature().unwrap();
        for _ in 0..300 {
            let parent = repo.head().unwrap().peel_to_commit().unwrap();
            repo.commit(
                Some("HEAD"),
                &signature,
                &signature,
                "history",
                &tree,
                &[&parent],
            )
            .unwrap();
        }
        let page = CommitGraph::from_repo_limit(&repo, 100).unwrap();
        let more = CommitGraph::from_repo_limit(&repo, 200).unwrap();
        assert_eq!(page.nodes.len(), 100);
        assert!(page.has_more && more.has_more);
        for (before, after) in page.nodes.iter().zip(&more.nodes) {
            assert_eq!((&before.oid, before.lane), (&after.oid, after.lane));
        }
        for (first, last) in [(0, 3), (40, 70), (195, 210)] {
            let indexed: Vec<_> = more
                .visible_edges(first, last)
                .iter()
                .map(|e| (e.from_row, e.to_row))
                .collect();
            let expected: Vec<_> = more
                .edges
                .iter()
                .filter(|e| e.from_row < last && e.to_row >= first)
                .map(|e| (e.from_row, e.to_row))
                .collect();
            assert_eq!(indexed, expected);
        }
        assert!(!CommitGraph::from_repo_limit(&repo, 400).unwrap().has_more);
        // A long edge must remain visible even when both endpoints are offscreen.
        let mut graph = CommitGraph::default();
        graph.edges = vec![GraphEdge {
            from_row: 0,
            to_row: 1000,
            from_lane: 0,
            to_lane: 1,
        }];
        graph.edge_max = index_edges(&graph.edges);
        assert_eq!(graph.visible_edges(500, 510).len(), 1);
    }
}

/// Assign a horizontal lane to a commit using a greedy swimlane algorithm.
///
/// `active[i]` = the OID we are waiting for in lane `i` (the next commit
/// that should be rendered in that lane). `None` means the lane is free.
fn assign_lane(oid: &str, parents: &[String], active: &mut Vec<Option<String>>) -> usize {
    // Collect all lanes currently expecting this commit.
    let expecting: Vec<usize> = active
        .iter()
        .enumerate()
        .filter(|(_, s)| s.as_deref() == Some(oid))
        .map(|(i, _)| i)
        .collect();

    let lane = if !expecting.is_empty() {
        let lane = expecting[0];
        // The primary lane expects the first parent.
        active[lane] = parents.first().cloned();
        // Free any other lanes that were also expecting this commit (merge targets).
        for &other in expecting.iter().skip(1) {
            active[other] = None;
        }
        lane
    } else {
        // New branch tip: find a free slot or open a new lane.
        if let Some(pos) = active.iter().position(|s| s.is_none()) {
            active[pos] = parents.first().cloned();
            pos
        } else {
            active.push(parents.first().cloned());
            active.len() - 1
        }
    };

    // Register additional parents (merge commits) in free or new lanes.
    for parent in parents.iter().skip(1) {
        if !active.iter().any(|s| s.as_deref() == Some(parent.as_str())) {
            if let Some(pos) = active.iter().position(|s| s.is_none()) {
                active[pos] = Some(parent.clone());
            } else {
                active.push(Some(parent.clone()));
            }
        }
    }

    lane
}
