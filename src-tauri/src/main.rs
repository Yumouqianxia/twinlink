#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Thin Tauri adapter. All pairing, persistence, networking and transfer logic
//! lives in `twinlink-core`; this file only maps IPC commands and app events.

use std::{path::PathBuf, sync::Arc};
use tauri::{Emitter, Manager, State};
use twinlink_core::{
    model::{HistoryPage, Message, Profile, Snapshot, Transfer},
    network::engine_path,
    Core,
};
use uuid::Uuid;

mod updates;
mod startup;

struct AppState(Arc<Core>);
type Reply<T> = std::result::Result<T, String>;

fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[tauri::command]
async fn snapshot(state: State<'_, AppState>) -> Reply<Snapshot> {
    static FIRST: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
    if FIRST.swap(false, std::sync::atomic::Ordering::Relaxed) {
        startup::record("frontend-ready", "first snapshot requested via IPC");
    }
    state.0.snapshot().await.map_err(error)
}

/// Import the coordinator's one-time server setup code. The code contains an
/// access token, so it is validated and stored by the core and never echoed back.
#[tauri::command]
async fn import_server(state: State<'_, AppState>, code: String) -> Reply<()> {
    state.0.import_server_code(&code).await.map_err(error)
}

/// Create a room as host and return the pairing key that must be shared.
#[tauri::command]
async fn create_room(state: State<'_, AppState>) -> Reply<String> {
    state.0.create_room().await.map_err(error)
}

/// Join a room from a pasted pairing key.
#[tauri::command]
async fn join_room(state: State<'_, AppState>, code: String) -> Reply<()> {
    state.0.join_room(&code).await.map_err(error)
}

/// Explicit "copy pairing key" action for the current room.
#[tauri::command]
async fn pairing_key(state: State<'_, AppState>) -> Reply<String> {
    state.0.pairing_key().await.map_err(error)
}

#[tauri::command]
async fn pause_peer(state: State<'_, AppState>) -> Reply<()> {
    state.0.pause().await.map_err(error)
}

#[tauri::command]
async fn resume_peer(state: State<'_, AppState>) -> Reply<()> {
    state.0.resume().await.map_err(error)
}

/// Clear the saved room only on explicit user action; chat history is kept.
#[tauri::command]
async fn forget_room(state: State<'_, AppState>) -> Reply<()> {
    state.0.forget_room().await.map_err(error)
}

#[tauri::command]
async fn save_profile(state: State<'_, AppState>, profile: Profile) -> Reply<()> {
    state.0.save_profile(profile).await.map_err(error)
}

#[tauri::command]
async fn export_data(state: State<'_, AppState>, path: String) -> Reply<()> {
    state.0.export_data(PathBuf::from(path)).await.map_err(error)
}

#[tauri::command]
async fn import_data(state: State<'_, AppState>, path: String) -> Reply<twinlink_core::backup::ImportReport> {
    state.0.import_data(PathBuf::from(path)).await.map_err(error)
}

#[tauri::command]
async fn send_message(state: State<'_, AppState>, text: String) -> Reply<Message> {
    state.0.send_message(text).await.map_err(error)
}

#[tauri::command]
async fn offer_file(state: State<'_, AppState>, path: String) -> Reply<Transfer> {
    state.0.offer_file(PathBuf::from(path)).await.map_err(error)
}

#[tauri::command]
async fn accept_file(state: State<'_, AppState>, id: String) -> Reply<()> {
    state
        .0
        .accept_file(Uuid::parse_str(&id).map_err(error)?)
        .await
        .map_err(error)
}

#[tauri::command]
async fn reject_file(state: State<'_, AppState>, id: String) -> Reply<()> {
    state
        .0
        .reject_file(Uuid::parse_str(&id).map_err(error)?)
        .await
        .map_err(error)
}

#[tauri::command]
async fn cancel_transfer(state: State<'_, AppState>, id: String) -> Reply<()> {
    state
        .0
        .cancel_transfer(Uuid::parse_str(&id).map_err(error)?)
        .await
        .map_err(error)
}

/// One page of stored history, strictly older than `before_id` (null = newest).
#[tauri::command]
async fn message_history(
    state: State<'_, AppState>,
    before_id: Option<String>,
) -> Reply<HistoryPage> {
    state.0.message_history(before_id).await.map_err(error)
}

fn main() {
    startup::initialize();
    startup::record("webview-runtime", &tauri::webview_version().unwrap_or_else(|_| "unavailable".into()));
    let application = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(updates::Updates::default())
        .setup(|app| {
            let initialized: Result<(), Box<dyn std::error::Error>> = (|| {
            startup::record("setup", "resolving application paths");
            let paths = app.path();
            let data = paths.app_data_dir()?;
            let downloads = startup::downloads(paths.download_dir().ok().map(|p| p.join("TwinLink")), &data)?;
            let mut engine = engine_path(&paths.resource_dir()?);
            // In `tauri dev` the bundled resource path does not exist yet, so the
            // source-tree engine directory is used instead.
            if cfg!(debug_assertions) || !engine.exists() {
                let dev_engine = engine_path(&PathBuf::from(env!("CARGO_MANIFEST_DIR")));
                if dev_engine.exists() {
                    engine = dev_engine;
                }
            }
            startup::record("core-open", "opening local database and saved state");
            let core = Core::open(data, downloads, engine)?;
            startup::record("core-ready", "local storage initialized");
            let mut events = core.subscribe();
            let handle = app.handle().clone();
            let resync = core.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    match events.recv().await {
                        Ok(event) => {
                            let _ = handle.emit("app-event", event);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            if let Ok(snapshot) = resync.snapshot().await {
                                let _ = handle.emit("app-resync", snapshot);
                            }
                        }
                        Err(_) => break,
                    }
                }
            });
            // One supervisor task owns the EasyTier child for the app's lifetime.
            let runner = core.clone();
            tauri::async_runtime::spawn(async move { runner.run().await });
            app.manage(AppState(core));
            Ok(())
            })();
            if let Err(error) = initialized {
                startup::record("setup-error", &error.to_string());
                startup::show_error(&error.to_string());
                // Tauri otherwise turns a setup error into an unhandled panic.
                std::process::exit(1);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            snapshot,
            import_server,
            create_room,
            join_room,
            pairing_key,
            pause_peer,
            resume_peer,
            forget_room,
            save_profile,
            export_data,
            import_data,
            send_message,
            offer_file,
            accept_file,
            reject_file,
            cancel_transfer,
            message_history,
            updates::check_update,
            updates::install_update
        ])
        .build(tauri::generate_context!());
    let application = match application {
        Ok(application) => application,
        Err(error) => {
            startup::record("startup-error", &error.to_string());
            startup::show_error(&error.to_string());
            std::process::exit(1);
        }
    };
    application.run(|app, event| {
            if let tauri::RunEvent::Ready = event { startup::record("ready", "application event loop ready"); }
            if let tauri::RunEvent::Exit = event {
                startup::record("exit", "application event loop exiting");
                if let Some(state) = app.try_state::<AppState>() {
                    let core = state.0.clone();
                    tauri::async_runtime::block_on(async move { core.shutdown().await });
                }
            }
        });
}
