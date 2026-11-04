pub mod crypto;
pub mod files;
pub mod mailbox;
mod mailbox_service;
mod direct_files;
pub mod backup;
pub mod model;
pub mod preview;
pub mod network;
pub mod pairing;
pub mod state;
mod storage;
mod wire;

use anyhow::{bail, Context, Result};
use model::{
    now, AppEvent, Message, Profile, ReconnectStatus, RoomInfo, Snapshot, Transfer,
};
use pairing::{RoomSpec, ServerConfig, StoredRoom};
use state::{SavedState, StateFile};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex as StdMutex},
    time::{Duration, Instant},
};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{broadcast, mpsc, watch, Mutex},
    time::{sleep, timeout},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use wire::{Control, Frame};

struct Inner {
    profile: Profile,
    /// Room identity used to key persisted chat. Kept after "forget room" so the
    /// previous conversation stays readable instead of being wiped.
    history_room: String,
    connection: String,
    peer_name: Option<String>,
    sender: Option<mpsc::Sender<Frame>>,
    room: Option<StoredRoom>,
    paused: bool,
    reconnect: Option<ReconnectStatus>,
    mailbox_state: String,
    mailbox_error: Option<String>,
    mailbox_pending: usize,
    /// True while the supervisor holds a live EasyTier child. `shutdown` waits
    /// for this to clear so app exit never orphans a child process.
    attempt_active: bool,
    /// Monotonic identity of the active room configuration. Every mailbox
    /// request captures it so a late reply can never mutate a newer session.
    generation: u64,
}

/// What the supervisor should currently be doing.
#[derive(Clone)]
struct Desired {
    room: Option<StoredRoom>,
    paused: bool,
    generation: u64,
}

/// Tunable retry behaviour; tests use much shorter values.
#[derive(Clone)]
pub struct Policy {
    pub connect_timeout: Duration,
    pub handshake_timeout: Duration,
    pub retry_base: Duration,
    pub retry_cap: Duration,
    pub max_failures: u32,
    /// Randomised halving of the delay avoids lock-step retries between peers.
    pub jitter: bool,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(90),
            handshake_timeout: Duration::from_secs(8),
            retry_base: Duration::from_secs(1),
            retry_cap: Duration::from_secs(30),
            max_failures: 0, // Unlimited transient retries; explicit pause stops them.
            jitter: true,
        }
    }
}

impl Policy {
    /// Delay before retry number `failures + 1` (so `failures` is at least 1).
    pub fn delay(&self, failures: u32) -> Duration {
        let exponent = failures.saturating_sub(1).min(16);
        let factor = 1u64 << exponent;
        let base = (self.retry_base.as_millis() as u64)
            .saturating_mul(factor)
            .min(self.retry_cap.as_millis() as u64);
        if !self.jitter || base < 4 {
            return Duration::from_millis(base);
        }
        let half = base / 2;
        Duration::from_millis(half + rand::random::<u64>() % half)
    }
}

pub struct Core {
    inner: Mutex<Inner>,
    /// Serialises user actions that replace the selected room or server.
    operations: Mutex<()>,
    store: storage::Store,
    events: broadcast::Sender<AppEvent>,
    engine: PathBuf,
    data_dir: PathBuf,
    state_file: StateFile,
    /// Persisted copy; guarded separately from `inner` to avoid lock nesting.
    saved: StdMutex<SavedState>,
    desired: watch::Sender<Desired>,
    /// Cancelled once at app exit; every attempt selects on it so children die.
    shutdown: CancellationToken,
    policy: Policy,
    mailbox_wake: tokio::sync::Notify,
    direct: Mutex<direct_files::DirectState>,
    /// Test-only transport override so the durable mailbox can run against an
    /// in-process server without real TLS. Production always builds its own.
    #[cfg(test)]
    transport: StdMutex<Option<Arc<dyn crate::mailbox::Transport>>>,
}

enum Attempt {
    /// Desired state changed while connecting; the child has been killed.
    Changed,
    /// A real session ran and then ended (peer left, socket error, timeout).
    SessionEnded,
    /// We never reached a session (engine missing/exited, port busy, timeout).
    Failed,
    /// The app is exiting; the supervisor must stop and reap its child.
    Shutdown,
}

impl Core {
    pub fn open(data_dir: PathBuf, download_dir: PathBuf, engine: PathBuf) -> Result<Arc<Self>> {
        Self::open_with_policy(data_dir, download_dir, engine, Policy::default())
    }

    pub fn open_with_policy(
        data_dir: PathBuf,
        download_dir: PathBuf,
        engine: PathBuf,
        policy: Policy,
    ) -> Result<Arc<Self>> {
        std::fs::create_dir_all(&data_dir)?;
        state::restrict_dir(&data_dir)?;
        let state_file = StateFile::new(&data_dir);
        // A corrupt or unreadable state file must not prevent the app from
        // starting; load() quarantines it and returns defaults.
        let saved = state_file.load().unwrap_or_default();
        let store = storage::Store::open(&data_dir.join("twinlink.db"))?;
        store.recover_all_direct()?;
        let profile = store.profile(Profile {
            display_name: default_name(),
            download_dir: download_dir.to_string_lossy().into_owned(),
            auto_accept_files: false,
        })?;
        let history_room = saved
            .room
            .as_ref()
            .and_then(|room| room.spec().ok())
            .map(|spec| spec.room_id)
            .or(store.history_room()?)
            .unwrap_or_default();
        let desired = Desired { room: saved.room.clone(), paused: saved.paused, generation: 0 };
        let (events, _) = broadcast::channel(256);
        let (desired_tx, _) = watch::channel(desired);
        Ok(Arc::new(Self {
            inner: Mutex::new(Inner {
                profile,
                history_room,
                connection: "disconnected".into(),
                peer_name: None,
                sender: None,
                room: saved.room.clone(),
                paused: saved.paused,
                reconnect: None,
                mailbox_state: "notConfigured".into(),
                mailbox_error: None,
                mailbox_pending: 0,
                attempt_active: false,
                generation: 0,
            }),
            operations: Mutex::new(()),
            store,
            events,
            engine,
            data_dir,
            state_file,
            saved: StdMutex::new(saved),
            desired: desired_tx,
            shutdown: CancellationToken::new(),
            policy,
            mailbox_wake: tokio::sync::Notify::new(),
            direct: Mutex::new(direct_files::DirectState::default()),
            #[cfg(test)]
            transport: StdMutex::new(None),
        }))
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AppEvent> {
        self.events.subscribe()
    }

    fn emit(&self, event: AppEvent) {
        let _ = self.events.send(event);
    }

    /// UI events are room-scoped by construction: a late mailbox reply from an
    /// old room must never paint a message or transfer into the new room.
    pub(crate) async fn emit_in_room(&self, room_id: &str, event: AppEvent) {
        let inner = self.inner.lock().await;
        if inner.history_room == room_id {
            self.emit(event);
        }
    }

    /// Verify the full room configuration, not just the room id, is still the
    /// session that started this request.
    pub(crate) async fn generation_is_current(&self, room_id: &str, generation: u64) -> bool {
        if self.shutdown.is_cancelled() {
            return false;
        }
        let inner = self.inner.lock().await;
        inner.generation == generation
            && inner.history_room == room_id
            && inner
                .room
                .as_ref()
                .and_then(|room| room.spec().ok())
                .map(|spec| spec.room_id == room_id)
                .unwrap_or(false)
    }

    fn emit_error(&self, message: String) {
        self.emit(AppEvent::Error { message });
    }

    async fn state_event(&self) -> AppEvent {
        let inner = self.inner.lock().await;
        state_event(&inner)
    }

    async fn emit_state(&self) {
        let event = self.state_event().await;
        self.emit(event);
    }

    pub async fn snapshot(&self) -> Result<Snapshot> {
        // Never hold `saved` and `inner` at the same time: user actions take
        // them in the opposite order and would otherwise deadlock.
        let saved_server = self.saved.lock().unwrap().server.clone();
        let inner = self.inner.lock().await;
        let server = inner
            .room
            .as_ref()
            .map(|room| &room.server)
            .or(saved_server.as_ref())
            .map(ServerConfig::summary);
        let room = inner
            .room
            .as_ref()
            .and_then(|room| room.spec().ok())
            .map(|spec| RoomInfo::from_spec(&spec));
        let transfers = self.store.transfers(&inner.history_room)?;
        Ok(Snapshot {
            profile: inner.profile.clone(),
            connection: inner.connection.clone(),
            peer_name: inner.peer_name.clone(),
            messages: self.store.messages(&inner.history_room)?,
            transfers,
            engine_available: self.engine.is_file(),
            preview: false,
            paused: inner.paused,
            reconnect: inner.reconnect.clone(),
            server,
            room,
            // The engine enforces --p2p-only; an authenticated live channel
            // cannot silently carry file payloads through the relay.
            route: if inner.sender.is_some() { "direct" } else { "unknown" }.into(),
            mailbox: model::MailboxInfo {
                state: inner.mailbox_state.clone(),
                message: inner.mailbox_error.clone(),
                pending: inner.mailbox_pending,
            },
        })
    }

    pub async fn message_history(&self, before_id: Option<String>) -> Result<model::HistoryPage> {
        if let Some(id) = before_id.as_deref() {
            let parsed = Uuid::parse_str(id).context("历史记录位置无效")?;
            anyhow::ensure!(parsed.to_string() == id, "历史记录位置无效");
        }
        let room = self.inner.lock().await.history_room.clone();
        self.store.message_page(&room, before_id.as_deref())
    }

    pub async fn save_profile(&self, mut profile: Profile) -> Result<()> {
        profile.display_name = profile.display_name.trim().into();
        if profile.display_name.is_empty()
            || profile.display_name.chars().count() > 64
            || profile.display_name.chars().any(char::is_control)
        {
            bail!("设备名称应为 1–64 个可显示字符");
        }
        let path = Path::new(&profile.download_dir);
        if !path.is_absolute() || !path.is_dir() {
            bail!("请选择已存在的下载目录");
        }
        self.store.save_profile(&profile)?;
        self.inner.lock().await.profile = profile;
        Ok(())
    }

    /* ---------- pairing ---------- */

    /// Import a server setup code. The code contains the access token, so it is
    /// never echoed back or stored anywhere but the private state file.
    pub async fn import_server_code(&self, code: &str) -> Result<()> {
        let server = pairing::parse_server_code(code)?;
        let _operation = self.operations.lock().await;
        let mut saved = self.saved.lock().unwrap();
        saved.server = Some(server.clone());
        self.state_file.save(&saved)?;
        drop(saved);
        self.emit(AppEvent::Server { server: Some(server.summary()) });
        Ok(())
    }

    pub async fn create_room(&self) -> Result<String> {
        let _operation = self.operations.lock().await;
        let server = self
            .saved
            .lock()
            .unwrap()
            .server
            .clone()
            .context("请先在设置中导入服务器设置码")?;
        let room = StoredRoom::create(server, "host")?;
        let code = room.invite_code()?;
        self.install_room(room).await?;
        Ok(code)
    }

    pub async fn join_room(&self, code: &str) -> Result<()> {
        // Parse and validate before taking any lock or touching the current room.
        let room = StoredRoom::from_invite(code)?;
        let _operation = self.operations.lock().await;
        self.install_room(room).await
    }

    /// Explicit pairing-key export. The only command that returns the seed-bearing
    /// invite; it is never part of a snapshot or an event.
    pub async fn pairing_key(&self) -> Result<String> {
        let room = self
            .inner
            .lock()
            .await
            .room
            .clone()
            .context("当前没有房间")?;
        room.invite_code()
    }

    async fn install_room(&self, room: StoredRoom) -> Result<()> {
        let spec = room.spec()?;
        let history_id = spec.room_id.clone();
        {
            let mut inner = self.inner.lock().await;
            if inner.room.as_ref() != Some(&room) && inner.profile.auto_accept_files {
                let mut profile = inner.profile.clone();
                profile.auto_accept_files = false;
                self.store.save_profile(&profile)?;
                inner.profile = profile;
            }
        }
        {
            let mut saved = self.saved.lock().unwrap();
            saved.room = Some(room.clone());
            // Selecting a room is an explicit intent to connect.
            saved.paused = false;
            self.state_file.save(&saved).context("无法保存房间设置")?;
        }
        let generation = {
            let mut inner = self.inner.lock().await;
            inner.room = Some(room.clone());
            inner.history_room = spec.room_id;
            inner.paused = false;
            inner.reconnect = None;
            inner.connection = "connecting".into();
            inner.peer_name = None;
            inner.sender = None;
            inner.generation = inner.generation.wrapping_add(1);
            inner.generation
        };
        // Notifies the supervisor, which kills any child from the old room.
        self.desired.send_replace(Desired { room: Some(room), paused: false, generation });
        if self.store.remember_history_room(&history_id).is_err() {
            self.emit_error("房间已保存，但最近会话索引暂时无法写入".into());
        }
        self.emit_state().await;
        Ok(())
    }

    /// Explicit manual disconnect: persisted, stops retrying until resumed.
    pub async fn pause(&self) -> Result<()> {
        let _operation = self.operations.lock().await;
        let save_result = {
            let mut saved = self.saved.lock().unwrap();
            saved.paused = true;
            self.state_file.save(&saved)
        };
        let generation = {
            let mut inner = self.inner.lock().await;
            inner.paused = true;
            inner.reconnect = None;
            inner.connection = "disconnected".into();
            inner.peer_name = None;
            inner.sender = None;
            inner.generation = inner.generation.wrapping_add(1);
            inner.generation
        };
        // Copy the room out before touching the watch value: holding a `borrow`
        // guard and calling `send_replace` in the same expression self-deadlocks.
        let room = self.desired.borrow().room.clone();
        self.desired.send_replace(Desired { room, paused: true, generation });
        self.emit_state().await;
        save_result?;
        Ok(())
    }

    pub async fn resume(&self) -> Result<()> {
        let _operation = self.operations.lock().await;
        let room = self
            .inner
            .lock()
            .await
            .room
            .clone()
            .context("还没有保存的房间，请创建或加入房间")?;
        {
            let mut saved = self.saved.lock().unwrap();
            saved.paused = false;
            self.state_file.save(&saved)?;
        }
        let generation = {
            let mut inner = self.inner.lock().await;
            inner.paused = false;
            inner.reconnect = None;
            inner.generation = inner.generation.wrapping_add(1);
            inner.generation
        };
        // Always notifies, even when paused was already false, so a supervisor
        // that stopped after max_failures starts a fresh attempt cycle.
        self.desired.send_replace(Desired { room: Some(room), paused: false, generation });
        self.emit_state().await;
        Ok(())
    }

    /// Clear the saved room but keep its chat history on disk. Requires explicit
    /// user action; nothing else calls this.
    pub async fn forget_room(&self) -> Result<()> {
        let _operation = self.operations.lock().await;
        {
            let mut saved = self.saved.lock().unwrap();
            saved.room = None;
            saved.paused = false;
            self.state_file.save(&saved)?;
        }
        let generation = {
            let mut inner = self.inner.lock().await;
            inner.room = None;
            inner.paused = false;
            inner.reconnect = None;
            inner.connection = "disconnected".into();
            inner.peer_name = None;
            inner.sender = None;
            inner.generation = inner.generation.wrapping_add(1);
            inner.generation
        };
        self.desired.send_replace(Desired { room: None, paused: false, generation });
        self.emit_state().await;
        Ok(())
    }

    pub async fn disconnect(&self) -> Result<()> {
        self.pause().await
    }

    /* ---------- supervisor ---------- */

    /// Runs forever (until the event channel is gone). One task owns all engine
    /// children, so retries can never leak a duplicate worker.
    pub async fn run(self: Arc<Self>) {
        // The mailbox session is independent of the overlay supervisor so
        // offline messages keep flowing while EasyTier reconnects.
        let mailbox = self.clone();
        let mailbox_task = tokio::spawn(async move { mailbox.mailbox_loop().await });
        let mut changes = self.desired.subscribe();
        loop {
            let desired = changes.borrow_and_update().clone();
            match desired.room {
                Some(room) if !desired.paused => self.connect_loop(room, &changes).await,
                _ => {
                    {
                        let mut inner = self.inner.lock().await;
                        inner.reconnect = None;
                        if inner.sender.is_none() {
                            inner.connection = "disconnected".into();
                            inner.peer_name = None;
                        }
                    }
                    self.emit_state().await;
                }
            }
            if self.shutdown.is_cancelled() {
                break;
            }
            // `connect_loop` waits on cloned receivers, so this receiver is still
            // marked "unseen" when the desired state changed and returns at once.
            tokio::select! {
                result = changes.changed() => {
                    if result.is_err() {
                        break;
                    }
                }
                _ = self.shutdown.cancelled() => break,
            }
        }
        let _ = mailbox_task.await;
    }

    async fn connect_loop(self: &Arc<Self>, room: StoredRoom, changes: &watch::Receiver<Desired>) {
        let spec = match room.spec() {
            Ok(spec) => spec,
            Err(error) => {
                self.emit_error(format!("房间参数无效：{error:#}"));
                return;
            }
        };
        let mut failures: u32 = 0;
        if !self.engine.is_file() {
            self.inner.lock().await.connection = "disconnected".into();
            self.emit_state().await;
            self.emit_error("未找到联机组件 EasyTier；离线信箱仍可使用".into());
            return;
        }
        loop {
            if self.shutdown.is_cancelled() {
                return;
            }
            {
                let mut inner = self.inner.lock().await;
                inner.connection = "connecting".into();
                inner.peer_name = None;
                if inner.room.as_ref() != Some(&room) {
                    return;
                }
            }
            let outcome = self.attempt(&spec, changes).await;
            match outcome {
                Attempt::Changed | Attempt::Shutdown => return,
                Attempt::SessionEnded => failures = 0,
                Attempt::Failed => failures = failures.saturating_add(1),
            }
            if self.policy.max_failures > 0 && failures >= self.policy.max_failures {
                {
                    let mut inner = self.inner.lock().await;
                    inner.reconnect = None;
                    inner.connection = "disconnected".into();
                }
                self.emit_state().await;
                self.emit_error(format!(
                    "连续 {failures} 次连接失败，已暂停自动重连。请检查网络或服务器后手动重连"
                ));
                return;
            }
            let delay = self.policy.delay(failures + 1);
            {
                let mut inner = self.inner.lock().await;
                inner.connection = "connecting".into();
                inner.reconnect = Some(ReconnectStatus {
                    attempt: failures + 1,
                    next_retry_ms: delay.as_millis() as u64,
                });
            }
            self.emit_state().await;
            let mut notify = changes.clone();
            tokio::select! {
                _ = sleep(delay) => {}
                _ = notify.changed() => return,
                _ = self.shutdown.cancelled() => return,
            }
        }
    }

    async fn attempt(self: &Arc<Self>, spec: &RoomSpec, changes: &watch::Receiver<Desired>) -> Attempt {
        let state_dir = self.data_dir.join("engine-state");
        if let Err(error) = std::fs::create_dir_all(&state_dir) {
            self.emit_error(format!("无法创建联机组件目录：{error}"));
            return Attempt::Failed;
        }
        let _ = state::restrict_dir(&state_dir);
        let process = match network::spawn_engine(&self.engine, &state_dir, spec) {
            Ok(process) => process,
            Err(error) => {
                self.emit_error(format!("{error:#}"));
                return Attempt::Failed;
            }
        };
        let network::EngineProcess { mut child, forward_port } = process;
        self.inner.lock().await.attempt_active = true;
        // Cancelled when the app exits, so `child.kill()` always runs before the
        // process goes away instead of leaving an EasyTier behind.
        let cancel = self.shutdown.child_token();
        let mut notify = changes.clone();
        let result = tokio::select! {
            biased;
            _ = self.shutdown.cancelled() => Attempt::Shutdown,
            _ = notify.changed() => Attempt::Changed,
            status = child.wait() => {
                self.emit_error(format!("联机组件已退出：{status:?}"));
                Attempt::Failed
            }
            result = self.establish(spec, forward_port, &cancel) => match result {
                Ok(()) => Attempt::SessionEnded,
                Err(error) => {
                    // A peer being away is normal; the state/countdown already
                    // explains it without repeated error toasts every timeout.
                    let _ = error;
                    Attempt::Failed
                }
            },
        };
        // The child is killed on every attempt boundary (config change, session
        // end, failure and app shutdown), so no orphaned EasyTier stays behind.
        let _ = child.kill().await;
        self.inner.lock().await.attempt_active = false;
        self.mark_disconnected().await;
        result
    }

    async fn establish(
        self: &Arc<Self>,
        spec: &RoomSpec,
        forward_port: u16,
        cancel: &CancellationToken,
    ) -> Result<()> {
        let deadline = Instant::now() + self.policy.connect_timeout;
        let secret = spec.network_secret.clone();
        let generation = {
            let inner = self.inner.lock().await;
            anyhow::ensure!(!inner.paused && inner.room.as_ref() == Some(&spec.stored), "连接已改变");
            inner.generation
        };
        if spec.stored.role == "host" {
            let listener = TcpListener::bind(("127.0.0.1", network::SERVICE_PORT))
                .await
                .context("聊天端口被占用，请关闭另一个应用实例")?;
            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    bail!("等待对方加入超时，请确认对方使用了同一配对码，且服务器一致");
                }
                let (stream, _) = timeout(remaining, listener.accept())
                    .await
                    .context("等待对方加入超时，请确认对方使用了同一配对码，且服务器一致")??;
                match self.session(stream, &secret, true, cancel.clone(), &spec.room_id, generation).await {
                    Ok(()) => return Ok(()),
                    Err(error) => {
                        // A stranger or a stale peer can reach the loopback port;
                        // keep waiting for the real partner until the deadline.
                        self.emit_error(format!("对方验证失败：{error:#}"));
                    }
                }
            }
        } else {
            loop {
                if Instant::now() >= deadline {
                    bail!("连接超时，请确认对方已创建房间，且配对码未失效");
                }
                if let Ok(Ok(stream)) =
                    timeout(Duration::from_secs(3), TcpStream::connect(("127.0.0.1", forward_port)))
                        .await
                {
                    match self.session(stream, &secret, false, cancel.clone(), &spec.room_id, generation).await {
                        Ok(()) => return Ok(()),
                        Err(_) => (),
                    }
                }
                sleep(Duration::from_millis(700)).await;
            }
        }
    }

    pub(crate) async fn mark_disconnected(&self) {
        let _operation = self.operations.lock().await;
        {
            let mut direct = self.direct.lock().await;
            direct.cancel.cancel();
            *direct = direct_files::DirectState::default();
        }
        let mut inner = self.inner.lock().await;
        inner.sender = None;
        inner.peer_name = None;
        inner.reconnect = None;
        inner.connection = "disconnected".into();
        // Mailbox deliveries continue while the P2P session is down. A transient
        // disconnect must never fail or cancel a durable transfer.
        let room = inner.history_room.clone();
        let event = state_event(&inner);
        drop(inner);
        if let Err(error) = self.recover_direct_room(&room).await { self.emit_error(error.to_string()); }
        self.emit(event);
    }

    /* ---------- messaging and files ---------- */

    pub async fn send_message(&self, text: String) -> Result<Message> {
        let _operation = self.operations.lock().await;
        if text.trim().is_empty() || text.len() > 16 * 1024 {
            bail!("消息不能为空，且不能超过 16 KiB");
        }
        // Persist first: the durable mailbox outbox is the source of truth. A
        // direct send is only a latency optimisation and may fail silently.
        let message = self.mailbox_send_message(text.clone()).await?;
        if let Some(sender) = self.inner.lock().await.sender.clone() {
            let id = Uuid::parse_str(&message.id)?;
            let _ = sender.try_send(Frame::Control(Control::Chat { id, text, timestamp: message.timestamp }));
        }
        Ok(message)
    }

    pub async fn offer_file(&self, path: PathBuf) -> Result<Transfer> {
        if let Some(transfer) = self.direct_offer(&path).await? { return Ok(transfer); }
        self.mailbox_offer_file(path).await
    }

    pub async fn accept_file(&self, id: Uuid) -> Result<()> {
        let _operation = self.operations.lock().await;
        let room = self.current_room_id().await?;
        if self.store.transfer(&room, &id.to_string())?.is_some_and(|t| t.via.as_deref() == Some("direct")) {
            return self.direct_accept(&room, id).await;
        }
        self.mailbox_accept_file(id).await
    }

    async fn cancel_file(&self, id: Uuid, rejected: bool) -> Result<()> {
        let _operation = self.operations.lock().await;
        let room = self.current_room_id().await?;
        if self.store.transfer(&room, &id.to_string())?.is_some_and(|t| t.via.as_deref() == Some("direct")) {
            return self.direct_cancel(&room, id, rejected, true).await;
        }
        self.mailbox_cancel_file(id, rejected).await
    }

    pub async fn cancel_transfer(&self, id: Uuid) -> Result<()> { self.cancel_file(id, false).await }
    pub async fn reject_file(&self, id: Uuid) -> Result<()> { self.cancel_file(id, true).await }

    /* ---------- session ---------- */

    async fn session(
        self: &Arc<Self>,
        mut stream: TcpStream,
        secret: &str,
        server: bool,
        cancel: CancellationToken,
        room_id: &str,
        generation: u64,
    ) -> Result<()> {
        stream.set_nodelay(true)?;
        let name = self.inner.lock().await.profile.display_name.clone();
        let peer = timeout(
            self.policy.handshake_timeout,
            wire::authenticate(&mut stream, secret, &name, server),
        )
        .await
        .context("验证对方身份超时")??;
        let (mut reader, mut writer) = stream.into_split();
        let (tx, mut rx) = mpsc::channel::<Frame>(8);
        {
            let mut inner = self.inner.lock().await;
            anyhow::ensure!(inner.generation == generation && inner.history_room == room_id && !inner.paused, "连接已改变");
            {
                let mut direct = self.direct.lock().await;
                direct.cancel.cancel();
                *direct = direct_files::DirectState::default();
                direct.supported = peer.files;
            }
            inner.sender = Some(tx.clone());
            inner.connection = "connected".into();
            inner.peer_name = Some(peer.name.clone());
            inner.reconnect = None;
        }
        self.emit_state().await;
        let write_loop = async {
            let mut heartbeat = tokio::time::interval(Duration::from_secs(20));
            loop {
                tokio::select! {
                    frame = rx.recv() => match frame {
                        Some(frame) => wire::write_frame(&mut writer, &frame).await?,
                        None => break,
                    },
                    _ = heartbeat.tick() => wire::write_frame(&mut writer, &Frame::Control(Control::Ping)).await?,
                }
            }
            Ok::<(), anyhow::Error>(())
        };
        let read_loop = async {
            loop {
                let frame = timeout(Duration::from_secs(70), wire::read_frame(&mut reader))
                    .await
                    .context("连接长时间未响应")??;
                self.handle(frame, &tx, room_id, generation).await?;
            }
            #[allow(unreachable_code)]
            Ok::<(), anyhow::Error>(())
        };
        let outcome = tokio::select! {
            _ = cancel.cancelled() => Ok(()),
            result = write_loop => result,
            result = read_loop => result,
        };
        // After the handshake any failure is a session end, not a pairing
        // failure: surface it and let the supervisor retry with backoff.
        if !cancel.is_cancelled() {
            if let Err(error) = outcome {
                self.emit_error(format!("连接已断开：{error:#}"));
            }
        }
        self.mark_disconnected().await;
        Ok(())
    }

    async fn handle(
        self: &Arc<Self>,
        frame: Frame,
        tx: &mpsc::Sender<Frame>,
        room_id: &str,
        generation: u64,
    ) -> Result<()> {
        let _operation = self.operations.lock().await;
        anyhow::ensure!(self.generation_is_current(room_id, generation).await, "连接已改变");
        let rejected = matches!(&frame, Frame::Control(Control::Reject { .. }));
        match frame {
            Frame::Control(Control::Ping) => { let _ = tx.try_send(Frame::Control(Control::Pong)); },
            Frame::Control(Control::Pong) => (),
            Frame::Control(Control::Receipt { id }) => {
                if let Some(message) = self.store.apply_receipt(room_id, &id.to_string(), None, "direct")? {
                    self.emit_in_room(room_id, AppEvent::Message { message }).await;
                }
                self.refresh_mailbox_pending(room_id).await;
            }
            Frame::Control(Control::Chat { id, text, timestamp }) => {
                if text.trim().is_empty() || text.len() > 16 * 1024 {
                    bail!("对方发送了无效消息");
                }
                let (room, duplicate) = {
                    let room = room_id.to_owned();
                    let duplicate = self.store.has_message(&room, &id.to_string())?;
                    (room, duplicate)
                };
                if !duplicate {
                    // Does the durable mailbox already own this message? A direct
                    // copy must not overwrite its pending/stored state or the
                    // server fallback still in flight.
                    let owned = self.store.message(&room, &id.to_string())?;
                    let message = match owned {
                        Some(existing) => existing,
                        None => Message {
                            id: id.to_string(),
                            text,
                            outgoing: false,
                            timestamp: if timestamp == 0 { now() } else { timestamp },
                            delivery: "delivered".into(),
                            via: Some("direct".into()),
                        },
                    };
                    self.store.save_message(&room, &message)?;
                    self.emit_in_room(&room, AppEvent::Message { message }).await;
                }
                let _ = tx.try_send(Frame::Control(Control::Receipt { id }));
            }
            Frame::Control(Control::Offer { id, name, size, hash }) => {
                if self.direct.lock().await.supported {
                    self.direct_receive_offer(room_id, id, name, size, hash).await?;
                }
            }
            Frame::Control(Control::Accept { id }) => {
                if self.direct.lock().await.supported {
                    self.direct_start_send(room_id, generation, id, tx.clone()).await?;
                }
            }
            Frame::Control(Control::Finish { id }) => self.direct_finish(room_id, id, tx).await?,
            Frame::Control(Control::Complete { id }) => self.direct_complete(room_id, id).await?,
            Frame::Control(Control::Cancel { id }) | Frame::Control(Control::Reject { id }) => {
                if self.store.transfer(room_id, &id.to_string())?.is_some_and(|t| t.via.as_deref() == Some("direct")) {
                    self.direct_cancel(room_id, id, rejected, false).await?;
                }
            }
            Frame::Chunk { id, bytes } => self.direct_chunk(room_id, id, bytes).await?,
        }
        Ok(())
    }

    /// Kill the engine and stop retrying when the app exits. Deliberately does
    /// not persist `paused`, so the next launch still auto-reconnects.
    /// Terminal: after this the instance is not reused. Kills any live engine
    /// child (including one still dialling before a session) and waits briefly
    /// for the supervisor to observe the cancellation.
    pub async fn shutdown(&self) {
        let room = self.desired.borrow().room.clone();
        let generation = {
            let mut inner = self.inner.lock().await;
            inner.generation = inner.generation.wrapping_add(1);
            inner.generation
        };
        // Deliberately not persisted as paused: a restart should reconnect.
        self.desired.send_replace(Desired { room, paused: true, generation });
        self.shutdown.cancel();
        for _ in 0..150 {
            {
                let inner = self.inner.lock().await;
                if !inner.attempt_active && inner.sender.is_none() {
                    break;
                }
            }
            sleep(Duration::from_millis(20)).await;
        }
    }
}

fn state_event(inner: &Inner) -> AppEvent {
    AppEvent::State {
        connection: inner.connection.clone(),
        peer_name: inner.peer_name.clone(),
        paused: inner.paused,
        reconnect: inner.reconnect.clone(),
        room: inner
            .room
            .as_ref()
            .and_then(|room| room.spec().ok())
            .map(|spec| RoomInfo::from_spec(&spec)),
    }
}

impl RoomInfo {
    fn from_spec(spec: &RoomSpec) -> Self {
        Self {
            role: spec.stored.role.clone(),
            room_id: spec.room_id.clone(),
            network_name: spec.network_name.clone(),
            virtual_ip: spec.virtual_ip.clone(),
            peer_virtual_ip: spec.peer_ip.clone(),
        }
    }
}

fn default_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "我的电脑".into())
}

#[cfg(test)]
mod tests;
