//! Local, secret-bearing application state.
//!
//! `app-state.json` holds the configured server (including its access token) and
//! the selected room (including the room seed). It is written atomically and with
//! `0600` permissions on Unix. Nothing here is ever sent to the frontend except
//! through the explicit pairing-key command.

use crate::pairing::{ServerConfig, StoredRoom};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Default)]
pub struct SavedState {
    pub server: Option<ServerConfig>,
    pub room: Option<StoredRoom>,
    /// Explicit manual disconnect. Suppresses automatic reconnect until resumed.
    pub paused: bool,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FileState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    server: Option<ServerConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    room: Option<StoredRoom>,
    #[serde(default)]
    paused: bool,
}

pub struct StateFile {
    path: PathBuf,
}

impl StateFile {
    pub fn new(dir: &Path) -> Self {
        Self { path: dir.join("app-state.json") }
    }

    pub fn load(&self) -> Result<SavedState> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(SavedState::default())
            }
            Err(error) => return Err(error).context("无法读取本地设置"),
        };
        match serde_json::from_slice::<FileState>(&bytes) {
            Ok(mut file) => {
                // A hand-edited or partially corrupted file must not make the app
                // spawn an engine with unusable parameters. Drop only what fails.
                if file.server.as_ref().is_some_and(|server| server.validate().is_err()) {
                    file.server = None;
                }
                if file.room.as_ref().is_some_and(|room| room.validate().is_err()) {
                    file.room = None;
                }
                Ok(SavedState { server: file.server, room: file.room, paused: file.paused })
            }
            Err(_) => {
                // Keep the unreadable file for later inspection instead of
                // silently overwriting it, then start from defaults.
                let _ = fs::rename(&self.path, self.path.with_extension("json.corrupt"));
                Ok(SavedState::default())
            }
        }
    }

    pub fn save(&self, state: &SavedState) -> Result<()> {
        let file = FileState {
            server: state.server.clone(),
            room: state.room.clone(),
            paused: state.paused,
        };
        let bytes = serde_json::to_vec_pretty(&file)?;
        write_private(&self.path, &bytes)
    }
}

/// Write a file only the current user can read, replacing it atomically.
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().context("目标文件没有父目录")?;
    fs::create_dir_all(dir)?;
    restrict_dir(dir)?;
    // Exclusive, unpredictable temporary file: an old fixed-name symlink must
    // never redirect a credential write into another local file.
    let mut tmp = tempfile::Builder::new()
        .prefix(".twinlink-state-")
        .tempfile_in(dir)
        .context("无法写入本地设置")?;
    restrict_file(tmp.path())?;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|error| error.error).context("无法保存本地设置")?;
    #[cfg(unix)]
    fs::File::open(dir)?.sync_all()?;
    Ok(())
}

pub fn restrict_dir(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

pub fn restrict_file(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pairing::test_server;

    #[test]
    fn round_trips_server_and_room_and_restricts_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let file = StateFile::new(dir.path());
        let state = SavedState {
            server: Some(test_server("twinlink-state-prefix")),
            room: Some(StoredRoom::create(test_server("twinlink-state-prefix"), "host").unwrap()),
            paused: true,
        };
        file.save(&state).unwrap();
        let loaded = file.load().unwrap();
        assert_eq!(loaded.server.as_ref().unwrap().relay_url, "tcp://45.76.70.254:11010");
        assert!(loaded.paused);
        assert_eq!(loaded.room.unwrap().role, "host");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dir.path().join("app-state.json")).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "secret state file must not be world readable");
            let dir_mode = fs::metadata(dir.path()).unwrap().permissions().mode();
            assert_eq!(dir_mode & 0o777, 0o700);
        }
    }

    #[test]
    fn corrupt_file_is_moved_aside_instead_of_crashing() {
        let dir = tempfile::tempdir().unwrap();
        let file = StateFile::new(dir.path());
        fs::write(dir.path().join("app-state.json"), b"{not json").unwrap();
        let loaded = file.load().unwrap();
        assert!(loaded.server.is_none() && loaded.room.is_none());
        assert!(dir.path().join("app-state.json.corrupt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn stale_fixed_temp_symlink_cannot_redirect_private_write() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let unrelated = dir.path().join("unrelated");
        fs::write(&unrelated, b"keep existing bytes").unwrap();
        let target = dir.path().join("app-state.json");
        symlink(&unrelated, target.with_extension("tmp")).unwrap();
        write_private(&target, b"private state").unwrap();
        assert_eq!(fs::read(&unrelated).unwrap(), b"keep existing bytes");
        assert_eq!(fs::read(target).unwrap(), b"private state");
    }

    #[test]
    fn invalid_room_is_dropped_but_valid_server_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let file = StateFile::new(dir.path());
        let mut state = SavedState {
            server: Some(test_server("twinlink-state-prefix")),
            ..SavedState::default()
        };
        let mut room = StoredRoom::create(test_server("twinlink-state-prefix"), "host").unwrap();
        room.seed = "NOT-A-SEED".into();
        state.room = Some(room);
        file.save(&state).unwrap();
        let loaded = file.load().unwrap();
        assert!(loaded.room.is_none());
        assert!(loaded.server.is_some());
    }
}
