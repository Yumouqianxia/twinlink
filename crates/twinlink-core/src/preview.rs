//! In-app image preview authorization.
//!
//! The WebView never hands a filesystem path to the backend and never receives
//! image bytes over IPC. It asks the custom `twinlink-preview` URI scheme for
//! the record id only; this module decides, per request, whether that id names a
//! locally present, safe raster image belonging to the room currently shown.
//!
//! Deny-by-default rules:
//! * only the current room's history is eligible;
//! * outgoing records resolve to their persisted `source_path`;
//! * incoming records must be `completed` and resolve to the published
//!   `final_path`, never the in-progress `.part` scratch file;
//! * incoming paths must be in the directory the file was actually accepted
//!   into; only regular files, never symbolic links, are eligible;
//! * only PNG/JPEG/GIF/WebP magic is accepted and the declared dimensions must
//!   be sane, so a mislabelled SVG/HTML is rejected even if it is renamed;
//! * at most [`MAX_PREVIEW_BYTES`] is read, independent of the send size limit.

use crate::{model::Transfer, Core};
use anyhow::{bail, Context, Result};
use std::io::Read;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Preview reads are capped far below the transfer limit: the point is an
/// on-screen thumbnail, not a second copy of the file.
pub const MAX_PREVIEW_BYTES: u64 = 20 * 1024 * 1024;
const MAX_DIMENSION: u32 = 16_384;
const MAX_PIXELS: u64 = 40_000_000;

pub struct PreviewImage {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
}

/// Resolve one preview request. `id` is an opaque transfer UUID; no path,
/// extension or query parameter from the request is trusted.
pub async fn preview_image(core: &Core, room: &str, id: &str) -> Result<PreviewImage> {
    let parsed = Uuid::parse_str(id).context("预览标识无效")?;
    anyhow::ensure!(parsed.to_string() == id, "预览标识无效");

    let _operation = core.operations.lock().await;
    anyhow::ensure!(core.inner.lock().await.history_room == room, "会话已改变");
    let transfer = core
        .store
        .transfer(room, id)?
        .context("当前会话中没有这个文件")?;

    let path = resolve_local_file(core, room, id, &transfer)?;
    let bytes = tokio::task::spawn_blocking(move || read_capped(&path)).await??;
    let (mime, width, height) = sniff_image(&bytes).context("这个文件不是可预览的图片")?;
    anyhow::ensure!(
        width > 0
            && height > 0
            && width <= MAX_DIMENSION
            && height <= MAX_DIMENSION
            && u64::from(width) * u64::from(height) <= MAX_PIXELS,
        "图片尺寸过大，暂不支持在应用内预览"
    );
    Ok(PreviewImage { bytes, mime })
}

/// Only files this device already holds may be previewed: never a pending
/// incoming offer, never a file that is still downloading.
fn resolve_local_file(core: &Core, room: &str, id: &str, transfer: &Transfer) -> Result<PathBuf> {
    let path = if transfer.outgoing {
        let upload = core.store.upload(room, id)?.context("本地文件记录缺失")?;
        PathBuf::from(upload.source_path)
    } else {
        anyhow::ensure!(transfer.status == "completed", "文件尚未接收完成");
        let download = core.store.download(room, id)?.context("本地文件记录缺失")?;
        anyhow::ensure!(download.status == "completed", "文件尚未接收完成");
        let published = download
            .final_path
            .as_deref()
            .context("文件尚未写入接收目录")?;
        let published = PathBuf::from(published);
        // Bind the read to the directory the user actually accepted into. The
        // stored path alone is not trusted if it drifted out of that directory.
        let target = download.target_dir.as_deref().context("接收目录缺失")?;
        let target = Path::new(target).canonicalize().context("接收目录不可用")?;
        anyhow::ensure!(
            !std::fs::symlink_metadata(&published)?
                .file_type()
                .is_symlink(),
            "不支持预览符号链接"
        );
        let resolved = published.canonicalize().context("文件不可用")?;
        anyhow::ensure!(
            resolved.parent() == Some(target.as_path()),
            "文件位置与接收目录不一致"
        );
        anyhow::ensure!(
            !published
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("part")),
            "文件尚未接收完成"
        );
        resolved
    };

    let meta = std::fs::symlink_metadata(&path).context("文件不可用")?;
    anyhow::ensure!(!meta.file_type().is_symlink(), "不支持预览符号链接");
    anyhow::ensure!(meta.file_type().is_file(), "只能预览文件，不能预览目录");
    let resolved = path.canonicalize().context("文件不可用")?;
    Ok(resolved)
}

fn read_capped(path: &Path) -> Result<Vec<u8>> {
    let meta = std::fs::metadata(path)?;
    if meta.len() > MAX_PREVIEW_BYTES {
        bail!(
            "图片超过 {} MiB，暂不支持在应用内预览",
            MAX_PREVIEW_BYTES / 1024 / 1024
        );
    }
    let file = std::fs::File::open(path)?;
    let mut bytes = Vec::with_capacity(meta.len().min(MAX_PREVIEW_BYTES) as usize);
    // `take` hard-stops the read even if the file grows between `metadata` and
    // `open`, so a preview can never balloon the process.
    file.take(MAX_PREVIEW_BYTES + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() as u64 <= MAX_PREVIEW_BYTES,
        "图片过大，暂不支持在应用内预览"
    );
    Ok(bytes)
}

/// Raster-only sniffing by magic bytes; returns `(mime, width, height)`.
/// Extensions and client-supplied content types are never consulted.
fn sniff_image(bytes: &[u8]) -> Option<(&'static str, u32, u32)> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return png_size(bytes).map(|(w, h)| ("image/png", w, h));
    }
    if bytes.starts_with(b"\xff\xd8\xff") {
        return jpeg_size(bytes).map(|(w, h)| ("image/jpeg", w, h));
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return gif_size(bytes).map(|(w, h)| ("image/gif", w, h));
    }
    if bytes.len() >= 30 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return webp_size(bytes).map(|(w, h)| ("image/webp", w, h));
    }
    None
}

fn be32(bytes: &[u8], at: usize) -> Option<u32> {
    bytes
        .get(at..at + 4)
        .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

fn be16(bytes: &[u8], at: usize) -> Option<u32> {
    bytes
        .get(at..at + 2)
        .map(|b| u32::from(u16::from_be_bytes([b[0], b[1]])))
}

fn le16(bytes: &[u8], at: usize) -> Option<u32> {
    bytes
        .get(at..at + 2)
        .map(|b| u32::from(u16::from_le_bytes([b[0], b[1]])))
}

fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.get(12..16)? != b"IHDR" {
        return None;
    }
    Some((be32(bytes, 16)?, be32(bytes, 20)?))
}

fn gif_size(bytes: &[u8]) -> Option<(u32, u32)> {
    Some((le16(bytes, 6)?, le16(bytes, 8)?))
}

fn jpeg_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut pos = 2usize;
    while pos + 4 <= bytes.len() {
        if bytes[pos] != 0xFF {
            pos += 1;
            continue;
        }
        let marker = bytes[pos + 1];
        // Standalone markers carry no length payload.
        if (0xD0..=0xD9).contains(&marker) || marker == 0x01 || marker == 0xFF {
            pos += 2;
            continue;
        }
        let length = be16(bytes, pos + 2)? as usize;
        if length < 2 {
            return None;
        }
        let is_sof = matches!(marker, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF);
        if is_sof {
            let height = be16(bytes, pos + 5)?;
            let width = be16(bytes, pos + 7)?;
            return Some((width, height));
        }
        pos += 2 + length;
    }
    None
}

fn webp_size(bytes: &[u8]) -> Option<(u32, u32)> {
    match bytes.get(12..16)? {
        b"VP8 " => {
            // Lossy: 3-byte frame tag, then the 0x9d 0x01 0x2a sync code.
            if bytes.get(23..26)? != [0x9d, 0x01, 0x2a] {
                return None;
            }
            Some((le16(bytes, 26)? & 0x3FFF, le16(bytes, 28)? & 0x3FFF))
        }
        b"VP8L" => {
            if bytes.get(20)? != &0x2f {
                return None;
            }
            let bits = u32::from_le_bytes([
                *bytes.get(21)?,
                *bytes.get(22)?,
                *bytes.get(23)?,
                *bytes.get(24)?,
            ]);
            Some(((bits & 0x3FFF) + 1, ((bits >> 14) & 0x3FFF) + 1))
        }
        b"VP8X" => {
            let read24 = |at: usize| {
                bytes
                    .get(at..at + 3)
                    .map(|b| u32::from(b[0]) | (u32::from(b[1]) << 8) | (u32::from(b[2]) << 16))
            };
            Some((read24(24)? + 1, read24(27)? + 1))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_images_and_mislabelled_markup() {
        assert!(sniff_image(b"<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>").is_none());
        assert!(sniff_image(b"<!doctype html><html></html>").is_none());
        assert!(sniff_image(b"GIF89a").is_none());
        assert!(sniff_image(b"").is_none());
    }

    #[test]
    fn reads_png_dimensions() {
        let mut png = Vec::from(*b"\x89PNG\r\n\x1a\n");
        png.extend_from_slice(&13u32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&640u32.to_be_bytes());
        png.extend_from_slice(&480u32.to_be_bytes());
        assert_eq!(sniff_image(&png), Some(("image/png", 640, 480)));
    }

    #[test]
    fn reads_gif_webp_and_jpeg_dimensions() {
        let mut gif = Vec::from(*b"GIF89a");
        gif.extend_from_slice(&320u16.to_le_bytes());
        gif.extend_from_slice(&200u16.to_le_bytes());
        assert_eq!(sniff_image(&gif), Some(("image/gif", 320, 200)));

        let mut webp = Vec::from(*b"RIFF");
        webp.extend_from_slice(&0u32.to_le_bytes());
        webp.extend_from_slice(b"WEBP");
        webp.extend_from_slice(b"VP8X");
        webp.extend_from_slice(&10u32.to_le_bytes());
        webp.push(0);
        webp.extend_from_slice(&[0, 0, 0]);
        webp.extend_from_slice(&[0x7F, 0x02, 0x00]); // 640 - 1
        webp.extend_from_slice(&[0x2F, 0x01, 0x00]); // 304 - 1
        assert_eq!(sniff_image(&webp), Some(("image/webp", 640, 304)));

        let mut jpeg = Vec::from(*b"\xff\xd8\xff\xe0");
        jpeg.extend_from_slice(&16u16.to_be_bytes());
        jpeg.extend_from_slice(b"JFIF\0\x01\x01\x00\x00\x01\x00\x01\x00\x00");
        jpeg.extend_from_slice(b"\xff\xc0");
        jpeg.extend_from_slice(&17u16.to_be_bytes());
        jpeg.push(8);
        jpeg.extend_from_slice(&1080u16.to_be_bytes());
        jpeg.extend_from_slice(&1920u16.to_be_bytes());
        jpeg.extend_from_slice(&[1, 1, 0x11, 0]);
        assert_eq!(sniff_image(&jpeg), Some(("image/jpeg", 1920, 1080)));
    }
}
