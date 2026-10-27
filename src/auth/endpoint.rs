//! Parse credential destinations without suffix or substring host matching.
use url::Url;

pub struct Endpoint {
    pub host: String,
    pub username: Option<String>,
    pub namespace: String,
    pub project: String,
}

pub fn normalize_host(host: &str) -> Result<String, String> {
    let host = host.trim();
    if host.is_empty()
        || host
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '/' | '\\' | '@' | '?' | '#'))
    {
        return Err("Enter a hostname, optionally followed by an HTTPS port.".into());
    }
    let url =
        Url::parse(&format!("https://{host}/")).map_err(|_| "Invalid account host.".to_string())?;
    authority(&url)
}

fn authority(url: &Url) -> Result<String, String> {
    let host = url.host_str().ok_or("Remote URL has no host.")?;
    // Do not silently collapse a trailing dot or non-default port to another account.
    Ok(match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    })
}

/// SSH remotes are accepted only for API resolution; PATs are never SSH passwords.
pub fn parse(remote: &str, for_api: bool) -> Result<Endpoint, String> {
    if remote.chars().any(|c| c.is_control() || c == '\\') {
        return Err("Invalid remote URL.".into());
    }
    let expanded;
    let input = if for_api && !remote.contains("://") {
        let (host, path) = remote.split_once(':').ok_or("Unsupported remote URL.")?;
        if !host.contains('@') || path.starts_with('/') {
            return Err("Unsupported SSH remote URL.".into());
        }
        expanded = format!("ssh://{host}/{path}");
        &expanded
    } else {
        remote
    };
    let url = Url::parse(input).map_err(|_| "Invalid remote URL.".to_string())?;
    let ssh = url.scheme() == "ssh";
    if url.scheme() != "https" && !(for_api && ssh) {
        return Err(
            "Saved tokens require HTTPS (or an SSH remote for provider API access).".into(),
        );
    }
    if url.password().is_some() || url.query().is_some() || url.fragment().is_some() {
        return Err("Remove passwords, query parameters and fragments from the remote URL.".into());
    }
    let host = if ssh {
        url.host_str().ok_or("Remote URL has no host.")?.to_string()
    } else {
        authority(&url)?
    };
    let username = if ssh || url.username().is_empty() {
        None
    } else {
        let name = percent_encoding::percent_decode_str(url.username())
            .decode_utf8()
            .map_err(|_| "Invalid remote username.".to_string())?
            .into_owned();
        if name.chars().any(char::is_control) {
            return Err("Invalid remote username.".into());
        }
        Some(name)
    };
    let path = url
        .path()
        .trim_matches('/')
        .strip_suffix(".git")
        .unwrap_or(url.path().trim_matches('/'));
    let decoded = percent_encoding::percent_decode_str(path)
        .decode_utf8()
        .map_err(|_| "Invalid repository path encoding.")?;
    if decoded.chars().any(|c| c.is_control() || c == '\\')
        || decoded
            .split('/')
            .any(|part| matches!(part, "" | "." | ".."))
    {
        return Err("Invalid repository path.".into());
    }
    let parts: Vec<_> = decoded.split('/').collect();
    let azure = if host == "dev.azure.com" {
        match parts.as_slice() {
            [org, project, "_git", repo] => Some((org.to_string(), *project, *repo)),
            _ => return Err("Azure remote must use organization/project/_git/repository.".into()),
        }
    } else if ssh
        && matches!(
            host.as_str(),
            "ssh.dev.azure.com" | "vs-ssh.visualstudio.com"
        )
    {
        match parts.as_slice() {
            ["v3", org, project, repo] => Some((org.to_string(), *project, *repo)),
            _ => {
                return Err("Azure SSH remote must use v3/organization/project/repository.".into())
            }
        }
    } else if let Some(org) = host
        .strip_suffix(".visualstudio.com")
        .filter(|s| !s.is_empty() && !s.contains('.'))
    {
        match parts.as_slice() {
            [project, "_git", repo] | ["DefaultCollection", project, "_git", repo] => {
                Some((org.to_string(), *project, *repo))
            }
            _ => return Err("Invalid legacy Azure repository URL.".into()),
        }
    } else {
        None
    };
    if let Some((org, project, repo)) = azure {
        // Azure clone URLs commonly carry the organization as a placeholder user.
        let username = username.filter(|user| user != &org);
        return Ok(Endpoint {
            host: "dev.azure.com".into(),
            username,
            namespace: format!("{org}/{project}"),
            project: repo.into(),
        });
    }
    let (namespace, project) = decoded
        .rsplit_once('/')
        .ok_or("Remote URL must include an owner and repository.")?;
    if namespace.is_empty() || project.is_empty() {
        return Err("Invalid repository path.".into());
    }
    Ok(Endpoint {
        host,
        username,
        namespace: namespace.into(),
        project: project.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_azure_ssh_uses_v3_organization_and_project() {
        let endpoint = parse("org@vs-ssh.visualstudio.com:v3/org/project/repo", true).unwrap();
        assert_eq!(endpoint.host, "dev.azure.com");
        assert_eq!(endpoint.namespace, "org/project");
        assert_eq!(endpoint.project, "repo");
        assert_eq!(endpoint.username, None);
    }
    #[test]
    fn azure_cloud_https_legacy_and_ssh_resolve_the_same_project() {
        for url in [
            "https://org@dev.azure.com/org/My%20Project/_git/repo",
            "https://org.visualstudio.com/My%20Project/_git/repo",
            "https://org.visualstudio.com/DefaultCollection/My%20Project/_git/repo",
            "git@ssh.dev.azure.com:v3/org/My%20Project/repo",
            "ssh://git@ssh.dev.azure.com/v3/org/My%20Project/repo",
        ] {
            let endpoint = parse(url, true).unwrap();
            assert_eq!(endpoint.host, "dev.azure.com", "{url}");
            assert_eq!(endpoint.namespace, "org/My Project", "{url}");
            assert_eq!(endpoint.project, "repo");
            assert!(endpoint.username.is_none());
        }
        for url in [
            "https://dev.azure.com/org/_git/repo",
            "https://dev.azure.com/org/project/_git/repo/extra",
            "git@ssh.dev.azure.com:v3/org/repo",
            "https://dev.azure.com/org/project/_git/%2e%2e",
            "https://dev.azure.com/org/project/_git/repo%0a",
        ] {
            assert!(parse(url, true).is_err(), "{url}");
        }
        for host in [
            "dev.azure.com.evil.test",
            "org.visualstudio.com.evil.test",
            "org.evil.visualstudio.com",
        ] {
            assert_eq!(
                parse(&format!("https://{host}/org/project"), true)
                    .unwrap()
                    .host,
                host
            );
        }
        assert_eq!(
            parse("https://alice@dev.azure.com/org/project/_git/repo", true)
                .unwrap()
                .username
                .as_deref(),
            Some("alice")
        );
    }
    #[test]
    fn exact_https_authority_and_username() {
        let endpoint = parse("https://Alice@GITHUB.com:443/org/repo.git", false).unwrap();
        assert_eq!(endpoint.host, "github.com");
        assert_eq!(endpoint.username.as_deref(), Some("Alice"));
        assert_eq!(
            parse("https://github.com:8443/org/repo", false)
                .unwrap()
                .host,
            "github.com:8443"
        );
        assert_eq!(
            parse("https://github.com.evil.test/org/repo", false)
                .unwrap()
                .host,
            "github.com.evil.test"
        );
        assert_eq!(
            parse("https://github.com@evil.test/org/repo", false)
                .unwrap()
                .host,
            "evil.test"
        );
    }
    #[test]
    fn rejects_unsafe_or_ambiguous_destinations() {
        for url in [
            "http://github.com/a/b",
            "file:///a/b",
            "https://a:secret@github.com/a/b",
            "https://github.com/a/b?token=x",
            "https://github.com/a/b\n",
            "https://github.com\\@evil.test/a/b",
            "git@github.com:a/b",
        ] {
            assert!(parse(url, false).is_err(), "{url}");
        }
        for host in [
            "https://github.com",
            "github.com/path",
            "github.com@evil.test",
            "github.com?x",
            "github.com\nother",
        ] {
            assert!(normalize_host(host).is_err());
        }
    }
    #[test]
    fn ssh_api_and_nested_gitlab_paths() {
        let endpoint = parse("git@gitlab.com:group/subgroup/project.git", true).unwrap();
        assert_eq!(endpoint.host, "gitlab.com");
        assert_eq!(endpoint.namespace, "group/subgroup");
        assert_eq!(endpoint.project, "project");
        assert!(endpoint.username.is_none());
        assert!(parse("ssh://git@gitlab.com:2222/group/project", true).is_ok());
    }
}
