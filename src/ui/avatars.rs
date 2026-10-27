//! Optional author pictures. Off by default because looking one up sends a
//! hash of the author's email address to Gravatar (or, for GitHub no-reply
//! addresses, the account number to GitHub). Downloads run on one background
//! thread and are cached on disk for a week; rows show initials meanwhile.
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

const SIZE: u32 = 64;
const MAX_BYTES: u64 = 512 * 1024;
const MAX_ENTRIES: usize = 4000;
const CACHE_DAYS: u64 = 7;

enum Entry {
    Pending,
    Ready(egui::TextureHandle),
    Missing,
}

#[derive(Default)]
struct Cache {
    enabled: bool,
    entries: HashMap<String, Entry>,
    queue: VecDeque<String>,
    worker: bool,
}

impl Cache {
    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            // A queued lookup has not started and must be eligible to queue
            // again when pictures are re-enabled. Keep the in-flight lookup
            // (already removed from this queue) and completed cache results.
            while let Some(email) = self.queue.pop_front() {
                if matches!(self.entries.get(&email), Some(Entry::Pending)) {
                    self.entries.remove(&email);
                }
            }
        }
    }
}

fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

pub fn set_enabled(enabled: bool) {
    cache().lock().unwrap().set_enabled(enabled);
}

/// The picture for `email`, if enabled and already downloaded. The first
/// request queues a download and repaints when it arrives.
pub fn texture(ctx: &egui::Context, email: &str) -> Option<egui::TextureId> {
    let key = email.trim().to_lowercase();
    if key.is_empty() {
        return None;
    }
    let mut cache = cache().lock().unwrap();
    if !cache.enabled {
        return None;
    }
    match cache.entries.get(&key) {
        Some(Entry::Ready(texture)) => return Some(texture.id()),
        Some(_) => return None,
        None => {}
    }
    if cache.entries.len() >= MAX_ENTRIES {
        return None;
    }
    cache.entries.insert(key.clone(), Entry::Pending);
    cache.queue.push_back(key);
    if !cache.worker {
        cache.worker = true;
        let ctx = ctx.clone();
        std::thread::spawn(move || worker(ctx));
    }
    None
}

fn worker(ctx: egui::Context) {
    loop {
        let next = {
            let mut cache = cache().lock().unwrap();
            let next = cache.queue.pop_front().filter(|_| cache.enabled);
            if next.is_none() {
                cache.worker = false;
            }
            next
        };
        let Some(email) = next else { return };
        let entry = match load(&email) {
            Some(image) => Entry::Ready(ctx.load_texture(
                format!("avatar:{email}"),
                image,
                egui::TextureOptions::LINEAR,
            )),
            None => Entry::Missing,
        };
        cache().lock().unwrap().entries.insert(email, entry);
        ctx.request_repaint();
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Where a picture for this address can be found.
pub(crate) fn source_url(email: &str) -> String {
    // <id>+<login>@users.noreply.github.com belongs to GitHub account <id>.
    if let Some(local) = email.strip_suffix("@users.noreply.github.com") {
        if let Some((id, _)) = local.split_once('+') {
            if !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) {
                return format!("https://avatars.githubusercontent.com/u/{id}?s={SIZE}&v=4");
            }
        }
    }
    use sha2::Digest;
    let hash = hex(&sha2::Sha256::digest(email.as_bytes()));
    format!("https://gravatar.com/avatar/{hash}?s={SIZE}&d=404")
}

fn cache_path(email: &str) -> Option<PathBuf> {
    use sha2::Digest;
    let dir = crate::app_dirs::cache_dir()?
        .join("recorde")
        .join("avatars");
    Some(dir.join(hex(&sha2::Sha256::digest(email.as_bytes()))))
}

fn fresh(path: &std::path::Path) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|time| time.elapsed().ok())
        .is_some_and(|age| age.as_secs() < CACHE_DAYS * 86_400)
}

fn load(email: &str) -> Option<egui::ColorImage> {
    let cached = cache_path(email);
    if let Some(path) = &cached {
        if fresh(&path.with_extension("none")) {
            return None;
        }
        if fresh(&path.with_extension("img")) {
            if let Some(image) = std::fs::read(path.with_extension("img"))
                .ok()
                .and_then(|bytes| decode(&bytes))
            {
                return Some(image);
            }
        }
    }
    let (bytes, missing) = download(&source_url(email));
    if let Some(path) = &cached {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match &bytes {
            Some(bytes) => {
                let _ = std::fs::write(path.with_extension("img"), bytes);
            }
            // Remember only a definite "no picture"; retry network errors next session.
            None if missing => {
                let _ = std::fs::write(path.with_extension("none"), b"");
            }
            None => {}
        }
    }
    bytes.and_then(|bytes| decode(&bytes))
}

/// Returns the body, or whether the service said there is no picture.
fn download(url: &str) -> (Option<Vec<u8>>, bool) {
    use std::io::Read;
    let agent = ureq::AgentBuilder::new()
        .redirects(3)
        .timeout(std::time::Duration::from_secs(10))
        .build();
    match agent.get(url).set("User-Agent", "Recorde").call() {
        Ok(response) => {
            let mut bytes = Vec::new();
            let read = response
                .into_reader()
                .take(MAX_BYTES + 1)
                .read_to_end(&mut bytes);
            if read.is_err() || bytes.len() as u64 > MAX_BYTES {
                return (None, false);
            }
            (Some(bytes), false)
        }
        Err(ureq::Error::Status(404, _)) => (None, true),
        Err(_) => (None, false),
    }
}

/// Decode, scale to a square and clear the corners so it draws as a circle.
pub(crate) fn decode(bytes: &[u8]) -> Option<egui::ColorImage> {
    let image = image::load_from_memory(bytes).ok()?;
    let image = image
        .resize_to_fill(SIZE, SIZE, image::imageops::FilterType::Triangle)
        .to_rgba8();
    let radius = SIZE as f32 / 2.0;
    let mut pixels = image.into_raw();
    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as f32 + 0.5 - radius;
            let dy = y as f32 + 0.5 - radius;
            // One pixel of anti-aliasing at the edge.
            let coverage = (radius - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
            let alpha = &mut pixels[((y * SIZE + x) * 4 + 3) as usize];
            *alpha = (*alpha as f32 * coverage) as u8;
        }
    }
    Some(egui::ColorImage::from_rgba_unmultiplied(
        [SIZE as usize, SIZE as usize],
        &pixels,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabling_drops_queued_pending_lookups_but_preserves_in_flight_and_results() {
        let ctx = egui::Context::default();
        let texture = ctx.load_texture(
            "completed-avatar",
            egui::ColorImage::from_rgba_unmultiplied([1, 1], &[255; 4]),
            egui::TextureOptions::LINEAR,
        );
        let texture_id = texture.id();
        // Use a local cache so this fixture never queues network work or
        // changes the application's shared avatar preference.
        let mut cache = Cache {
            enabled: true,
            worker: true,
            entries: HashMap::from([
                ("in-flight".into(), Entry::Pending),
                ("queued-one".into(), Entry::Pending),
                ("queued-two".into(), Entry::Pending),
                ("completed".into(), Entry::Ready(texture)),
                ("missing".into(), Entry::Missing),
            ]),
            queue: VecDeque::from(["queued-one".into(), "queued-two".into()]),
        };
        cache.set_enabled(false);
        cache.set_enabled(true);
        assert!(cache.enabled && cache.worker);
        assert!(cache.queue.is_empty());
        assert!(!cache.entries.contains_key("queued-one"));
        assert!(!cache.entries.contains_key("queued-two"));
        assert!(matches!(
            cache.entries.get("in-flight"),
            Some(Entry::Pending)
        ));
        assert!(matches!(cache.entries.get("missing"), Some(Entry::Missing)));
        assert!(
            matches!(cache.entries.get("completed"), Some(Entry::Ready(handle)) if handle.id() == texture_id)
        );

        // The existing worker may finish while pictures are disabled; its
        // result remains reusable rather than being discarded on re-enable.
        cache.set_enabled(false);
        cache.entries.insert("in-flight".into(), Entry::Missing);
        cache.worker = false;
        cache.set_enabled(true);
        assert!(matches!(
            cache.entries.get("in-flight"),
            Some(Entry::Missing)
        ));
        assert!(!cache.worker);
    }

    #[test]
    fn urls_use_github_ids_or_hashed_emails_only() {
        assert_eq!(
            source_url("12345+octo@users.noreply.github.com"),
            "https://avatars.githubusercontent.com/u/12345?s=64&v=4"
        );
        let gravatar = source_url("someone@example.com");
        assert!(gravatar.starts_with("https://gravatar.com/avatar/"));
        assert!(!gravatar.contains("someone"));
        assert!(gravatar.ends_with("?s=64&d=404"));
        // A no-reply address without an account number is hashed as usual.
        assert!(source_url("octo@users.noreply.github.com").contains("gravatar.com"));
    }

    #[test]
    fn decoded_pictures_are_round_and_disabled_lookups_do_nothing() {
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(10, 20, image::Rgba([200, 10, 10, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let image = decode(&png).unwrap();
        assert_eq!(image.size, [64, 64]);
        assert_eq!(image.pixels[0].a(), 0, "corners are transparent");
        assert_eq!(image.pixels[32 * 64 + 32].a(), 255);
        assert!(decode(b"not an image").is_none());

        set_enabled(false);
        let ctx = egui::Context::default();
        assert!(texture(&ctx, "someone@example.com").is_none());
        assert!(cache().lock().unwrap().queue.is_empty());
    }
}
