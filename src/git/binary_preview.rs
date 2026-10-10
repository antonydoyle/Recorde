//! Bounded binary snapshots prepared on the diff worker, never during painting.
use std::io::{Cursor, Read};

const MAX_BYTES: u64 = 8 * 1024 * 1024;
const MAX_DIMENSION: u32 = 4096;

#[derive(Debug, Clone)]
pub struct ImagePreview {
    pub original_size: [u32; 2],
    pub size: [usize; 2],
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone)]
pub enum BinarySide {
    Missing,
    Unavailable(String),
    Content {
        size: u64,
        hex: String,
        image: Option<ImagePreview>,
        note: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub struct BinaryPreview {
    pub before: BinarySide,
    pub after: BinarySide,
}

pub(crate) fn snapshot(
    repo: &git2::Repository,
    delta: &git2::DiffDelta<'_>,
    workdir: bool,
) -> BinaryPreview {
    let before = if matches!(delta.status(), git2::Delta::Added | git2::Delta::Untracked) {
        BinarySide::Missing
    } else {
        blob(repo, delta.old_file().id())
    };
    let after = if delta.status() == git2::Delta::Deleted {
        BinarySide::Missing
    } else if workdir {
        match repo.workdir().zip(delta.new_file().path()) {
            Some((root, path)) => match path
                .to_str()
                .ok_or_else(|| "Path encoding is unsupported.".to_string())
                .and_then(|file| super::safe_path::ScopedFile::new(root, file))
            {
                Ok(path) => working_file(&path),
                Err(error) => BinarySide::Unavailable(error),
            },
            None => BinarySide::Unavailable("Working file is unavailable.".into()),
        }
    } else {
        blob(repo, delta.new_file().id())
    };
    BinaryPreview { before, after }
}

fn blob(repo: &git2::Repository, oid: git2::Oid) -> BinarySide {
    let result = (|| {
        let (size, kind) = repo.odb()?.read_header(oid)?;
        if kind != git2::ObjectType::Blob {
            return Err(git2::Error::from_str("This Git object is not a file."));
        }
        if size as u64 > MAX_BYTES {
            return Ok(content(&[], size as u64));
        }
        let blob = repo.find_blob(oid)?;
        Ok(content(blob.content(), blob.size() as u64))
    })();
    result.unwrap_or_else(|e: git2::Error| BinarySide::Unavailable(e.message().into()))
}

fn working_file(path: &super::safe_path::ScopedFile) -> BinarySide {
    let result = (|| {
        let file = path.open()?;
        let metadata = file.metadata()?;
        let size = metadata.len();
        let mut bytes = Vec::new();
        // Large files get only a short hex prefix; file growth cannot exceed the cap.
        file.take(if size > MAX_BYTES { 128 } else { MAX_BYTES + 1 })
            .read_to_end(&mut bytes)?;
        Ok(content(&bytes, size.max(bytes.len() as u64)))
    })();
    result.unwrap_or_else(|e: std::io::Error| BinarySide::Unavailable(e.to_string()))
}

fn content(bytes: &[u8], size: u64) -> BinarySide {
    use std::fmt::Write;
    let mut hex = String::new();
    for (row, chunk) in bytes
        .iter()
        .take(128)
        .copied()
        .collect::<Vec<_>>()
        .chunks(16)
        .enumerate()
    {
        let _ = write!(hex, "{:08x}  ", row * 16);
        for byte in chunk {
            let _ = write!(hex, "{byte:02x} ");
        }
        hex.push('\n');
    }
    let (image, note) = if size > MAX_BYTES {
        (
            None,
            Some("Image preview limited to files up to 8 MiB.".into()),
        )
    } else {
        match decode(bytes) {
            Ok(image) => (Some(image), None),
            Err(note) => (None, Some(note)),
        }
    };
    BinarySide::Content {
        size,
        hex,
        image,
        note,
    }
}

fn decode(bytes: &[u8]) -> Result<ImagePreview, String> {
    let format = image::guess_format(bytes)
        .map_err(|_| "Binary data; first 128 bytes shown in hexadecimal.")?;
    if !matches!(
        format,
        image::ImageFormat::Png
            | image::ImageFormat::Jpeg
            | image::ImageFormat::WebP
            | image::ImageFormat::Bmp
            | image::ImageFormat::Gif
    ) {
        return Err("Image preview supports PNG, JPEG, WebP, BMP and GIF (first frame); first 128 bytes shown.".into());
    }
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|_| "Image could not be decoded within preview limits (4096 pixels per side).")?;
    let original_size = [decoded.width(), decoded.height()];
    let thumbnail = if decoded.width() > 1024 || decoded.height() > 1024 {
        decoded.thumbnail(1024, 1024).to_rgba8()
    } else {
        decoded.to_rgba8()
    };
    Ok(ImagePreview {
        original_size,
        size: [thumbnail.width() as usize, thumbnail.height() as usize],
        rgba: thumbnail.into_raw(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{operations::compute_workdir_diff, test_support::TestRepo};

    fn png(color: [u8; 4], width: u32) -> Vec<u8> {
        let image = image::RgbaImage::from_pixel(width, 2, image::Rgba(color));
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        bytes.into_inner()
    }

    fn pixel(side: &BinarySide) -> &[u8] {
        let BinarySide::Content {
            image: Some(image), ..
        } = side
        else {
            panic!("expected image: {side:?}")
        };
        &image.rgba[..4]
    }

    #[test]
    fn previews_use_head_index_and_worktree_snapshots() {
        let repo = TestRepo::new();
        repo.commit("image.png", &png([255, 0, 0, 255], 2));
        repo.write("image.png", &png([0, 255, 0, 255], 2));
        repo.git_ok(&["add", "image.png"]);
        repo.write("image.png", &png([0, 0, 255, 255], 2));
        let staged = compute_workdir_diff(repo.path(), "image.png", true)
            .unwrap()
            .binary_preview
            .unwrap();
        assert_eq!(pixel(&staged.before), [255, 0, 0, 255]);
        assert_eq!(pixel(&staged.after), [0, 255, 0, 255]);
        let working = compute_workdir_diff(repo.path(), "image.png", false)
            .unwrap()
            .binary_preview
            .unwrap();
        assert_eq!(pixel(&working.before), [0, 255, 0, 255]);
        assert_eq!(pixel(&working.after), [0, 0, 255, 255]);
        repo.git_ok(&["rm", "-f", "image.png"]);
        let deleted = compute_workdir_diff(repo.path(), "image.png", true)
            .unwrap()
            .binary_preview
            .unwrap();
        assert!(matches!(deleted.after, BinarySide::Missing));
    }

    #[test]
    fn preview_limits_reject_large_images_and_bound_binary_bytes() {
        assert!(decode(&png([1, 2, 3, 255], 4097)).is_err());
        assert!(decode(b"\x89PNG\r\n\x1a\ninvalid").is_err());
        let BinarySide::Content { hex, image, .. } = content(&[0; 256], 256) else {
            unreachable!()
        };
        assert_eq!(hex.lines().count(), 8);
        assert!(image.is_none());
        let BinarySide::Content { image, note, .. } =
            content(&png([1, 2, 3, 255], 2), MAX_BYTES + 1)
        else {
            unreachable!()
        };
        assert!(image.is_none() && note.unwrap().contains("8 MiB"));
    }

    #[test]
    fn jpeg_and_webp_decode_and_untracked_image_has_no_before_side() {
        for format in [
            image::ImageFormat::Jpeg,
            image::ImageFormat::WebP,
            image::ImageFormat::Bmp,
            image::ImageFormat::Gif,
        ] {
            let image = image::RgbImage::from_pixel(3, 2, image::Rgb([20, 40, 60]));
            let mut bytes = Cursor::new(Vec::new());
            image.write_to(&mut bytes, format).unwrap();
            assert_eq!(decode(bytes.get_ref()).unwrap().original_size, [3, 2]);
        }
        let repo = TestRepo::new();
        repo.write("new.png", &png([1, 2, 3, 255], 2));
        let diff = compute_workdir_diff(repo.path(), "new.png", false).unwrap();
        let preview = diff.binary_preview.unwrap();
        assert!(matches!(preview.before, BinarySide::Missing));
        assert_eq!(pixel(&preview.after), [1, 2, 3, 255]);
    }
}
