//! Local startup diagnostics. Never writes profile, room or message contents.
use std::{fs, io::Write, path::{Path, PathBuf}, sync::{Mutex, OnceLock}};

static LOG: OnceLock<PathBuf> = OnceLock::new();
static WRITER: Mutex<()> = Mutex::new(());

pub fn initialize() {
    let preferred = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("TwinLink/logs"))
    } else {
        std::env::var_os("HOME").map(|p| PathBuf::from(p).join("Library/Logs/TwinLink"))
    };
    for directory in preferred.into_iter().chain(std::iter::once(std::env::temp_dir().join("TwinLink/logs"))) {
        if fs::create_dir_all(&directory).is_err() { continue; }
        let path = directory.join("startup.log");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&directory, fs::Permissions::from_mode(0o700));
        }
        if fs::OpenOptions::new().create(true).append(true).open(&path).is_err() { continue; }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
        }
        if fs::metadata(&path).is_ok_and(|m| m.len() > 512 * 1024) {
            let previous = directory.join("startup.previous.log");
            let _ = fs::remove_file(&previous);
            let _ = fs::rename(&path, previous);
        }
        let _ = LOG.set(path);
        break;
    }
    record("start", &format!("version={} os={} arch={}", env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH));
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Only location is recorded: panic payloads could contain user data.
        let detail = info.location().map(|p| format!("{}:{}:{}", p.file(), p.line(), p.column())).unwrap_or_default();
        record("panic", &detail);
        previous(info);
        if std::thread::current().name() == Some("main") {
            show_error("程序发生内部错误。请将启动日志提供给开发者以定位问题。");
        }
    }));
}

pub fn record(stage: &str, detail: &str) {
    let Some(path) = LOG.get() else { return; };
    let Ok(_guard) = WRITER.lock() else { return; };
    let time = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
    // Startup errors may include local paths, but never serialize application state.
    let detail: String = detail.chars().filter(|c| *c != '\r' && *c != '\n').take(2048).collect();
    if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{time} [{stage}] {detail}");
        let _ = file.flush();
    }
}

pub fn show_error(message: &str) {
    eprintln!("{message}");
    #[cfg(windows)]
    {
        // Automated isolated-runner tests can suppress a blocking error dialog.
        if std::env::var_os("TWINLINK_NO_ERROR_DIALOG").is_some() { return; }
        let log = LOG.get().map(|p| p.display().to_string()).unwrap_or_else(|| "日志目录不可写".into());
        let text: Vec<u16> = format!("同频未能启动。\n\n{message}\n\n启动日志：{log}\n\n不会清除聊天记录。")
            .encode_utf16().chain(Some(0)).collect();
        let title: Vec<u16> = "同频 · 启动失败".encode_utf16().chain(Some(0)).collect();
        #[link(name = "user32")]
        extern "system" {
            fn MessageBoxW(window: *mut std::ffi::c_void, text: *const u16, title: *const u16, flags: u32) -> i32;
        }
        // Valid NUL-terminated buffers remain alive for the synchronous dialog.
        unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), 0x10); }
    }
}

/// A redirected/unavailable Downloads folder must not prevent text chat startup.
/// The application-owned fallback remains outside the installation directory.
pub fn downloads(preferred: Option<PathBuf>, data: &Path) -> std::io::Result<PathBuf> {
    if let Some(path) = preferred {
        if path.is_absolute() && fs::create_dir_all(&path).is_ok() { return Ok(path); }
        record("downloads-fallback", "system Downloads directory unavailable; using application received directory");
    } else {
        record("downloads-fallback", "system Downloads path unavailable; using application received directory");
    }
    let fallback = data.join("received");
    fs::create_dir_all(&fallback)?;
    Ok(fallback)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn blocked_downloads_preserves_existing_file_and_uses_owned_directory() {
        let root = std::env::temp_dir().join(format!("twinlink-startup-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let blocked = root.join("Downloads");
        fs::write(&blocked, b"existing data").unwrap();
        let data = root.join("appdata");
        assert_eq!(downloads(Some(blocked.clone()), &data).unwrap(), data.join("received"));
        assert_eq!(fs::read(blocked).unwrap(), b"existing data");
        assert_eq!(downloads(None, &data).unwrap(), data.join("received"));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn available_unicode_downloads_is_kept() {
        let root = std::env::temp_dir().join(format!("同频-启动-{}", uuid::Uuid::new_v4()));
        let chosen = root.join("下载文件");
        assert_eq!(downloads(Some(chosen.clone()), &root.join("appdata")).unwrap(), chosen);
        assert!(!root.join("appdata/received").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
