//! Durable mailbox session: encrypted outbox, live event intake and consented
//! file transfer. This task is intentionally independent from the EasyTier
//! supervisor: offline chat and files keep working while the overlay is down.

use crate::{
    crypto::{self, peer_role},
    files::{hash_file, validate_name, MAX_FILE_SIZE},
    mailbox::{
        Body, IncomingEvent, MailboxClient, MailboxError,
    },
    model::{now, Message, Transfer},
    storage::{DownloadRow, OutboxRow, UploadRow},
    Core, Desired,
};
use anyhow::{anyhow, bail, Context, Result};
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::{sync::watch, time::sleep};
use uuid::Uuid;

const TICK: Duration = Duration::from_millis(900);
const DIRECT_GRACE_MS: u64 = 1_500;
const RETRY_CAP: Duration = Duration::from_secs(30);
const CHUNKS_PER_TICK: u64 = 4;

impl Core {
    pub(crate) async fn mailbox_loop(self: std::sync::Arc<Self>) {
        let mut changes = self.desired.subscribe();
        loop {
            let desired = changes.borrow_and_update().clone();
            if self.shutdown.is_cancelled() {
                break;
            }
            let configured = desired.room.is_some();
            if let Some(room) = desired.room.clone().filter(|_| !desired.paused) {
                let generation = desired.generation;
                self.set_mailbox_for(generation, "connecting", None).await;
                let result = self.mailbox_session(room, generation, &mut changes).await;
                if let Err(error) = result {
                    self.set_mailbox_for(generation, "error", Some(error.to_string())).await;
                }
            } else {
                let state = if configured { "paused" } else { "notConfigured" };
                self.set_mailbox_for(desired.generation, state, None).await;
            }
            if self.shutdown.is_cancelled() {
                break;
            }
            if changes.has_changed().unwrap_or(false) {
                continue;
            }
            tokio::select! {
                _ = changes.changed() => {},
                _ = self.shutdown.cancelled() => break,
            }
        }
        self.set_mailbox("notConfigured", None).await;
    }

    async fn mailbox_session(
        self: &std::sync::Arc<Self>,
        room: crate::pairing::StoredRoom,
        generation: u64,
        changes: &mut watch::Receiver<Desired>,
    ) -> Result<()> {
        let spec = room.spec().context("离线信箱配置无效")?;
        let role = spec.stored.role.clone();
        let seed = spec.stored.seed.clone();
        let room_id = spec.room_id.clone();
        let peer = peer_role(&role)?;
        #[cfg(test)]
        let initialized = match self.transport.lock().unwrap().clone() {
            Some(transport) => Ok(MailboxClient::with_transport(
                &room.server.mailbox_url, &room.server.access_token, &spec, transport,
            )),
            None => MailboxClient::new(&room.server, &spec),
        };
        #[cfg(not(test))]
        let initialized = MailboxClient::new(&room.server, &spec);
        let client = match initialized {
            Ok(client) => client,
            Err(error) => {
                self.set_mailbox_for(generation, "error", Some(error.to_string())).await;
                return Ok(());
            }
        };

        let mut failures = 0u32;
        loop {
            if self.session_stale(changes, generation) {
                return Ok(());
            }
            let mut changed = changes.clone();
            let registered = tokio::select! {
                biased;
                _ = self.shutdown.cancelled() => return Ok(()),
                _ = changed.changed() => {
                    changes.mark_changed();
                    return Ok(());
                }
                result = client.register() => result,
            };
            match registered {
                Ok(()) => break,
                Err(error) if error.retryable() => {
                    failures += 1;
                    self.set_mailbox_for(generation, "retrying", Some(error.to_string())).await;
                    let delay = backoff(failures);
                    if self.wait_session(changes, generation, delay).await {
                        return Ok(());
                    }
                }
                Err(error) => {
                    self.set_mailbox_for(generation, "error", Some(error.to_string())).await;
                    return Ok(());
                }
            }
        }

        if self.session_stale(changes, generation) {
            return Ok(());
        }
        self.set_mailbox_for(generation, "online", None).await;
        failures = 0;
        loop {
            if self.session_stale(changes, generation) {
                return Ok(());
            }
            // Drop the entire in-flight operation when the user pauses,
            // switches rooms or exits. Merely checking an epoch between ticks
            // leaves an old downloader alive to append alongside its restored
            // replacement after restart.
            let mut changed = changes.clone();
            let tick = tokio::select! {
                biased;
                _ = self.shutdown.cancelled() => return Ok(()),
                _ = changed.changed() => {
                    changes.mark_changed();
                    return Ok(());
                }
                result = self.mailbox_tick(&client, &room_id, &role, &seed, peer, generation) => result,
            };
            match tick {
                Ok(()) => {
                    failures = 0;
                    self.set_mailbox_for(generation, "online", None).await;
                }
                Err(error) if error.retryable() || error.is_quota() => {
                    failures += 1;
                    if self.session_stale(changes, generation) {
                        return Ok(());
                    }
                    self.set_mailbox_for(generation, "retrying", Some(error.to_string())).await;
                    let delay = if error.is_quota() { RETRY_CAP } else { backoff(failures) };
                    if self.wait_session(changes, generation, delay).await {
                        return Ok(());
                    }
                }
                Err(error) if error.is_auth() => {
                    self.set_mailbox_for(generation, "error", Some("离线信箱凭证无效，请重新导入服务器配对码".into()))
                        .await;
                    return Ok(());
                }
                Err(error) => {
                    // Protocol/config problems are surfaced rather than retried
                    // forever; a fresh session starts on resume or room change.
                    self.set_mailbox_for(generation, "error", Some(error.to_string())).await;
                    return Ok(());
                }
            }
            tokio::select! {
                _ = sleep(TICK) => {},
                _ = self.mailbox_wake.notified() => {},
                _ = changes.changed() => {},
                _ = self.shutdown.cancelled() => return Ok(()),
            }
        }
    }

    /// A session is stale as soon as the user pauses, switches rooms, imports a
    /// new server (all bump the generation) or the app shuts down. Comparing the
    /// full configuration epoch prevents a re-installed identical room from
    /// silently reusing in-flight work from the previous session.
    fn session_stale(&self, changes: &watch::Receiver<Desired>, generation: u64) -> bool {
        if self.shutdown.is_cancelled() {
            return true;
        }
        let desired = changes.borrow();
        desired.paused || desired.generation != generation || desired.room.is_none()
    }

    /// Returns true when the session must end.
    async fn wait_session(
        &self,
        changes: &mut watch::Receiver<Desired>,
        generation: u64,
        duration: Duration,
    ) -> bool {
        let deadline = tokio::time::Instant::now() + duration;
        loop {
            if self.session_stale(changes, generation) {
                return true;
            }
            tokio::select! {
                _ = tokio::time::sleep_until(deadline) => {
                    return self.session_stale(changes, generation);
                },
                _ = changes.changed() => {},
                _ = self.shutdown.cancelled() => return true,
            }
        }
    }

    async fn set_mailbox(&self, state: &str, message: Option<String>) {
        self.set_mailbox_scoped(None, state, message).await;
    }

    async fn set_mailbox_for(&self, generation: u64, state: &str, message: Option<String>) {
        self.set_mailbox_scoped(Some(generation), state, message).await;
    }

    async fn set_mailbox_scoped(&self, generation: Option<u64>, state: &str, message: Option<String>) {
        let mut inner = self.inner.lock().await;
        if generation.is_some_and(|expected| inner.generation != expected) {
            return;
        }
        let pending = if inner.history_room.is_empty() { 0 }
            else { self.store.pending_outbox(&inner.history_room).unwrap_or(0) };
        if inner.mailbox_state == state && inner.mailbox_error == message && inner.mailbox_pending == pending {
            return;
        }
        inner.mailbox_state = state.into();
        inner.mailbox_error = message.clone();
        inner.mailbox_pending = pending;
        // The synchronous broadcast is ordered before any room change that
        // acquires this same lock, so a stale session cannot update the new UI.
        self.emit(crate::model::AppEvent::Mailbox { state: state.to_owned(), message, pending });
    }

    async fn mailbox_tick(
        &self,
        client: &MailboxClient,
        room_id: &str,
        role: &str,
        seed: &str,
        peer: &str,
        generation: u64,
    ) -> std::result::Result<(), MailboxError> {
        // Drain authenticated incoming events before attempting new posts, so
        // a full server queue cannot prevent both peers from freeing it.
        let events = client.list_events().await?;
        for event in events {
            if !self.generation_is_current(room_id, generation).await { return Ok(()); }
            self.handle_mailbox_event(client, room_id, role, seed, peer, event).await?;
        }
        // 1. Durable outbox. Ciphertext is sealed once and reused verbatim, so
        // retries never change nonce or bytes.
        let due = self
            .store
            .due_events(room_id, now(), 4)
            .map_err(db_error)?;
        for row in due {
            self.post_outbox(client, room_id, row).await?;
        }

        // 3. One file operation per category keeps message delivery responsive.
        self.upload_tick(client, room_id, role, seed, peer).await?;
        self.download_tick(client, room_id, role, seed).await?;
        Ok(())
    }

    async fn post_outbox(
        &self,
        client: &MailboxClient,
        room_id: &str,
        row: OutboxRow,
    ) -> std::result::Result<(), MailboxError> {
        if row.kind == "offer" {
            let id = row.dedupe.strip_prefix("file-offer:").unwrap_or("");
            let upload = self.store.upload(room_id, id).map_err(db_error)?;
            match upload {
                Some(upload) if upload.status == "waiting" => {
                    if let Err(error) = client.file_manifest(id).await {
                        if !error.retryable() {
                            self.store.fail_event(room_id, &row.event_id, &error.to_string()).map_err(db_error)?;
                            return self.classify_upload_error(room_id, &upload, error).await;
                        }
                        return Err(error);
                    }
                }
                _ => { self.store.delete_event(room_id, &row.event_id).map_err(db_error)?; return Ok(()); }
            }
        }
        match client.post_event(&row.event_id, &row.envelope).await {
            Ok(created) => {
                if created.id != row.event_id {
                    return Err(MailboxError {
                        status: None,
                        code: "protocol".into(),
                        message: "离线信箱事件回执不一致".into(),
                    });
                }
                if row.kind == "message" {
                    // `event_id` equals the message id: keep the encrypted copy
                    // for dedupe/retry, but never claim delivery on `stored`.
                    match self.store.mark_message_stored(room_id, &row.event_id) {
                        Ok(Some(message)) => self.emit_in_room(room_id, crate::model::AppEvent::Message { message }).await,
                        Ok(None) => {}
                        Err(_) => {
                            // The local promotion failed: retry on the next tick
                            // rather than reporting success we did not persist.
                            return Err(db_error("无法保存消息状态"));
                        }
                    }
                } else {
                    if row.kind == "fileAction" {
                        if let Some(id) = row.dedupe.strip_prefix("file-cancel-").or_else(||row.dedupe.strip_prefix("file-reject-")) {
                            // The event and deletion are idempotent. Retain the
                            // immutable outbox row until both succeed, even if
                            // the peer is offline or this process exits.
                            client.delete_file(id).await?;
                        }
                    }
                    // Non-message rows (offer / receipt / file action) are deleted
                    // once the server holds the durable copy. Keeping a `stored`
                    // row would suppress a later, legitimately new event that
                    // reuses the same dedupe key (e.g. a retried receipt).
                    self.store.delete_event(room_id, &row.event_id).map_err(db_error)?;
                }
                self.refresh_mailbox_pending(room_id).await;
                Ok(())
            }
            Err(error) if error.retryable() || error.is_quota() => {
                let delay = if error.is_quota() { RETRY_CAP } else { backoff(row.attempts + 1) }.as_millis() as u64;
                let _ = self.store.reschedule_event(
                    room_id,
                    &row.event_id,
                    now().saturating_add(delay),
                    &error.to_string(),
                );
                Err(error)
            }
            Err(error) if error.is_auth() => Err(error),
            Err(error) => {
                self.store.fail_event(room_id, &row.event_id, &error.to_string()).map_err(db_error)?;
                self.refresh_mailbox_pending(room_id).await;
                // Show a terminal protocol rejection instead of leaving an
                // unsendable message silently labelled as pending forever.
                Err(error)
            }
        }
    }

    pub(crate) async fn refresh_mailbox_pending(&self, room_id: &str) {
        let pending = self.store.pending_outbox(room_id).unwrap_or(0);
        let mut inner = self.inner.lock().await;
        if inner.history_room != room_id || inner.mailbox_pending == pending {
            return;
        }
        inner.mailbox_pending = pending;
        let state = inner.mailbox_state.clone();
        let message = inner.mailbox_error.clone();
        drop(inner);
        self.emit(crate::model::AppEvent::Mailbox { state, message, pending });
    }

    async fn handle_mailbox_event(
        &self,
        client: &MailboxClient,
        room_id: &str,
        _role: &str,
        seed: &str,
        _peer: &str,
        event: IncomingEvent,
    ) -> std::result::Result<(), MailboxError> {
        // Already fully processed: only the server-side delete is left.
        if self.store.has_seen_event(room_id, &event.id).unwrap_or(false) {
            let _ = client.delete_event(&event.id).await;
            return Ok(());
        }
        // A poison event stays quarantined on the server so a human can inspect
        // it; repeated ticks do not re-emit the error.
        if self.store.has_quarantined_event(room_id, &event.id).unwrap_or(false) {
            return Ok(());
        }
        let payload = match base64_decode(&event.payload) {
            Ok(payload) => payload,
            Err(error) => {
                self.quarantine(room_id, &event, "离线消息编码无效").await?;
                let _ = error;
                return Ok(());
            }
        };
        let plaintext = match crypto::open_event(
            seed,
            room_id,
            &event.sender,
            &event.recipient,
            &event.id,
            &payload,
        ) {
            Ok(plaintext) => plaintext,
            Err(_) => {
                self.quarantine(room_id, &event, "无法验证的离线消息").await?;
                return Ok(());
            }
        };
        let body = match Body::decode(&plaintext) {
            Ok(body) => body,
            Err(_) => {
                self.quarantine(room_id, &event, "离线消息格式无效").await?;
                return Ok(());
            }
        };
        // Apply the effect first. Only after the local state is durable do we
        // acknowledge to the server, so a crash can never lose a message.
        let result: Result<()> = match body {
            Body::Message { id, text, timestamp } => {
                self.receive_mailbox_message(room_id, &id, text, timestamp).await
            }
            Body::Receipt { message_id } => {
                match self.store.apply_receipt(room_id, &message_id, Some(&event.id), "mailbox") {
                    Ok(Some(message)) => {
                        self.emit_in_room(room_id, crate::model::AppEvent::Message { message }).await
                    }
                    Ok(None) => {}
                    Err(_) => {}
                }
                // A receipt whose message is already delivered is still acked:
                // apply_receipt records the seen marker atomically.
                Ok(())
            }
            Body::Offer { id, name, size, hash } => {
                self.receive_offer(room_id, &id, name, size, hash).await
            }
            Body::FileComplete { id } => {
                self.receive_file_complete(client, room_id, &id).await
            }
            Body::Cancel { id } => self.receive_cancel(client, room_id, &id, false).await,
            Body::Reject { id } => self.receive_cancel(client, room_id, &id, true).await,
        };
        if let Err(error) = result {
            // Do not crash the session and do not acknowledge an event we did
            // not apply; the server copy stays for a later retry or inspection.
            self.emit_in_room(room_id, crate::model::AppEvent::Error {
                message: format!("处理离线消息失败，稍后重试：{error}"),
            })
            .await;
            return Ok(());
        }
        // Receipts already recorded their seen marker transactionally with the
        // effect; everything else is marked here before the server delete.
        self.store.mark_seen_event(room_id, &event.id).map_err(db_error)?;
        if let Err(error) = client.delete_event(&event.id).await {
            if !error.is_missing() && error.retryable() {
                return Err(error);
            }
        }
        Ok(())
    }

    async fn quarantine(
        &self,
        room_id: &str,
        event: &IncomingEvent,
        reason: &str,
    ) -> std::result::Result<(), MailboxError> {
        self.store.quarantine_event(room_id, event, reason).map_err(db_error)?;
        self.emit_in_room(room_id, crate::model::AppEvent::Error {
            message: format!("收到一条{reason}，已暂时隔离保留").into(),
        })
        .await;
        Ok(())
    }

    async fn receive_mailbox_message(
        &self,
        room_id: &str,
        id: &str,
        text: String,
        timestamp: u64,
    ) -> Result<()> {
        let duplicate = self.store.has_message(room_id, id)?;
        if !duplicate {
            let message = Message {
                id: id.to_owned(),
                text,
                outgoing: false,
                timestamp,
                delivery: "delivered".into(),
                via: Some("mailbox".into()),
            };
            self.store.save_message(room_id, &message)?;
            self.emit_in_room(room_id, crate::model::AppEvent::Message { message }).await;
        }
        self.enqueue_receipt(room_id, id).await
    }

    async fn enqueue_receipt(&self, room_id: &str, message_id: &str) -> Result<()> {
        let (role, peer, seed) = {
            let inner = self.inner.lock().await;
            let room = inner.room.clone().context("房间已切换")?;
            let spec = room.spec()?;
            if spec.room_id != room_id {
                return Ok(());
            }
            let peer = peer_role(&spec.stored.role)?.to_owned();
            (spec.stored.role, peer, spec.stored.seed)
        };
        let event_id = Uuid::new_v4().to_string();
        let plaintext = Body::Receipt { message_id: message_id.into() }.encode()?;
        let envelope = crypto::seal_event(&seed, room_id, &role, &peer, &event_id, &plaintext)?;
        self.store.enqueue_event(
            room_id,
            &event_id,
            &format!("receipt:{message_id}"),
            "receipt",
            &peer,
            &envelope,
            now(),
        )?;
        self.mailbox_wake.notify_one();
        Ok(())
    }

    async fn receive_offer(&self, room_id: &str, id: &str, name: String, size: u64, hash: String) -> Result<()> {
        if size > MAX_FILE_SIZE {
            bail!("对方文件超过 1 GiB 限制");
        }
        if Uuid::parse_str(id).is_err()
            || hash.len() != 64
            || !hash.bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            bail!("对方文件信息无效");
        }
        validate_name(&name)?;
        if self.store.download(room_id, id)?.is_some() {
            return Ok(());
        }
        let download = DownloadRow {
            room: room_id.into(),
            file_id: id.into(),
            sender: "peer".into(),
            name: name.clone(),
            size,
            hash,
            status: "offered".into(),
            target_dir: None,
            temp_path: None,
            next_index: 0,
            error: None,
            created_at: now(),
            final_path: None,
        };
        let transfer = Transfer {
            id: id.into(),
            name,
            size,
            transferred: 0,
            outgoing: false,
            status: "offered".into(),
            error: None,
        };
        self.store.insert_offer(&download, &transfer)?;
        self.emit_in_room(room_id, crate::model::AppEvent::Transfer { transfer }).await;
        Ok(())
    }

    async fn receive_file_complete(
        &self,
        client: &MailboxClient,
        room_id: &str,
        id: &str,
    ) -> Result<()> {
        if let Some(upload) = self.store.upload(room_id, id)? {
            // Only an upload that actually reached the peer may become
            // completed. A late completion for a failed/cancelled upload must
            // never rewrite terminal history.
            let transfer = Transfer {
                id: id.into(),
                name: upload.name.clone(),
                size: upload.size,
                transferred: upload.size,
                outgoing: true,
                status: "completed".into(),
                error: None,
            };
            if self.store.complete_upload(room_id, id, &transfer)? {
                let _ = remove_dir_all(&spool_dir(&self.data_dir, room_id, id));
                self.emit_in_room(room_id, crate::model::AppEvent::Transfer { transfer }).await;
            }
        }
        // Deleting is safe only after the local completion transaction (or when
        // no local upload exists any more); a DB error above retries the event.
        let _ = client.delete_file(id).await;
        Ok(())
    }

    async fn receive_cancel(
        &self,
        client: &MailboxClient,
        room_id: &str,
        id: &str,
        rejected: bool,
    ) -> Result<()> {
        let status = if rejected { "rejected" } else { "cancelled" };
        if let Some(upload) = self.store.upload(room_id, id)? {
            if !["completed", "cancelled", "rejected", "withdrawn"].contains(&upload.status.as_str()) {
                self.store.set_upload_status(room_id, id, status, None)?;
                let _ = remove_dir_all(&spool_dir(&self.data_dir, room_id, id));
                let transfer = Transfer {
                    id: id.into(),
                    name: upload.name.clone(),
                    size: upload.size,
                    transferred: 0,
                    outgoing: true,
                    status: status.into(),
                    error: None,
                };
                self.store.save_transfer(room_id, &transfer)?;
                self.emit_in_room(room_id, crate::model::AppEvent::Transfer { transfer }).await;
            }
            let _ = client.delete_file(id).await;
        }
        if let Some(download) = self.store.download(room_id, id)? {
            if !["completed", "cancelled", "rejected"].contains(&download.status.as_str()) {
                self.store
                    .set_download(room_id, id, status, download.target_dir.as_deref(), None, 0, None)?;
                if let Some(path) = download.temp_path.clone() {
                    let _ = fs::remove_file(path);
                }
                let transfer = Transfer {
                    id: id.into(),
                    name: download.name.clone(),
                    size: download.size,
                    transferred: 0,
                    outgoing: false,
                    status: status.into(),
                    error: None,
                };
                self.store.save_transfer(room_id, &transfer)?;
                self.emit_in_room(room_id, crate::model::AppEvent::Transfer { transfer }).await;
            }
        }
        Ok(())
    }

    /* ---------- uploads ---------- */

    async fn upload_tick(
        &self,
        client: &MailboxClient,
        room_id: &str,
        role: &str,
        seed: &str,
        _peer: &str,
    ) -> std::result::Result<(), MailboxError> {
        let uploads = self.store.uploads(room_id).map_err(db_error)?;
        for upload in uploads {
            match upload.status.as_str() {
                "pending" => {
                    let prepared = self.prepare_upload(room_id, &upload).await;
                    let _operation = self.operations.lock().await;
                    if !matches!(self.store.upload(room_id, &upload.file_id), Ok(Some(row)) if row.status == "pending") { continue; }
                    let (status, error) = match prepared { Ok(()) => ("uploading", None), Err(e) => ("failed", Some(e.to_string())) };
                    let transfer = Transfer { id: upload.file_id.clone(), name: upload.name.clone(), size: upload.size,
                        transferred: 0, outgoing: true, status: status.into(), error: error.clone() };
                    if self.store.update_upload_with_event(room_id, &upload.file_id, status, error.as_deref(), &transfer, None).map_err(db_error)? {
                        if error.is_some() { let _ = remove_dir_all(&spool_dir(&self.data_dir, room_id, &upload.file_id)); }
                        self.emit_in_room(room_id, crate::model::AppEvent::Transfer { transfer }).await;
                    }
                    return Ok(());
                }
                "uploading" => {
                    return self.upload_chunks(client, room_id, role, seed, &upload).await;
                }
                _ => {}
            }
        }
        Ok(())
    }

    async fn prepare_upload(&self, room_id: &str, upload: &UploadRow) -> Result<()> {
        let dir = spool_dir(&self.data_dir, room_id, &upload.file_id);
        let source = PathBuf::from(&upload.source_path);
        let expected_hash = upload.hash.clone();
        let expected_size = upload.size;
        let id = upload.file_id.clone();
        let room = room_id.to_owned();
        let seed = {
            let inner = self.inner.lock().await;
            let spec = inner.room.as_ref().context("房间已切换")?.spec()?;
            if spec.room_id != room_id {
                bail!("房间已切换");
            }
            spec.stored.seed
        };
        let dir_for_task = dir.clone();
        let file_id = id.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            if dir_for_task.exists() {
                remove_dir_all(&dir_for_task)?;
            }
            secure_dir(&dir_for_task)?;
            let source_meta = fs::metadata(&source).context("无法读取源文件")?;
            let mut input = fs::File::open(&source).context("无法打开源文件")?;
            let plain_path = dir_for_task.join("source.bin");
            let mut plain = secure_file(&plain_path)?;
            let mut hasher = Sha256::new();
            let mut buf = vec![0u8; 256 * 1024];
            let mut copied = 0u64;
            loop {
                let read = input.read(&mut buf).context("读取源文件失败")?;
                if read == 0 {
                    break;
                }
                copied += read as u64;
                if copied > expected_size {
                    bail!("源文件在发送前发生变化");
                }
                hasher.update(&buf[..read]);
                plain.write_all(&buf[..read]).context("写入加密暂存失败")?;
            }
            plain.sync_all()?;
            drop(plain);
            let after = fs::metadata(&source).context("无法读取源文件")?;
            if copied != expected_size
                || hex::encode(hasher.finalize()) != expected_hash
                || after.len() != source_meta.len()
            {
                let _ = remove_dir_all(&dir_for_task);
                bail!("源文件已改变，请重新选择发送");
            }
            // The immutable copy is the only plaintext that may be encrypted;
            // verify that copy itself before creating any uploadable chunk.
            let mut copy = fs::File::open(&plain_path)?;
            let mut copy_hash = Sha256::new();
            loop {
                let read = copy.read(&mut buf)?;
                if read == 0 { break; }
                copy_hash.update(&buf[..read]);
            }
            if hex::encode(copy_hash.finalize()) != expected_hash {
                let _ = remove_dir_all(&dir_for_task);
                bail!("文件暂存校验失败，请重新发送");
            }
            drop(copy);
            let chunks = crypto::chunk_count(expected_size)?;
            let mut chunk_file = fs::File::open(&plain_path)?;
            for index in 0..chunks {
                let expected = crypto::chunk_plaintext_size(expected_size, index)?;
                let mut bytes = vec![0u8; expected];
                chunk_file.read_exact(&mut bytes)?;
                let cipher = crypto::seal_file_chunk(
                    &seed,
                    &room,
                    &file_id,
                    index,
                    expected_size,
                    &expected_hash,
                    &bytes,
                )?;
                let path = dir_for_task.join(format!("chunk-{index:08}"));
                let mut output = secure_file(&path)?;
                output.write_all(&cipher)?;
                output.sync_all()?;
            }
            let mut extra = [0u8; 1];
            if chunk_file.read(&mut extra)? != 0 {
                bail!("源文件长度与声明不一致");
            }
            fs::remove_file(&plain_path)?;
            Ok(())
        })
        .await
        .map_err(|error| anyhow!("文件加密任务失败：{error}"))??;
        Ok(())
    }

    async fn upload_chunks(
        &self,
        client: &MailboxClient,
        room_id: &str,
        _role: &str,
        _seed: &str,
        upload: &UploadRow,
    ) -> std::result::Result<(), MailboxError> {
        let manifest = match client
            .declare_file(&upload.file_id, upload.encrypted_size, upload.chunk_count)
            .await
        {
            Ok(manifest) => manifest,
            Err(error) => return self.classify_upload_error(room_id, upload, error).await,
        };
        let dir = spool_dir(&self.data_dir, room_id, &upload.file_id);
        let received: HashSet<u64> = manifest.received.iter().copied().collect();
        let mut committed = 0u64;
        for index in &received {
            committed = committed.saturating_add(crypto::chunk_plaintext_size(upload.size, *index).unwrap_or(0) as u64);
        }
        if let Some(index) = (0..upload.chunk_count).find(|index| !received.contains(index)) {
            let path = dir.join(format!("chunk-{index:08}"));
            let expected = crypto::chunk_plaintext_size(upload.size, index).map_err(db_error)? + crypto::TAG_LEN;
            let cached = async {
                use tokio::io::AsyncReadExt;
                let input = tokio::fs::File::open(&path).await?;
                let mut bytes = Vec::with_capacity(expected + 1);
                input.take((expected + 1) as u64).read_to_end(&mut bytes).await?;
                std::io::Result::Ok(bytes)
            }.await;
            let bytes = match cached {
                Ok(bytes) if bytes.len() == expected => bytes,
                _ => return self.classify_upload_error(room_id, upload, MailboxError {
                    status: None, code: "spool".into(), message: "加密暂存文件缺失或损坏，请重新选择发送".into(),
                }).await,
            };
            if !matches!(self.store.upload(room_id, &upload.file_id), Ok(Some(row)) if row.status == "uploading") { return Ok(()); }
            if let Err(error) = client.put_chunk(&upload.file_id, index, &bytes).await {
                return self.classify_upload_error(room_id, upload, error).await;
            }
            let _operation = self.operations.lock().await;
            if !matches!(self.store.upload(room_id, &upload.file_id), Ok(Some(row)) if row.status == "uploading") { return Ok(()); }
            committed = committed.saturating_add(crypto::chunk_plaintext_size(upload.size, index).unwrap_or(0) as u64);
            self.update_transfer(room_id, &upload.file_id, "uploading", committed, upload.size, None).await;
            return Ok(());
        }

        let complete = match client.complete_file(&upload.file_id).await {
            Ok(manifest) => manifest,
            Err(error) => return self.classify_upload_error(room_id, upload, error).await,
        };
        if !complete.complete {
            return Ok(());
        }
        let event_id = upload.file_id.clone();
        let offer = Body::Offer {
            id: upload.file_id.clone(),
            name: upload.name.clone(),
            size: upload.size,
            hash: upload.hash.clone(),
        }
        .encode()
        .map_err(db_error)?;
        let (role, peer, seed) = self.current_identity(room_id).await.map_err(db_error)?;
        let envelope = crypto::seal_event(&seed, room_id, &role, &peer, &event_id, &offer)
            .map_err(db_error)?;
        let _operation = self.operations.lock().await;
        if !matches!(self.store.upload(room_id, &upload.file_id), Ok(Some(row)) if row.status == "uploading") { return Ok(()); }
        let waiting = Transfer { id: upload.file_id.clone(), name: upload.name.clone(), size: upload.size,
            transferred: upload.size, outgoing: true, status: "waiting".into(), error: None };
        if self.store.update_upload_with_event(room_id, &upload.file_id, "waiting", None, &waiting,
            Some((&event_id, &format!("file-offer:{}", upload.file_id), "offer", &peer, &envelope))).map_err(db_error)? {
            self.emit_in_room(room_id, crate::model::AppEvent::Transfer { transfer: waiting }).await;
        }
        self.mailbox_wake.notify_one();
        Ok(())
    }

    async fn classify_upload_error(
        &self,
        room_id: &str,
        upload: &UploadRow,
        error: MailboxError,
    ) -> std::result::Result<(), MailboxError> {
        if error.is_quota() {
            // 507 is classified as retryable by the HTTP layer, but retrying a
            // full server spins forever: treat it as a terminal, visible failure.
        } else if error.is_missing() {
            // Missing is terminal; fall through to failure below.
        } else if error.retryable() {
            return Err(error);
        }
        let message = if error.is_quota() {
            "服务器文件空间不足".to_owned()
        } else if error.is_missing() {
            "服务器文件已过期或被取消".to_owned()
        } else {
            error.to_string()
        };
        let _operation = self.operations.lock().await;
        let transfer = Transfer { id: upload.file_id.clone(), name: upload.name.clone(), size: upload.size,
            transferred: 0, outgoing: true, status: "failed".into(), error: Some(message.clone()) };
        if self.store.update_upload_with_event(room_id, &upload.file_id, "failed", Some(&message), &transfer, None).map_err(db_error)? {
            let _ = remove_dir_all(&spool_dir(&self.data_dir, room_id, &upload.file_id));
            self.emit_in_room(room_id, crate::model::AppEvent::Transfer { transfer }).await;
        }
        Ok(())
    }

    /* ---------- downloads ---------- */

    async fn download_tick(
        &self,
        client: &MailboxClient,
        room_id: &str,
        role: &str,
        seed: &str,
    ) -> std::result::Result<(), MailboxError> {
        let downloads = self.store.downloads(room_id).map_err(db_error)?;
        for download in downloads {
            match download.status.as_str() {
                "publishing" => {
                    self.recover_publish(client, room_id, &download).await?;
                    return Ok(());
                }
                "accepted" | "downloading" => {
                    self.download_chunks(client, room_id, role, seed, &download).await?;
                    return Ok(());
                }
                _ => {}
            }
        }
        Ok(())
    }

    async fn download_chunks(
        &self,
        client: &MailboxClient,
        room_id: &str,
        _role: &str,
        seed: &str,
        download: &DownloadRow,
    ) -> std::result::Result<(), MailboxError> {
        let target_dir = match download.target_dir.clone() {
            Some(dir) => PathBuf::from(dir),
            None => {
                self.fail_download(room_id, download, "下载目录缺失").await;
                return Ok(());
            }
        };
        let temp = match download.temp_path.clone() {
            Some(path) => PathBuf::from(path),
            None => {
                self.fail_download(room_id, download, "临时文件路径缺失").await;
                return Ok(());
            }
        };
        if download.next_index == 0 && !temp.exists() {
            if let Some(parent) = temp.parent() {
                let _ = fs::create_dir_all(parent);
            }
            match secure_file(&temp) {
                Ok(file) => { drop(file); }
                Err(error) => {
                    self.fail_download(room_id, download, &error.to_string()).await;
                    return Ok(());
                }
            }
        }
        let mut next = download.next_index;
        let mut offset: u64 = (0..next)
            .map(|index| crypto::chunk_plaintext_size(download.size, index).unwrap_or(0) as u64)
            .sum();
        let chunks = crypto::chunk_count(download.size).map_err(db_error)?;
        for _ in 0..CHUNKS_PER_TICK {
            if next >= chunks {
                break;
            }
            let ciphertext = match client.get_chunk(&download.file_id, next).await {
                Ok(bytes) => bytes,
                Err(error) if error.retryable() => return Err(error),
                Err(error) => {
                    let message = if error.is_missing() { "文件已过期或被对方取消" } else { "无法读取文件分块" };
                    self.fail_download(room_id, download, message).await;
                    return Ok(());
                }
            };
            let _operation = self.operations.lock().await;
            // Cancellation/rejection may have landed while the chunk was in
            // flight: never resurrect a cancelled download by appending bytes.
            match self.store.download(room_id, &download.file_id) {
                Ok(Some(row)) if matches!(row.status.as_str(), "accepted" | "downloading" | "publishing") => {}
                _ => return Ok(()),
            }
            let plaintext = match crypto::open_file_chunk(
                seed,
                room_id,
                &download.file_id,
                next,
                download.size,
                &download.hash,
                &ciphertext,
            ) {
                Ok(bytes) => bytes,
                Err(_) => {
                    let _ = fs::remove_file(&temp);
                    self.fail_download(room_id, download, "文件分块校验失败，下载已终止").await;
                    return Ok(());
                }
            };
            let current_len = fs::metadata(&temp).map(|m| m.len()).unwrap_or(0);
            if current_len != offset {
                let _ = fs::remove_file(&temp);
                self.store
                    .set_download(room_id, &download.file_id, "failed", Some(&target_dir.to_string_lossy()), Some(&temp.to_string_lossy()), 0, Some("临时文件不完整，请重新接收"))
                    .map_err(db_error)?;
                self.update_transfer(room_id, &download.file_id, "failed", 0, download.size, Some("临时文件不完整，请重新接收".into())).await;
                return Ok(());
            }
            let mut file = fs::OpenOptions::new().append(true).open(&temp).map_err(db_error)?;
            file.write_all(&plaintext).map_err(db_error)?;
            file.sync_data().map_err(db_error)?;
            drop(file);
            offset += plaintext.len() as u64;
            next += 1;
            self.store
                .set_download(room_id, &download.file_id, "downloading", Some(&target_dir.to_string_lossy()), Some(&temp.to_string_lossy()), next, None)
                .map_err(db_error)?;
            self.update_transfer(room_id, &download.file_id, "downloading", offset, download.size, None).await;
            drop(_operation);
        }
        if next < chunks {
            return Ok(());
        }
        // Verify the complete plaintext once, then publish without overwriting.
        let expected_hash = download.hash.clone();
        let temp_clone = temp.clone();
        let verified = tokio::task::spawn_blocking(move || -> Result<()> {
            let mut file = fs::File::open(&temp_clone)?;
            let mut hasher = Sha256::new();
            let mut buf = vec![0u8; 256 * 1024];
            loop {
                let read = file.read(&mut buf)?;
                if read == 0 { break; }
                hasher.update(&buf[..read]);
            }
            if hex::encode(hasher.finalize()) != expected_hash {
                bail!("文件校验失败");
            }
            Ok(())
        })
        .await
        .map_err(|error| db_error(anyhow!(error)))?;
        if let Err(error) = verified {
            let _ = fs::remove_file(&temp);
            self.fail_download(room_id, download, &format!("文件校验失败：{error}")).await;
            return Ok(());
        }
        let _operation = self.operations.lock().await;
        if !matches!(self.store.download(room_id, &download.file_id), Ok(Some(row)) if matches!(row.status.as_str(), "accepted" | "downloading")) {
            return Ok(());
        }
        let dest = match unique_destination(&target_dir, &download.name) {
            Ok(path) => path,
            Err(error) => {
                self.fail_download(room_id, download, &error.to_string()).await;
                return Ok(());
            }
        };
        // Durably record the publishing intent (final path + temp) before we
        // touch the destination so a crash mid-publish can be recovered.
        let publishing = Transfer {
            id: download.file_id.clone(),
            name: download.name.clone(),
            size: download.size,
            transferred: download.size,
            outgoing: false,
            status: "publishing".into(),
            error: None,
        };
        self.store
            .set_download_publishing(
                room_id,
                &download.file_id,
                &target_dir.to_string_lossy(),
                &temp.to_string_lossy(),
                next,
                &dest.to_string_lossy(),
                &publishing,
            )
            .map_err(db_error)?;
        // Another accept may have completed this download in the meantime.
        if let Ok(Some(row)) = self.store.download(room_id, &download.file_id) {
            if row.status == "completed" {
                let _ = fs::remove_file(&temp);
                return Ok(());
            }
        }
        if let Err(error) = publish_noclobber(&temp, &dest) {
            // A clobbered/racing destination is recoverable: keep the verified
            // temp and try a fresh unique name next tick.
            if matches!(self.store.download(room_id, &download.file_id), Ok(Some(row)) if row.status == "completed") {
                return Ok(());
            }
            let message = error.to_string();
            let transfer = Transfer { status: "failed".into(), error: Some(message.clone()), ..publishing };
            let _ = self.store.update_download_with_event(
                room_id, &download.file_id, "failed", Some(&message), &transfer, None,
            );
            self.emit_in_room(room_id, crate::model::AppEvent::Transfer { transfer }).await;
            return Ok(());
        }
        let (role, peer, seed) = self.current_identity(room_id).await.map_err(db_error)?;
        let event_id = Uuid::new_v4().to_string();
        let plaintext = Body::FileComplete { id: download.file_id.clone() }.encode().map_err(db_error)?;
        let envelope = crypto::seal_event(&seed, room_id, &role, &peer, &event_id, &plaintext).map_err(db_error)?;
        let completed = Transfer {
            id: download.file_id.clone(),
            name: download.name.clone(),
            size: download.size,
            transferred: download.size,
            outgoing: false,
            status: "completed".into(),
            error: None,
        };
        // One transaction: completion + receipt outbox. If it fails we keep the
        // published final file (its path is durable) and retry next tick.
        self.store
            .complete_download(
                room_id,
                &download.file_id,
                &dest.to_string_lossy(),
                &event_id,
                &format!("file-complete:{}", download.file_id),
                &peer,
                &envelope,
                &completed,
            )
            .map_err(db_error)?;
        let _ = fs::remove_file(&temp);
        self.emit_in_room(room_id, crate::model::AppEvent::Transfer {
            transfer: self.store.transfer(room_id, &download.file_id).map_err(db_error)?.unwrap_or(completed),
        })
        .await;
        self.mailbox_wake.notify_one();
        drop(_operation);
        let _ = client.delete_file(&download.file_id).await;
        Ok(())
    }

    /// Resume a download that crashed between publishing the final file and
    /// recording completion. `final_path` was written before the rename, so the
    /// destination we created is authoritative even if the DB row is stale.
    async fn recover_publish(
        &self,
        client: &MailboxClient,
        room_id: &str,
        download: &DownloadRow,
    ) -> std::result::Result<(), MailboxError> {
        let final_path = match download.final_path.clone() {
            Some(path) => PathBuf::from(path),
            None => {
                let transfer = Transfer {
                    id: download.file_id.clone(),
                    name: download.name.clone(),
                    size: download.size,
                    transferred: 0,
                    outgoing: false,
                    status: "failed".into(),
                    error: Some("发布状态缺失，请重新接收".into()),
                };
                let _ = self.store.update_download_with_event(
                    room_id, &download.file_id, "failed", Some("发布状态缺失，请重新接收"), &transfer, None,
                );
                self.emit_in_room(room_id, crate::model::AppEvent::Transfer { transfer }).await;
                return Ok(());
            }
        };
        let expected_hash = download.hash.clone();
        let check = final_path.clone();
        let verified = tokio::task::spawn_blocking(move || -> Result<()> {
            let mut file = fs::File::open(&check)?;
            let mut hasher = Sha256::new();
            let mut buf = vec![0u8; 256 * 1024];
            loop {
                let read = file.read(&mut buf)?;
                if read == 0 { break; }
                hasher.update(&buf[..read]);
            }
            if hex::encode(hasher.finalize()) != expected_hash {
                bail!("文件校验失败");
            }
            Ok(())
        })
        .await
        .map_err(|error| db_error(anyhow!(error)))?;
        if verified.is_err() {
            let transfer = Transfer {
                id: download.file_id.clone(),
                name: download.name.clone(),
                size: download.size,
                transferred: 0,
                outgoing: false,
                status: "failed".into(),
                error: Some("文件校验失败".into()),
            };
            let _ = self.store.update_download_with_event(
                room_id, &download.file_id, "failed", Some("文件校验失败"), &transfer, None,
            );
            self.emit_in_room(room_id, crate::model::AppEvent::Transfer { transfer }).await;
            return Ok(());
        }
        let (role, peer, seed) = match self.current_identity(room_id).await {
            Ok(values) => values,
            Err(_) => return Ok(()),
        };
        let event_id = Uuid::new_v4().to_string();
        let plaintext = Body::FileComplete { id: download.file_id.clone() }
            .encode()
            .map_err(db_error)?;
        let envelope = crypto::seal_event(&seed, room_id, &role, &peer, &event_id, &plaintext)
            .map_err(db_error)?;
        let completed = Transfer {
            id: download.file_id.clone(),
            name: download.name.clone(),
            size: download.size,
            transferred: download.size,
            outgoing: false,
            status: "completed".into(),
            error: None,
        };
        self.store
            .complete_download(
                room_id,
                &download.file_id,
                &final_path.to_string_lossy(),
                &event_id,
                &format!("file-complete:{}", download.file_id),
                &peer,
                &envelope,
                &completed,
            )
            .map_err(db_error)?;
        if let Some(temp) = download.temp_path.as_ref() {
            let _ = fs::remove_file(temp);
        }
        self.emit_in_room(room_id, crate::model::AppEvent::Transfer { transfer: completed }).await;
        self.mailbox_wake.notify_one();
        let _ = client.delete_file(&download.file_id).await;
        Ok(())
    }

    async fn fail_download(&self, room_id: &str, download: &DownloadRow, message: &str) {
        let transfer = Transfer {
            id: download.file_id.clone(),
            name: download.name.clone(),
            size: download.size,
            transferred: 0,
            outgoing: false,
            status: "failed".into(),
            error: Some(message.into()),
        };
        let saved = self.store.update_download_with_event(
            room_id,
            &download.file_id,
            "failed",
            Some(message),
            &transfer,
            None,
        );
        if matches!(saved, Ok(true)) { self.emit_in_room(room_id, crate::model::AppEvent::Transfer { transfer }).await; }
    }

    async fn current_identity(&self, room_id: &str) -> Result<(String, String, String)> {
        let inner = self.inner.lock().await;
        let spec = inner.room.as_ref().context("房间已切换")?.spec()?;
        if spec.room_id != room_id {
            bail!("房间已切换");
        }
        let peer = peer_role(&spec.stored.role)?.to_owned();
        Ok((spec.stored.role, peer, spec.stored.seed))
    }

    pub(crate) async fn update_transfer(
        &self,
        room_id: &str,
        id: &str,
        status: &str,
        transferred: u64,
        size: u64,
        error: Option<String>,
    ) {
        let mut transfer = match self.store.transfer(room_id, id) {
            Ok(Some(transfer)) => transfer,
            _ => Transfer {
                id: id.into(),
                name: String::new(),
                size,
                transferred,
                outgoing: true,
                status: status.into(),
                error: None,
            },
        };
        transfer.status = status.into();
        transfer.transferred = transferred;
        transfer.size = size;
        transfer.error = error;
        // Never paint a state we failed to persist: a lost save would resurrect
        // stale data on the next snapshot and mislead the user.
        if self.store.save_transfer(room_id, &transfer).is_err() {
            return;
        }
        self.emit_in_room(room_id, crate::model::AppEvent::Transfer { transfer }).await;
    }

    /* ---------- user actions used by lib.rs ---------- */

    pub(crate) async fn mailbox_send_message(&self, text: String) -> Result<Message> {
        let (room_id, role, peer, seed, has_peer) = {
            let inner = self.inner.lock().await;
            let room = inner.room.clone().context("还没有保存的房间，请创建或加入房间")?;
            let spec = room.spec()?;
            let peer = peer_role(&spec.stored.role)?.to_owned();
            (spec.room_id, spec.stored.role, peer, spec.stored.seed, inner.sender.is_some())
        };
        let message = Message {
            id: Uuid::new_v4().to_string(),
            text: text.clone(),
            outgoing: true,
            timestamp: now(),
            delivery: "pending".into(),
            via: None,
        };
        let plaintext = Body::Message {
            id: message.id.clone(),
            text,
            timestamp: message.timestamp,
        }
        .encode()?;
        let envelope = crypto::seal_event(&seed, &room_id, &role, &peer, &message.id, &plaintext)?;
        let due = now() + if has_peer { DIRECT_GRACE_MS } else { 0 };
        // One transaction: the visible message always has a sendable encrypted
        // copy, so quitting immediately after send cannot lose it.
        self.store.enqueue_message(
            &room_id,
            &message,
            &message.id,
            &message.id,
            &peer,
            &envelope,
            due,
        )?;
        self.emit_in_room(&room_id, crate::model::AppEvent::Message { message: message.clone() }).await;
        self.refresh_mailbox_pending(&room_id).await;
        self.mailbox_wake.notify_one();
        Ok(message)
    }

    pub(crate) async fn mailbox_offer_file(&self, path: PathBuf) -> Result<Transfer> {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .context("文件名不是有效 UTF-8")?
            .to_owned();
        validate_name(&name)?;
        let metadata = tokio::fs::metadata(&path).await.context("无法读取所选文件")?;
        if !metadata.is_file() {
            bail!("请选择普通文件");
        }
        if metadata.len() > MAX_FILE_SIZE {
            bail!("单个文件不能超过 1 GiB");
        }
        let selection = self.inner.lock().await.room.clone().context("还没有保存的房间，请创建或加入房间")?;
        let room_id = selection.spec()?.room_id;
        let (size, hash) = hash_file(&path).await?;
        let id = Uuid::new_v4().to_string();
        let encrypted_size = crypto::encrypted_size(size)?;
        let chunk_count = crypto::chunk_count(size)?;
        let upload = UploadRow {
            room: room_id.clone(),
            file_id: id.clone(),
            source_path: path.to_string_lossy().into_owned(),
            name: name.clone(),
            size,
            hash,
            encrypted_size,
            chunk_count,
            status: "pending".into(),
            error: None,
            created_at: now(),
        };
        let transfer = Transfer {
            id: id.clone(),
            name,
            size,
            transferred: 0,
            outgoing: true,
            status: "preparing".into(),
            error: None,
        };
        let _operation = self.operations.lock().await;
        anyhow::ensure!(self.inner.lock().await.room.as_ref() == Some(&selection), "房间已改变，请重新选择发送");
        self.store.insert_upload(&upload, &transfer)?;
        self.emit_in_room(&room_id, crate::model::AppEvent::Transfer { transfer: transfer.clone() }).await;
        self.mailbox_wake.notify_one();
        Ok(transfer)
    }

    pub(crate) async fn mailbox_accept_file(&self, id: Uuid) -> Result<()> {
        let room_id = self.current_room_id().await?;
        let download = self.store.download(&room_id, &id.to_string())?
            .context("未知的离线文件请求")?;
        if download.status != "offered" && download.status != "failed" {
            bail!("该文件当前不能接收");
        }
        let target = {
            let inner = self.inner.lock().await;
            PathBuf::from(&inner.profile.download_dir)
        };
        if !target.is_absolute() || !target.is_dir() {
            bail!("下载目录不可用，请在设置中重新选择");
        }
        let target = fs::canonicalize(target).context("下载目录不可用")?;
        if let Some(previous) = download.temp_path.as_ref() { let _ = fs::remove_file(previous); }
        let temp = target.join(format!(".twinlink-{}-{}.part", download.file_id, rand::thread_rng().next_u64()));
        self.store.set_download(
            &room_id,
            &download.file_id,
            "accepted",
            Some(&target.to_string_lossy()),
            Some(&temp.to_string_lossy()),
            0,
            None,
        )?;
        self.update_transfer(&room_id, &download.file_id, "accepted", 0, download.size, None).await;
        self.mailbox_wake.notify_one();
        Ok(())
    }

    pub(crate) async fn mailbox_cancel_file(&self, id: Uuid, rejected: bool) -> Result<()> {
        let room_id = self.current_room_id().await?;
        let id = id.to_string();
        let status = if rejected { "rejected" } else { "cancelled" };
        let (role, peer, seed) = self.current_identity(&room_id).await?;
        let event_id = Uuid::new_v4().to_string();
        let body = if rejected { Body::Reject { id: id.clone() } } else { Body::Cancel { id: id.clone() } };
        let envelope = crypto::seal_event(&seed, &room_id, &role, &peer, &event_id, &body.encode()?)?;
        let dedupe = format!("file-{}-{id}", if rejected { "reject" } else { "cancel" });
        let mut transfer = self.store.transfer(&room_id, &id)?.context("未知的文件传输")?;
        transfer.status = status.into();
        transfer.error = None;
        let event = Some((event_id.as_str(), dedupe.as_str(), "fileAction", peer.as_str(), envelope.as_slice()));
        if let Some(download) = self.store.download(&room_id, &id)? {
            if matches!(download.status.as_str(), "completed" | "publishing") {
                bail!("文件已保存或正在保存，无需取消");
            }
            if !self.store.update_download_with_event(&room_id, &id, status, None, &transfer, event)? { return Ok(()); }
            if let Some(path) = download.temp_path { let _ = fs::remove_file(path); }
        } else if let Some(upload) = self.store.upload(&room_id, &id)? {
            if rejected { bail!("只能拒收对方发送的文件"); }
            if upload.status == "completed" { bail!("对方已保存文件，无法撤回"); }
            if !self.store.update_upload_with_event(&room_id, &id, status, None, &transfer, event)? { return Ok(()); }
            let _ = remove_dir_all(&spool_dir(&self.data_dir, &room_id, &id));
        } else { bail!("未知的文件传输"); }
        self.emit_in_room(&room_id, crate::model::AppEvent::Transfer { transfer }).await;
        self.refresh_mailbox_pending(&room_id).await;
        self.mailbox_wake.notify_one();
        Ok(())
    }

    pub(crate) async fn current_room_id(&self) -> Result<String> {
        let inner = self.inner.lock().await;
        Ok(inner.room.as_ref().context("还没有保存的房间，请创建或加入房间")?.spec()?.room_id)
    }


}

fn backoff(failures: u32) -> Duration {
    let exponent = failures.saturating_sub(1).min(16);
    let millis = 500u64
        .saturating_mul(1u64 << exponent)
        .min(RETRY_CAP.as_millis() as u64);
    Duration::from_millis(millis / 2 + rand::random::<u64>() % (millis / 2).max(1))
}

fn db_error(error: impl std::fmt::Display) -> MailboxError {
    MailboxError { status: None, code: "local".into(), message: format!("本地存储失败：{error}") }
}

fn base64_decode(value: &str) -> std::result::Result<Vec<u8>, MailboxError> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| MailboxError { status: None, code: "protocol".into(), message: "离线消息编码无效".into() })
}

fn spool_dir(data_dir: &Path, room: &str, file_id: &str) -> PathBuf {
    data_dir.join("mailbox-spool").join(room).join(file_id)
}

fn secure_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn secure_file(path: &Path) -> Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}

fn remove_dir_all(path: &Path) -> Result<()> {
    if path.exists() {
        fs::remove_dir_all(path)?;
    }
    Ok(())
}

fn unique_destination(dir: &Path, name: &str) -> Result<PathBuf> {
    validate_name(name)?;
    let first = dir.join(name);
    if !first.exists() {
        return Ok(first);
    }
    let path = Path::new(name);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let ext = path.extension().and_then(|s| s.to_str());
    for number in 1..=9999 {
        let candidate = match ext {
            Some(ext) => dir.join(format!("{stem} ({number}).{ext}")),
            None => dir.join(format!("{stem} ({number})")),
        };
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    bail!("下载目录中同名文件过多")
}

/// Publish by creating the destination exclusively, so a racing file or a
/// pre-existing file is never overwritten.
/// Atomically place the verified temp file at `dest` without ever overwriting
/// an existing file and without exposing a partially written destination.
///
/// The temp file is already fully written and fsynced in the same directory as
/// the destination. We hard-link it to `dest`; `link(2)` fails with `EEXIST`
/// if a racing file appeared, which is exactly the no-clobber guarantee we
/// want, and it never makes a half-copied destination visible. The parent
/// directory is fsynced so the new name survives a crash.
fn publish_noclobber(temp: &Path, dest: &Path) -> Result<()> {
    // Refuse to traverse a symlink: linking onto a symlink path would follow it.
    if let Ok(meta) = fs::symlink_metadata(dest) {
        if meta.file_type().is_symlink() {
            bail!("目标路径是符号链接，已拒绝写入");
        }
        bail!("同名文件已存在，未覆盖");
    }
    if let Some(parent) = dest.parent() {
        if fs::symlink_metadata(parent)
            .map(|meta| meta.file_type().is_symlink())
            .unwrap_or(false)
        {
            bail!("下载目录是符号链接，已拒绝写入");
        }
    }
    match fs::hard_link(temp, dest) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            bail!("同名文件已存在，未覆盖");
        }
        Err(error) => return Err(error).context("发布下载文件失败"),
    }
    sync_parent(dest)?;
    Ok(())
}

fn sync_parent(path: &Path) -> Result<()> {
    #[cfg(unix)]
    if let Some(parent) = path.parent() {
        fs::File::open(parent)?.sync_all()?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}
