//! Validated, content-addressed original image storage.
//!
//! Callers must hold the store lock while publishing or reading assets.

use std::fs::{self, OpenOptions};
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{bail, Context, Result};
use image::io::Reader;
use image::ImageFormat;
use serde::{Deserialize, Serialize};

const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_PIXELS: u64 = 40_000_000;
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageAsset {
    pub asset_id: String,
    pub content_hash: String,
    pub mime_type: String,
    pub byte_size: u64,
    pub width: u32,
    pub height: u32,
}

fn extension(format: ImageFormat) -> Result<(&'static str, &'static str)> {
    match format {
        ImageFormat::Png => Ok(("png", "image/png")),
        ImageFormat::Jpeg => Ok(("jpg", "image/jpeg")),
        ImageFormat::WebP => Ok(("webp", "image/webp")),
        _ => bail!("unsupported_image_type"),
    }
}

/// Check type, dimensions and complete decodability before publication.
pub fn validate(bytes: &[u8], declared_mime: &str) -> Result<ImageAsset> {
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        bail!("invalid_image_size");
    }
    let format = image::guess_format(bytes).context("invalid_image")?;
    let (_, mime) = extension(format)?;
    if mime != declared_mime {
        bail!("image_mime_mismatch");
    }
    let reader = Reader::with_format(Cursor::new(bytes), format);
    let (width, height) = reader.into_dimensions().context("invalid_image")?;
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_PIXELS {
        bail!("invalid_image_dimensions");
    }
    let mut reader = Reader::with_format(Cursor::new(bytes), format);
    let mut limits = image::io::Limits::default();
    limits.max_image_width = Some(width);
    limits.max_image_height = Some(height);
    limits.max_alloc = Some(512 * 1024 * 1024);
    reader.limits(limits);
    reader.decode().context("invalid_image")?;
    let hash = blake3::hash(bytes).to_hex().to_string();
    Ok(ImageAsset {
        asset_id: format!("asset_{hash}"),
        content_hash: hash,
        mime_type: mime.to_owned(),
        byte_size: bytes.len() as u64,
        width,
        height,
    })
}

fn asset_directory(store: &Path) -> Result<PathBuf> {
    let root = store.canonicalize().context("invalid_store_directory")?;
    let assets = root.join("assets");
    match fs::symlink_metadata(&assets) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            bail!("invalid_asset_directory");
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(&assets)?;
        }
        Err(error) => return Err(error.into()),
    }
    // Windows junctions/reparse points may not report as a symlink.
    if assets.canonicalize()? != assets {
        bail!("invalid_asset_directory");
    }
    Ok(assets)
}

fn asset_filename(asset: &ImageAsset) -> Result<String> {
    if asset.content_hash.len() != 64
        || !asset.content_hash.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        || asset.asset_id != format!("asset_{}", asset.content_hash)
    {
        bail!("invalid_asset_id");
    }
    let ext = match asset.mime_type.as_str() {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        _ => bail!("unsupported_image_type"),
    };
    Ok(format!("{}.{ext}", asset.content_hash))
}

/// Check whether the managed original is present without loading image bytes.
pub fn is_present(store: &Path, asset: &ImageAsset) -> Result<bool> {
    let root = store.canonicalize().context("invalid_store_directory")?;
    let directory = root.join("assets");
    match fs::symlink_metadata(&directory) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => return Ok(false),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    }
    if directory.canonicalize()? != directory {
        return Ok(false);
    }
    let path = directory.join(asset_filename(asset)?);
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(!metadata.file_type().is_symlink() && metadata.is_file()
            && metadata.len() == asset.byte_size && metadata.len() <= MAX_BYTES as u64),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

/// Publish verified original bytes. Existing files are verified before reuse.
pub fn publish(store: &Path, bytes: &[u8], declared_mime: &str) -> Result<ImageAsset> {
    let asset = validate(bytes, declared_mime)?;
    let staged = stage(store, bytes)?;
    if let Err(error) = publish_staged(store, &asset, &staged) {
        let _ = discard_staged(store, &staged);
        return Err(error);
    }
    Ok(asset)
}

/// Flush original bytes into the asset directory before recording a pending operation.
pub fn stage(store: &Path, bytes: &[u8]) -> Result<String> {
    let directory = asset_directory(store)?;
    let name = format!(
        ".staged-{}-{}",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    );
    let temp = directory.join(&name);
    let mut file = OpenOptions::new().write(true).create_new(true).open(&temp)?;
    let result = (|| -> Result<()> {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result?;
    sync_directory(&directory)?;
    Ok(name)
}

fn staged_path(directory: &Path, name: &str) -> Result<PathBuf> {
    let suffix = name.strip_prefix(".staged-").ok_or_else(|| anyhow::anyhow!("invalid_staged_image"))?;
    let (process, sequence) = suffix.split_once('-').ok_or_else(|| anyhow::anyhow!("invalid_staged_image"))?;
    if process.is_empty() || sequence.is_empty() || !process.bytes().all(|b| b.is_ascii_digit())
        || !sequence.bytes().all(|b| b.is_ascii_digit()) {
        bail!("invalid_staged_image");
    }
    Ok(directory.join(name))
}

/// Publish a journaled image, or verify the already-published file during recovery.
pub fn publish_staged(store: &Path, asset: &ImageAsset, name: &str) -> Result<()> {
    let directory = asset_directory(store)?;
    let staged = staged_path(&directory, name)?;
    let destination = directory.join(asset_filename(asset)?);
    if destination.exists() {
        verify_file(&destination, asset)?;
        match fs::remove_file(&staged) {
            Ok(()) => sync_directory(&directory)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => return Err(error.into()),
        }
        return Ok(());
    }
    verify_file(&staged, asset)?;
    fs::rename(&staged, &destination)?;
    sync_directory(&directory)?;
    verify_file(&destination, asset)?;
    Ok(())
}

/// Remove a staged image whose journal entry could not be written.
pub fn discard_staged(store: &Path, name: &str) -> Result<()> {
    let directory = asset_directory(store)?;
    let staged = staged_path(&directory, name)?;
    match fs::remove_file(staged) {
        Ok(()) => sync_directory(&directory),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(unix)]
fn sync_directory(directory: &Path) -> Result<()> {
    fs::File::open(directory)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_directory(_directory: &Path) -> Result<()> {
    // Windows does not expose directory sync through the standard File API.
    Ok(())
}

fn verify_file(path: &Path, asset: &ImageAsset) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path).context("asset_missing")?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > MAX_BYTES as u64 {
        bail!("asset_missing");
    }
    let bytes = fs::read(path).context("asset_missing")?;
    if blake3::hash(&bytes).to_hex().as_str() != asset.content_hash {
        bail!("asset_corrupt");
    }
    Ok(bytes)
}

/// Return original bytes only after validating their content hash.
pub fn read(store: &Path, asset: &ImageAsset) -> Result<Vec<u8>> {
    let directory = asset_directory(store)?;
    verify_file(&directory.join(asset_filename(asset)?), asset)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, ImageOutputFormat, RgbaImage};

    fn png() -> Vec<u8> {
        let image = DynamicImage::ImageRgba8(RgbaImage::new(2, 3));
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, ImageOutputFormat::Png).unwrap();
        bytes.into_inner()
    }

    #[test]
    fn preserves_deduplicates_and_detects_corruption() {
        let root = std::env::temp_dir().join(format!(
            "tovli-image-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let original = png();
        let first = publish(&root, &original, "image/png").unwrap();
        let second = publish(&root, &original, "image/png").unwrap();
        assert_eq!(first, second);
        assert_eq!((first.width, first.height), (2, 3));
        assert_eq!(read(&root, &first).unwrap(), original);
        let path = root.join("assets").join(asset_filename(&first).unwrap());
        fs::write(path, b"tampered").unwrap();
        assert!(read(&root, &first).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_invalid_and_mismatched_content() {
        assert!(validate(&png(), "image/jpeg").is_err());
        assert!(validate(b"not an image", "image/png").is_err());
        assert!(validate(&vec![0; MAX_BYTES + 1], "image/png").is_err());
    }

    #[test]
    fn staged_publication_is_recoverable_and_rejects_paths() {
        let root = std::env::temp_dir().join(format!(
            "tovli-image-stage-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let original = png();
        let asset = validate(&original, "image/png").unwrap();
        let stage = stage(&root, &original).unwrap();
        assert!(read(&root, &asset).is_err());
        publish_staged(&root, &asset, &stage).unwrap();
        assert_eq!(read(&root, &asset).unwrap(), original);
        // A crash after rename can replay without the staged file.
        publish_staged(&root, &asset, &stage).unwrap();
        assert!(publish_staged(&root, &asset, "../outside").is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
