//! GitHub-only update transport. No updater IPC permissions are exposed: the
//! frontend cannot supply endpoints, public keys, headers or download URLs.
use std::time::Duration;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_updater::{Update, UpdaterExt};
use tokio::sync::Mutex;
use url::Url;
use crate::{AppState, Reply};

pub const ENDPOINT: &str = "https://raw.githubusercontent.com/Yumouqianxia/twinlink/updates/latest.json";
const RELEASE_PATH: &str = "/Yumouqianxia/twinlink/releases/download/";
#[derive(Default)]
pub struct Updates(pub Mutex<Option<Update>>);
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    current_version: String,
    version: Option<String>,
    notes: Option<String>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress { phase: &'static str, downloaded: u64, total: Option<u64> }

fn https(url: &Url) -> bool {
    url.scheme() == "https" && url.username().is_empty() && url.password().is_none()
        && url.port().is_none() && url.fragment().is_none()
}
fn asset(url: &Url) -> bool {
    https(url) && url.host_str() == Some("github.com")
        && url.path().starts_with(RELEASE_PATH) && url.path()[RELEASE_PATH.len()..].split('/').count() == 2
        && url.query().is_none()
}
// GitHub serves release bytes through its signed CDN redirect. No arbitrary
// host, subdomain wildcard or plain HTTP redirect is accepted.
fn redirect_allowed(url: &Url) -> bool {
    https(url) && match url.host_str() {
        Some("github.com") => asset(url),
        Some("release-assets.githubusercontent.com" | "objects.githubusercontent.com") => true,
        Some("raw.githubusercontent.com") => url.as_str() == ENDPOINT,
        _ => false,
    }
}

#[tauri::command]
pub async fn check_update(app: AppHandle, updates: State<'_, Updates>) -> Reply<UpdateInfo> {
    let mut pending = updates.0.try_lock().map_err(|_| "正在检查或安装更新，请稍候。".to_string())?;
    *pending = None;
    let updater = app.updater_builder()
        .endpoints(vec![Url::parse(ENDPOINT).expect("fixed HTTPS endpoint")]).map_err(|_| "更新源配置错误。")?
        .timeout(Duration::from_secs(60))
        .configure_client(|client| client.redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() < 5 && redirect_allowed(attempt.url()) { attempt.follow() }
            else { attempt.error("更新重定向超出 GitHub 范围") }
        })))
        .build().map_err(|_| "无法初始化更新组件。")?;
    let mut update = updater.check().await.map_err(|_| "无法从 GitHub 检查更新，请检查网络后重试。")?;
    if update.as_ref().is_some_and(|u| !asset(&u.download_url)) {
        return Err("更新包地址不属于本项目的 GitHub Releases，已拒绝。".into());
    }
    // Metadata is small; package downloads need a longer allowance on slow links.
    if let Some(package) = update.as_mut() {
        package.timeout = Some(Duration::from_secs(15 * 60));
    }
    let info = UpdateInfo {
        current_version: app.package_info().version.to_string(),
        version: update.as_ref().map(|u| u.version.clone()),
        notes: update.as_ref().and_then(|u| u.body.clone()),
    };
    *pending = update;
    Ok(info)
}

#[tauri::command]
pub async fn install_update(app: AppHandle, updates: State<'_, Updates>) -> Reply<()> {
    // Hold this lock across download and install; repeated clicks cannot start
    // multiple installers or replace the checked release while it is in use.
    let pending = updates.0.try_lock().map_err(|_| "更新正在进行，请稍候。".to_string())?;
    let update = pending.as_ref().ok_or("请先检查并确认可用的新版本。")?;
    if !asset(&update.download_url) { return Err("更新来源不受信任。".into()); }
    let mut downloaded = 0u64;
    let mut last = std::time::Instant::now();
    let bytes = update.download(|count, total| {
        downloaded = downloaded.saturating_add(count as u64);
        if last.elapsed() >= Duration::from_millis(150) {
            let _ = app.emit("update-progress", Progress { phase: "downloading", downloaded, total });
            last = std::time::Instant::now();
        }
    }, || {}).await.map_err(|_| "下载或签名校验失败，未安装更新。请稍后重试。")?;
    let _ = app.emit("update-progress", Progress { phase: "installing", downloaded, total: Some(downloaded) });
    // Tauri verifies the signature again on install. The version is bound to
    // the signed package (requireSignedVersion), and downgrades are disabled.
    update.install(bytes).map_err(|_| "安装未完成，请关闭其他同频窗口后重试。")?;
    // macOS needs an explicit restart. Persisted room/outbox/history are outside
    // the application bundle; graceful shutdown does not persist a pause.
    app.state::<AppState>().0.shutdown().await;
    app.restart();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_this_repository_can_supply_packages() {
        for good in ["https://github.com/Yumouqianxia/twinlink/releases/download/v0.0.2/TwinLink.app.tar.gz"] {
            assert!(asset(&Url::parse(good).unwrap()));
        }
        for bad in [
            "http://github.com/Yumouqianxia/twinlink/releases/download/v0.0.2/a",
            "https://github.com/other/twinlink/releases/download/v1/a",
            "https://github.com/Yumouqianxia/twinlink/releases/download/v1/a?url=evil",
            "https://github.com/Yumouqianxia/twinlink/releases/download/v1/a/b",
            "https://github.com.evil.test/Yumouqianxia/twinlink/releases/download/v1/a",
            "https://x@github.com/Yumouqianxia/twinlink/releases/download/v1/a",
        ] { assert!(!asset(&Url::parse(bad).unwrap()), "{bad}"); }
    }
    #[test]
    fn redirects_stay_on_exact_github_hosts() {
        assert!(redirect_allowed(&Url::parse(ENDPOINT).unwrap()));
        assert!(redirect_allowed(&Url::parse("https://release-assets.githubusercontent.com/github-production-release-asset/a?token=test").unwrap()));
        for bad in ["https://example.com/a", "http://objects.githubusercontent.com/a", "https://raw.githubusercontent.com/other/repo/main/latest.json", "https://github.com/login", "https://evil.githubusercontent.com/a"] {
            assert!(!redirect_allowed(&Url::parse(bad).unwrap()));
        }
    }
}
