use anyhow::{bail, Result};
use sha2::{Digest, Sha256};
use std::path::Path;
use tokio::io::AsyncReadExt;

pub const CHUNK_SIZE: usize = 64 * 1024;
pub const MAX_FILE_SIZE: u64 = 1024 * 1024 * 1024; // Matches the private mailbox's 1 GiB cap.

pub fn destination(directory: &Path, name: &str, index: u32) -> std::path::PathBuf {
    if index == 0 { return directory.join(name); }
    let path = Path::new(name);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or(name);
    match path.extension().and_then(|s| s.to_str()) {
        Some(extension) => directory.join(format!("{stem} ({index}).{extension}")),
        None => directory.join(format!("{stem} ({index})")),
    }
}

pub fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 240 || name.trim() != name
        || name.ends_with('.') || name.chars().any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
        || name == "." || name == ".." {
        bail!("文件名包含不支持的字符");
    }
    let stem = name.split('.').next().unwrap().to_uppercase();
    if ["CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9"].contains(&stem.as_str()) {
        bail!("此文件名是系统保留名称");
    }
    Ok(())
}

pub async fn hash_file(path: &Path) -> Result<(u64, String)> {
    let mut file = tokio::fs::File::open(path).await?;
    let metadata = file.metadata().await?;
    if !metadata.is_file() { bail!("请选择普通文件"); }
    let mut hash = Sha256::new();
    let mut buffer = vec![0u8; CHUNK_SIZE];
    let mut size = 0u64;
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 { break; }
        hash.update(&buffer[..read]);
        size = size.checked_add(read as u64).ok_or_else(|| anyhow::anyhow!("文件长度无效"))?;
        if size > metadata.len() { bail!("读取期间源文件已改变，请重新选择发送"); }
    }
    if size != metadata.len() { bail!("读取期间源文件已改变，请重新选择发送"); }
    Ok((size, hex::encode(hash.finalize())))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prevents_paths_and_windows_special_names() {
        for name in ["../secret", "a/b", "a\\b", "C:foo", "NUL.txt", "LPT1", "a.", "a\0", "..", ""] {
            assert!(validate_name(name).is_err(), "{name:?}");
        }
        assert!(validate_name("工作方案 v2.pdf").is_ok());
    }
}
