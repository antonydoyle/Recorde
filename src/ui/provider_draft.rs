//! Text drafts are scoped to repository, remote, account and review number.
//! Confirmation and merge actions are never restored or submitted automatically.
use crate::providers::write::{Action, Context};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

fn path(context: &Context, action: &Action) -> Result<Option<PathBuf>, String> {
    let kind = match action {
        Action::Create { .. } => "create".to_string(),
        Action::Review { number, .. } => format!("review-{number}"),
        Action::Merge { .. } => return Ok(None),
    };
    let mut hash = Sha256::new();
    for value in [
        &context.remote_url,
        &context.remote,
        &context.account,
        &kind,
    ] {
        hash.update((value.len() as u64).to_le_bytes());
        hash.update(value.as_bytes());
    }
    let id = format!("{:x}", hash.finalize());
    let repo = git2::Repository::open(&context.path).map_err(|e| e.to_string())?;
    Ok(Some(
        repo.path()
            .join("recorde/provider-drafts")
            .join(format!("{id}.json")),
    ))
}

pub fn load(context: &Context, action: &Action) -> Result<Option<Action>, String> {
    let Some(path) = path(context, action)? else {
        return Ok(None);
    };
    let metadata = match std::fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        result => result.map_err(|e| e.to_string())?,
    };
    if !metadata.is_file() || metadata.len() > 262144 {
        return Err("Provider draft is invalid or too large.".into());
    }
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let saved: Action =
        serde_json::from_slice(&bytes).map_err(|_| "Cannot read the saved provider draft.")?;
    match (action, saved) {
        (Action::Create { .. }, saved @ Action::Create { .. }) => Ok(Some(saved)),
        (
            Action::Review {
                number,
                head,
                target,
                ..
            },
            Action::Review {
                number: saved_number,
                body,
                decision,
                inline,
                ..
            },
        ) if *number == saved_number => {
            // Keep freshly loaded commit/target guards, never resurrect an old approval target.
            Ok(Some(Action::Review {
                number: *number,
                head: head.clone(),
                target: target.clone(),
                body,
                decision,
                inline,
            }))
        }
        _ => Ok(None),
    }
}

pub fn save(context: &Context, action: &Action) -> Result<(), String> {
    use std::io::Write;
    let Some(path) = path(context, action)? else {
        return Ok(());
    };
    let bytes = serde_json::to_vec(action).map_err(|e| e.to_string())?;
    if bytes.len() > 262144 {
        return Err("Provider draft exceeds 256 KiB; shorten the text to save it.".into());
    }
    let directory = path.parent().ok_or("Invalid provider draft path.")?;
    std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    let mut temp = tempfile::NamedTempFile::new_in(directory).map_err(|e| e.to_string())?;
    temp.write_all(&bytes).map_err(|e| e.to_string())?;
    temp.as_file().sync_all().map_err(|e| e.to_string())?;
    temp.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn remove(context: &Context, action: &Action) -> Result<(), String> {
    let Some(path) = path(context, action)? else {
        return Ok(());
    };
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::write::Decision;
    #[test]
    fn drafts_survive_reload_but_cannot_cross_accounts_or_restore_stale_commit_guards() {
        let repo = crate::git::test_support::TestRepo::new();
        let mut context = Context {
            projects: Vec::new(),
            path: repo.path().into(),
            remote: "origin".into(),
            remote_url: "https://github.com/team/repo".into(),
            host: "github.com".into(),
            owner: "team".into(),
            project: "repo".into(),
            provider: crate::git::capabilities::ProviderType::GitHub,
            account: "alice".into(),
        };
        let review = |head: &str, body: &str| Action::Review {
            number: 42,
            head: head.into(),
            target: "main".into(),
            body: body.into(),
            decision: Decision::Comment,
            inline: None,
        };
        save(&context, &review("old", "unfinished review")).unwrap();
        assert!(
            load(&context, &review("new", "")).unwrap() == Some(review("new", "unfinished review"))
        );
        context.account = "bob".into();
        assert!(load(&context, &review("new", "")).unwrap().is_none());
        context.account = "alice".into();
        remove(&context, &review("new", "")).unwrap();
        assert!(load(&context, &review("new", "")).unwrap().is_none());
    }
}
