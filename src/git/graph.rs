use std::collections::HashMap;

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
    pub nodes: Vec<CommitNode>,
    pub edges: Vec<GraphEdge>,
    pub oid_to_row: HashMap<String, usize>,
    pub max_lane: usize,
    /// Shorthand names of all tag refs (used by the sidebar for correct classification).
    pub tag_names: std::collections::HashSet<String>,
}

impl CommitGraph {
    pub fn from_repo(repo: &git2::Repository) -> Result<Self, git2::Error> {
        let mut walk = repo.revwalk()?;
        walk.push_glob("refs/*")?;
        walk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME)?;

        // Collect tag shorthand names so the sidebar can classify them correctly.
        let tag_names: std::collections::HashSet<String> = repo
            .tag_names(None)?
            .iter()
            .flatten()
            .map(str::to_string)
            .collect();

        // Map OID → ref names for badge display.
        // For annotated tags reference.target() returns the tag-object OID, not the
        // commit OID, so we peel each reference to its underlying commit before
        // inserting into the map.
        let mut ref_map: HashMap<String, Vec<String>> = HashMap::new();
        for reference in repo.references()? {
            let reference = reference?;
            let Some(shorthand) = reference.shorthand() else { continue };
            // peel_to_commit handles both lightweight tags (direct) and annotated tags
            let commit_oid = reference
                .peel_to_commit()
                .map(|c| c.id().to_string())
                .or_else(|_| {
                    // fallback for refs that aren't commits (e.g. blobs, trees)
                    reference.target().map(|o| o.to_string()).ok_or_else(|| {
                        git2::Error::from_str("no target")
                    })
                });
            if let Ok(oid) = commit_oid {
                ref_map
                    .entry(oid)
                    .or_default()
                    .push(shorthand.to_string());
            }
        }

        let mut nodes: Vec<CommitNode> = Vec::new();
        let mut active: Vec<Option<String>> = Vec::new(); // lane slot → expected next OID
        let mut max_lane = 0usize;

        for oid in walk {
            let oid = oid?;
            let commit = repo.find_commit(oid)?;
            let oid_str = oid.to_string();

            let parents: Vec<String> = commit.parent_ids().map(|p| p.to_string()).collect();
            let lane = assign_lane(&oid_str, &parents, &mut active);
            if lane > max_lane {
                max_lane = lane;
            }

            let full_msg = commit.message().unwrap_or("").to_string();
            let summary = commit.summary().unwrap_or("").to_string();
            let body = {
                let b = full_msg.trim_start_matches(&summary).trim().to_string();
                if b.is_empty() { None } else { Some(b) }
            };

            nodes.push(CommitNode {
                oid: oid_str.clone(),
                summary,
                body,
                author: commit.author().name().unwrap_or("Unknown").to_string(),
                author_email: commit.author().email().unwrap_or("").to_string(),
                time: commit.time().seconds(),
                refs: ref_map.remove(&oid_str).unwrap_or_default(),
                parents,
                lane,
            });
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

        Ok(Self { nodes, edges, oid_to_row, max_lane, tag_names })
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
