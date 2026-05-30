/// A single node in the commit graph.
#[derive(Debug, Clone)]
pub struct CommitNode {
    pub oid: String,
    pub summary: String,
    pub author: String,
    pub time: i64,
    /// Parent OIDs.
    pub parents: Vec<String>,
    /// Horizontal lane index for rendering.
    pub lane: usize,
    /// Branch/tag labels on this commit.
    pub refs: Vec<String>,
}

/// The full computed commit graph ready for rendering.
#[derive(Debug, Default, Clone)]
pub struct CommitGraph {
    pub nodes: Vec<CommitNode>,
}

impl CommitGraph {
    /// Build a CommitGraph from an open git2 repository.
    pub fn from_repo(repo: &git2::Repository) -> Result<Self, git2::Error> {
        let mut walk = repo.revwalk()?;
        walk.push_head()?;
        walk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME)?;

        // Collect ref names keyed by OID for label display.
        let mut ref_map: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for reference in repo.references()? {
            let reference = reference?;
            if let (Some(name), Some(target)) = (reference.shorthand(), reference.target()) {
                ref_map
                    .entry(target.to_string())
                    .or_default()
                    .push(name.to_string());
            }
        }

        let mut nodes = Vec::new();
        // Simple lane assignment: track which lanes are occupied by open branches.
        let mut lane_tips: Vec<String> = Vec::new();

        for oid in walk {
            let oid = oid?;
            let commit = repo.find_commit(oid)?;
            let oid_str = oid.to_string();

            let parents: Vec<String> = commit
                .parent_ids()
                .map(|p| p.to_string())
                .collect();

            let lane = assign_lane(&oid_str, &parents, &mut lane_tips);

            nodes.push(CommitNode {
                summary: commit.summary().unwrap_or("").to_string(),
                author: commit
                    .author()
                    .name()
                    .unwrap_or("Unknown")
                    .to_string(),
                time: commit.time().seconds(),
                refs: ref_map.remove(&oid_str).unwrap_or_default(),
                parents,
                oid: oid_str,
                lane,
            });
        }

        Ok(Self { nodes })
    }
}

/// Assign a horizontal lane index using a greedy swimlane algorithm.
fn assign_lane(oid: &str, parents: &[String], lane_tips: &mut Vec<String>) -> usize {
    // Find the lane currently expecting this commit.
    let lane = if let Some(pos) = lane_tips.iter().position(|t| t == oid) {
        // Reuse the lane; update its tip to the first parent.
        if let Some(first_parent) = parents.first() {
            lane_tips[pos] = first_parent.clone();
        } else {
            lane_tips.remove(pos);
        }
        pos
    } else {
        // New branch: open a new lane.
        let pos = lane_tips.len();
        if let Some(first_parent) = parents.first() {
            lane_tips.push(first_parent.clone());
        }
        pos
    };

    // Register remaining parents onto new lanes if not already tracked.
    for parent in parents.iter().skip(1) {
        if !lane_tips.contains(parent) {
            lane_tips.push(parent.clone());
        }
    }

    lane
}
