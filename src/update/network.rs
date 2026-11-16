//! Bounded GitHub release transport. Credentials are used only on the initial API request.
use recorde_update_signing::{self as protocol, Manifest, Target};
use serde::Deserialize;
use std::io::{Read, Write};
use std::time::Duration;

const API: &str = "https://api.github.com/repos/antonydoyle/Recorde";
const MAX_METADATA: usize = 2 * 1024 * 1024;
#[derive(Deserialize)]
struct Asset {
    id: u64,
    name: String,
    browser_download_url: String,
}
#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}

pub struct Offer {
    pub manifest: Manifest,
    pub envelope: Vec<u8>,
}

fn api_url(suffix: &str) -> String {
    format!("{API}/{suffix}")
}
fn read_bounded(response: ureq::Response, limit: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut reader = response.into_reader().take(limit as u64 + 1);
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let mut buffer = [0u8; 8192];
    loop {
        if std::time::Instant::now() >= deadline {
            return Err("Update metadata exceeded its overall time limit.".into());
        }
        let count = reader
            .read(&mut buffer)
            .map_err(|_| "Update response interrupted.")?;
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    if bytes.len() > limit {
        return Err("Update response exceeds its size limit.".into());
    }
    Ok(bytes)
}
fn trusted_redirect(url: &url::Url) -> bool {
    // These CDN hosts serve many repositories. The verified signature and signed
    // artifact size/digest establish integrity; the host list only limits transport.
    url.scheme() == "https"
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
        && match url.host_str() {
            Some(
                "release-assets.githubusercontent.com"
                | "objects.githubusercontent.com"
                | "github-releases.githubusercontent.com",
            ) => true,
            Some("github.com") => url
                .path()
                .starts_with("/antonydoyle/Recorde/releases/download/"),
            _ => false,
        }
}
fn fetch(initial: &str, token: Option<&str>, asset: bool) -> Result<ureq::Response, String> {
    let first = url::Url::parse(initial).map_err(|_| "Invalid update endpoint.")?;
    if first.scheme() != "https"
        || first.host_str() != Some("api.github.com")
        || first.port().is_some()
        || !first.username().is_empty()
        || first.password().is_some()
        || first.fragment().is_some()
        || !(first.path() == "/repos/antonydoyle/Recorde/releases"
            || first
                .path()
                .starts_with("/repos/antonydoyle/Recorde/releases/"))
    {
        return Err("Update request is outside the fixed Recorde release API.".into());
    }
    fetch_transport(first, token, asset, trusted_redirect)
}
fn fetch_transport(
    first: url::Url,
    token: Option<&str>,
    asset: bool,
    redirect_allowed: impl Fn(&url::Url) -> bool,
) -> Result<ureq::Response, String> {
    let agent = ureq::AgentBuilder::new()
        .redirects(0)
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(10))
        .timeout_write(Duration::from_secs(10))
        .timeout(Duration::from_secs(if asset { 7200 } else { 30 }))
        .build();
    let mut current = first;
    for hop in 0..=3 {
        let mut request = agent
            .get(current.as_str())
            .set("User-Agent", "Recorde-Updater")
            .set(
                "Accept",
                if asset {
                    "application/octet-stream"
                } else {
                    "application/vnd.github+json"
                },
            );
        if hop == 0 {
            request = request.set("X-GitHub-Api-Version", "2022-11-28");
            if let Some(token) = token {
                crate::auth::redact::remember(token);
                request = request.set("Authorization", &format!("Bearer {token}"));
            }
        }
        let response = match request.call() {
            Ok(response) => response,
            Err(ureq::Error::Status(_, response)) => response,
            Err(_) => return Err("Cannot reach the Recorde release service.".into()),
        };
        if response.status() == 200 {
            return Ok(response);
        }
        if asset && matches!(response.status(), 301 | 302 | 303 | 307 | 308) && hop < 3 {
            let next = current
                .join(
                    response
                        .header("Location")
                        .ok_or("Update redirect has no destination.")?,
                )
                .map_err(|_| "Invalid update redirect.")?;
            if !redirect_allowed(&next) {
                return Err("Update redirect to an untrusted destination was blocked.".into());
            }
            current = next;
            continue;
        }
        return Err(match response.status() {401|403=>"Release access denied. Check the explicitly selected GitHub account and its repository access.",404=>"No matching Recorde release is accessible. Private releases require an explicitly selected GitHub account.",429=>"Release service rate limit reached; try later.",_=>"Release service did not return a successful response."}.into());
    }
    Err("Too many update redirects.".into())
}
fn release(tag: &str, token: Option<&str>) -> Result<Release, String> {
    let path = if tag == "latest" {
        "releases/latest".into()
    } else {
        format!(
            "releases/tags/{}",
            percent_encoding::utf8_percent_encode(tag, percent_encoding::NON_ALPHANUMERIC)
        )
    };
    serde_json::from_slice(&read_bounded(
        fetch(&api_url(&path), token, false)?,
        MAX_METADATA,
    )?)
    .map_err(|_| "Invalid release metadata.".into())
}
pub fn check(
    target: &Target,
    token: Option<&str>,
    keys: &protocol::KeySet,
) -> Result<Offer, String> {
    let release = if target.channel == "stable" {
        release("latest", token)?
    } else {
        let releases: Vec<Release> = serde_json::from_slice(&read_bounded(
            fetch(&api_url("releases?per_page=30"), token, false)?,
            MAX_METADATA,
        )?)
        .map_err(|_| "Invalid release metadata.")?;
        releases
            .into_iter()
            .filter(|r| !r.draft)
            .filter_map(|r| {
                semver::Version::parse(r.tag_name.strip_prefix('v')?)
                    .ok()
                    .map(|v| (v, r))
            })
            .max_by(|a, b| a.0.cmp_precedence(&b.0))
            .map(|(_, r)| r)
            .ok_or("No accessible preview release.")?
    };
    if release.draft || (target.channel == "stable" && release.prerelease) {
        return Err("Release is not published in the selected channel.".into());
    }
    let matches: Vec<_> = release
        .assets
        .iter()
        .filter(|a| a.name == target.manifest_name())
        .collect();
    if matches.len() != 1 {
        return Err(
            "This release has no unique signed update for this installation. Install it manually."
                .into(),
        );
    }
    let asset = matches[0];
    let envelope = read_bounded(
        fetch(
            &api_url(&format!("releases/assets/{}", asset.id)),
            token,
            true,
        )?,
        protocol::MAX_ENVELOPE,
    )?;
    let manifest = protocol::verify(&envelope, keys)?;
    if release.tag_name != format!("v{}", manifest.version) {
        return Err("Signed update does not match the selected release.".into());
    }
    Ok(Offer { manifest, envelope })
}
pub fn download(
    manifest: &Manifest,
    token: Option<&str>,
    writer: impl Write,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<(), String> {
    let release = release(&format!("v{}", manifest.version), token)?;
    if release.draft || release.tag_name != format!("v{}", manifest.version) {
        return Err("Update release is not published.".into());
    }
    let assets: Vec<_> = release
        .assets
        .iter()
        .filter(|a| a.name == manifest.artifact_name && a.browser_download_url == manifest.url)
        .collect();
    if assets.len() != 1 {
        return Err("Signed artifact does not uniquely match the release assets.".into());
    }
    let response = fetch(
        &api_url(&format!("releases/assets/{}", assets[0].id)),
        token,
        true,
    )?;
    protocol::copy_verified_cancelled(
        response.into_reader(),
        writer,
        manifest,
        cancel,
        Duration::from_secs((manifest.size / (64 * 1024) + 120).clamp(300, 7200)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn api_scope_rejects_prefix_confusion_and_credentials_before_network() {
        for endpoint in [
            "https://api.github.com/repos/antonydoyle/Recorde/releasesX",
            "https://api.github.com/repos/other/Recorde/releases",
            "https://user@api.github.com/repos/antonydoyle/Recorde/releases",
            "http://api.github.com/repos/antonydoyle/Recorde/releases",
        ] {
            assert!(fetch(endpoint, Some("fixture-never-sent"), false)
                .err()
                .unwrap()
                .contains("outside"));
        }
    }
    #[test]
    fn redirects_allow_only_fixed_https_asset_hosts_without_credentials() {
        for url in [
            "https://release-assets.githubusercontent.com/a?signature=opaque",
            "https://github.com/antonydoyle/Recorde/releases/download/v1.0.0/app",
        ] {
            assert!(trusted_redirect(&url::Url::parse(url).unwrap()));
        }
        for url in [
            "http://release-assets.githubusercontent.com/a",
            "https://release-assets.githubusercontent.com.evil.test/a",
            "https://user@objects.githubusercontent.com/a",
            "https://objects.githubusercontent.com:444/a",
            "https://api.github.com/repos/other/repo",
            "https://github.com/other/repo/releases/download/v1.0/app",
        ] {
            assert!(!trusted_redirect(&url::Url::parse(url).unwrap()));
        }
    }
    #[test]
    fn redirected_asset_download_strips_authorization_and_blocks_untrusted_destination() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        for allow in [false, true] {
            let first = TcpListener::bind("127.0.0.1:0").unwrap();
            let second = TcpListener::bind("127.0.0.1:0").unwrap();
            first.set_nonblocking(true).unwrap();
            second.set_nonblocking(true).unwrap();
            eprintln!(
                "Updater redirect fixture: pid={}, ports={},{}",
                std::process::id(),
                first.local_addr().unwrap().port(),
                second.local_addr().unwrap().port()
            );
            let first_url = format!("http://{}/releases/assets/1", first.local_addr().unwrap());
            let second_url = format!("http://{}/asset", second.local_addr().unwrap());
            let redirect = second_url.clone();
            let source = std::thread::spawn(move || {
                let deadline = std::time::Instant::now() + Duration::from_secs(5);
                let mut stream = loop {
                    match first.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error)
                            if error.kind() == std::io::ErrorKind::WouldBlock
                                && std::time::Instant::now() < deadline =>
                        {
                            std::thread::sleep(Duration::from_millis(10))
                        }
                        Err(error) => {
                            panic!("Redirect source fixture did not receive a request: {error}")
                        }
                    }
                };
                // Accepted sockets inherit the listener's non-blocking mode on Windows and macOS.
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut bytes = [0; 4096];
                let count = stream.read(&mut bytes).unwrap();
                let request = String::from_utf8_lossy(&bytes[..count]).to_ascii_lowercase();
                assert!(request.contains("authorization: bearer fixture-update-secret"));
                write!(stream,"HTTP/1.1 302 Found\r\nLocation: {redirect}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
            });
            let destination = std::thread::spawn(move || {
                let deadline =
                    std::time::Instant::now() + Duration::from_secs(if allow { 5 } else { 1 });
                loop {
                    match second.accept() {
                        Ok((mut stream, _)) => {
                            assert!(allow);
                            // Windows sockets inherit the listener's non-blocking
                            // mode; read the request with a blocking timeout.
                            stream.set_nonblocking(false).unwrap();
                            stream
                                .set_read_timeout(Some(Duration::from_secs(5)))
                                .unwrap();
                            let mut bytes = [0; 4096];
                            let count = stream.read(&mut bytes).unwrap();
                            let request =
                                String::from_utf8_lossy(&bytes[..count]).to_ascii_lowercase();
                            assert!(!request.contains("authorization:"));
                            assert!(!request.contains("fixture-update-secret"));
                            write!(stream,"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok").unwrap();
                            return;
                        }
                        Err(e)
                            if e.kind() == std::io::ErrorKind::WouldBlock
                                && std::time::Instant::now() < deadline =>
                        {
                            std::thread::sleep(Duration::from_millis(10))
                        }
                        Err(_) => {
                            assert!(!allow);
                            return;
                        }
                    }
                }
            });
            let result = fetch_transport(
                url::Url::parse(&first_url).unwrap(),
                Some("fixture-update-secret"),
                true,
                |url| allow && url.as_str() == second_url,
            );
            assert_eq!(result.is_ok(), allow);
            source.join().unwrap();
            destination.join().unwrap();
        }
    }
}
