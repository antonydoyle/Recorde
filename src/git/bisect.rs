use std::path::Path;

fn run(repo_path: &Path, args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new("git")
        .arg("-C").arg(repo_path)
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Start a bisect session. `bad_oid` is the known-bad commit; good is HEAD~N or left empty
/// to let the user supply it manually.
pub fn start(repo_path: &Path) -> Result<(), String> {
    run(repo_path, &["bisect", "start"])?;
    Ok(())
}

pub fn mark_bad(repo_path: &Path, oid: &str) -> Result<Option<String>, String> {
    run(repo_path, &["bisect", "bad", oid])?;
    Ok(next_step(repo_path))
}

pub fn mark_good(repo_path: &Path, oid: &str) -> Result<Option<String>, String> {
    run(repo_path, &["bisect", "good", oid])?;
    Ok(next_step(repo_path))
}

pub fn reset(repo_path: &Path) -> Result<(), String> {
    run(repo_path, &["bisect", "reset"])?;
    Ok(())
}

/// After good/bad, read BISECT_HEAD for the next commit to test.
fn next_step(repo_path: &Path) -> Option<String> {
    let repo = git2::Repository::open(repo_path).ok()?;
    let oid = std::fs::read_to_string(repo.path().join("BISECT_HEAD")).ok()?;
    Some(oid.trim().to_string())
}
