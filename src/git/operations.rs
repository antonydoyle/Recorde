use std::path::PathBuf;

use crate::git::graph::CommitGraph;
use crate::state::SharedState;

/// Load a repository and compute its commit graph, updating shared state.
/// Runs on a background thread so the UI stays responsive.
pub fn load_repo(path: PathBuf, state: SharedState) {
    std::thread::spawn(move || {
        let result = (|| -> Result<CommitGraph, git2::Error> {
            let repo = git2::Repository::open(&path)?;
            CommitGraph::from_repo(&repo)
        })();

        let mut s = state.lock().unwrap();
        match result {
            Ok(graph) => {
                s.repo_path = Some(path);
                s.commit_graph = Some(graph);
                s.error = None;
            }
            Err(e) => {
                s.error = Some(e.message().to_string());
            }
        }
    });
}
