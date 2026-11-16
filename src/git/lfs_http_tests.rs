//! Real git-lfs transport against a disposable authenticated loopback server.
//! No provider account, external network or persistent credential is used.
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{
    atomic::{AtomicBool, AtomicU8, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

struct Server {
    url: String,
    fail_download: Arc<AtomicU8>,
    objects: Arc<Mutex<std::collections::HashMap<String, Vec<u8>>>>,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Server {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        eprintln!(
            "LFS fixture listener: {url}; owned by test PID {}",
            std::process::id()
        );
        let objects = Arc::new(Mutex::new(std::collections::HashMap::new()));
        let fail_download = Arc::new(AtomicU8::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (base, data, failures, stopping) = (
            url.clone(),
            objects.clone(),
            fail_download.clone(),
            stop.clone(),
        );
        let worker = std::thread::spawn(move || {
            while !stopping.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        // Accepted sockets inherit the listener's non-blocking mode on Windows and macOS.
                        let _ = stream.set_nonblocking(false);
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                        let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
                        if let Err(error) =
                            serve(&mut stream, &base, &data, failures.load(Ordering::Relaxed))
                        {
                            eprintln!("LFS fixture connection ended: {error}");
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10))
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            url,
            objects,
            fail_download,
            stop,
            worker: Some(worker),
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}

fn serve(
    stream: &mut TcpStream,
    base: &str,
    objects: &Mutex<std::collections::HashMap<String, Vec<u8>>>,
    fail: u8,
) -> std::io::Result<()> {
    let mut header = Vec::new();
    while !header.ends_with(b"\r\n\r\n") {
        let mut byte = [0];
        stream.read_exact(&mut byte)?;
        header.push(byte[0]);
        if header.len() > 32 * 1024 {
            return Err(std::io::Error::other("oversized header"));
        }
    }
    let text = String::from_utf8_lossy(&header);
    let mut first = text.lines().next().unwrap_or_default().split_whitespace();
    let method = first.next().unwrap_or_default();
    let path = first.next().unwrap_or_default();
    let headers: std::collections::HashMap<_, _> = text
        .lines()
        .skip(1)
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.trim()))
        .collect();
    if headers.get("authorization").copied() != Some("Bearer recorde-local-fixture") {
        return respond(stream, 401, b"unauthorized");
    }
    let length = headers
        .get("content-length")
        .and_then(|n| n.parse::<usize>().ok())
        .unwrap_or(0);
    if length > 1024 * 1024 {
        return respond(stream, 413, b"too large");
    }
    if headers
        .get("expect")
        .is_some_and(|value| value.eq_ignore_ascii_case("100-continue"))
    {
        stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n")?;
    }
    let mut body = vec![0; length];
    stream.read_exact(&mut body)?;
    if method == "POST" && path == "/objects/batch" {
        let request: serde_json::Value = serde_json::from_slice(&body)?;
        let operation = request["operation"].as_str().unwrap_or_default();
        let mut result = vec![];
        for object in request["objects"].as_array().into_iter().flatten() {
            let oid = object["oid"].as_str().unwrap_or_default();
            if oid.len() != 64 || !oid.bytes().all(|b| b.is_ascii_hexdigit()) {
                return respond(stream, 422, b"invalid oid");
            }
            let mut actions = serde_json::Map::new();
            actions.insert(operation.into(), serde_json::json!({"href":format!("{base}/objects/{oid}"),"header":{"Authorization":"Bearer recorde-local-fixture"}}));
            result.push(serde_json::json!({"oid":oid,"size":object["size"],"authenticated":true,"actions":actions}));
        }
        return respond(
            stream,
            200,
            &serde_json::to_vec(&serde_json::json!({"transfer":"basic","objects":result}))?,
        );
    }
    if let Some(oid) = path.strip_prefix("/objects/") {
        if method == "PUT" {
            use sha2::Digest;
            if format!("{:x}", sha2::Sha256::digest(&body)) != oid {
                return respond(stream, 422, b"hash mismatch");
            }
            objects.lock().unwrap().insert(oid.into(), body);
            return respond(stream, 200, b"");
        }
        if method == "GET" {
            if fail == 1 {
                return respond(stream, 401, b"expired credential");
            }
            if fail == 2 {
                return respond(stream, 503, b"temporarily unavailable");
            }
            if let Some(bytes) = objects.lock().unwrap().get(oid) {
                return respond(stream, 200, bytes);
            }
            return respond(stream, 404, b"missing object");
        }
    }
    respond(stream, 404, b"unknown endpoint")
}

fn respond(stream: &mut TcpStream, status: u16, bytes: &[u8]) -> std::io::Result<()> {
    write!(stream, "HTTP/1.1 {status} Fixture\r\nContent-Type: application/vnd.git-lfs+json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len())?;
    stream.write_all(bytes)
}

#[test]
#[ignore = "requires git-lfs; run in the network LFS CI step"]
fn authenticated_lfs_http_round_trip_and_failed_downloads() {
    use super::test_support::TestRepo;
    let server = Server::start();
    let repo = TestRepo::new();
    repo.commit("base", b"base");
    repo.git_ok(&["lfs", "install", "--local"]);
    repo.git_ok(&["lfs", "track", "*.asset"]);
    repo.git_ok(&["config", "lfs.url", &server.url]);
    repo.git_ok(&[
        "config",
        "http.extraheader",
        "Authorization: Bearer recorde-local-fixture",
    ]);
    let bytes = b"authenticated network LFS fixture\0payload\n";
    repo.write("file.asset", bytes);
    repo.git_ok(&["add", "."]);
    repo.git_ok(&["commit", "-m", "network fixture"]);
    let remote = tempfile::tempdir().unwrap();
    let bare = git2::Repository::init_bare(remote.path()).unwrap();
    bare.set_head("refs/heads/main").unwrap();
    repo.git_ok(&["remote", "add", "origin", remote.path().to_str().unwrap()]);
    repo.git_ok(&["push", "origin", "main"]);
    assert_eq!(
        server.objects.lock().unwrap().values().next().unwrap(),
        bytes
    );
    for failure in [0, 1, 2] {
        server.fail_download.store(failure, Ordering::Relaxed);
        let root = tempfile::tempdir().unwrap();
        let cloned = root.path().join("clone");
        let result = repo.git(&[
            "-c",
            "filter.lfs.process=git-lfs filter-process",
            "-c",
            "filter.lfs.required=true",
            "-c",
            &format!("lfs.url={}", server.url),
            "-c",
            "http.extraheader=Authorization: Bearer recorde-local-fixture",
            "-c",
            "lfs.transfer.maxretries=1",
            "-c",
            "lfs.transfer.maxretrydelay=1",
            "clone",
            remote.path().to_str().unwrap(),
            cloned.to_str().unwrap(),
        ]);
        if failure == 0 {
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            assert_eq!(std::fs::read(cloned.join("file.asset")).unwrap(), bytes);
        } else {
            assert!(!result.status.success());
            assert_ne!(
                std::fs::read(cloned.join("file.asset")).ok().as_deref(),
                Some(bytes.as_slice())
            );
            assert_eq!(
                std::fs::read(repo.path().join("file.asset")).unwrap(),
                bytes
            );
        }
    }
}
