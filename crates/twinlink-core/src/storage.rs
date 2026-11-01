use crate::model::{now, Message, Profile, Transfer};
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use std::{path::Path, sync::Mutex};

pub struct Store(Mutex<Connection>);

fn delivery_rank(value: &str) -> u8 {
    match value {
        "pending" => 0,
        "unknown" => 1,
        "stored" => 2,
        "delivered" => 3,
        _ => 0,
    }
}

fn save_message_tx(tx: &rusqlite::Transaction<'_>, room: &str, message: &Message) -> Result<()> {
    tx.execute(
        "INSERT OR IGNORE INTO messages(room,id,timestamp,value) VALUES(?1,?2,?3,?4)",
        params![room, message.id, message.timestamp, serde_json::to_string(message)?],
    )?;
    Ok(())
}

fn save_transfer_tx(tx: &rusqlite::Transaction<'_>, room: &str, transfer: &Transfer) -> Result<()> {
    tx.execute(
        "INSERT INTO transfers(room,id,value) VALUES(?1,?2,?3) ON CONFLICT(room,id) DO UPDATE SET value=excluded.value",
        params![room, transfer.id, serde_json::to_string(transfer)?],
    )?;
    Ok(())
}

fn enqueue_event_tx(
    tx: &rusqlite::Transaction<'_>,
    room: &str,
    event_id: &str,
    dedupe: &str,
    kind: &str,
    recipient: &str,
    envelope: &[u8],
    due_at: u64,
) -> Result<()> {
    tx.execute(
        "INSERT OR IGNORE INTO outbox(room,event_id,dedupe,kind,recipient,envelope,created_at,next_attempt)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        params![room, event_id, dedupe, kind, recipient, envelope, now(), due_at],
    )?;
    Ok(())
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let mut conn = Connection::open(path)?;
        // Chat history and transfer metadata live here; keep it owner-only on
        // Unix like the rest of the app data directory.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
        // Connection pragmas are independent of schema changes. Never reset a
        // version marker before checking it, and never leave a partial schema
        // committed if a later statement fails.
        conn.execute_batch("PRAGMA busy_timeout=3000; PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;")?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let version: i64 = tx.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        anyhow::ensure!(version <= 3, "数据库版本较新，请升级应用后再打开");
        tx.execute_batch(include_str!("../migrations/001_schema.sql"))?;
        // Existing phase-2 databases may predate the publication journal. A
        // CREATE TABLE IF NOT EXISTS cannot add their missing column.
        let columns = {
            let mut stmt = tx.prepare("PRAGMA table_info(downloads)")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        if !columns.iter().any(|name| name == "final_path") {
            tx.execute_batch("ALTER TABLE downloads ADD COLUMN final_path TEXT;")?;
        }
        tx.commit()?;
        Ok(Self(Mutex::new(conn)))
    }

    pub fn profile(&self, default: Profile) -> Result<Profile> {
        let conn = self.0.lock().unwrap();
        let mut stmt = conn.prepare("SELECT value FROM settings WHERE key = 'profile'")?;
        let mut rows = stmt.query([])?;
        if let Some(row) = rows.next()? {
            let value: String = row.get(0)?;
            Ok(serde_json::from_str(&value)?)
        } else { Ok(default) }
    }

    pub fn save_profile(&self, profile: &Profile) -> Result<()> {
        self.0.lock().unwrap().execute(
            "INSERT INTO settings(key,value) VALUES('profile',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [serde_json::to_string(profile)?],
        )?;
        Ok(())
    }

    pub fn save_message(&self, room: &str, message: &Message) -> Result<()> {
        self.0.lock().unwrap().execute(
            "INSERT OR IGNORE INTO messages(room,id,timestamp,value) VALUES(?1,?2,?3,?4)",
            params![room, message.id, message.timestamp, serde_json::to_string(message)?],
        )?;
        Ok(())
    }

    pub fn messages(&self, room: &str) -> Result<Vec<Message>> {
        Ok(self.message_page(room, None)?.messages)
    }

    pub fn remember_history_room(&self, room: &str) -> Result<()> {
        self.0.lock().unwrap().execute(
            "INSERT INTO settings(key,value) VALUES('history_room',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [room],
        )?;
        Ok(())
    }

    pub fn history_room(&self) -> Result<Option<String>> {
        let conn = self.0.lock().unwrap();
        let selected: Option<String> = conn.query_row("SELECT value FROM settings WHERE key='history_room'", [], |r| r.get(0)).optional()?;
        if selected.is_some() { return Ok(selected); }
        conn.query_row("SELECT room FROM messages ORDER BY timestamp DESC,rowid DESC LIMIT 1", [], |r| r.get(0)).optional().map_err(Into::into)
    }

    pub fn message_page(&self, room: &str, before: Option<&str>) -> Result<crate::model::HistoryPage> {
        let conn = self.0.lock().unwrap();
        let cursor = match before {
            Some(id) => Some(conn.query_row("SELECT timestamp,rowid FROM messages WHERE room=?1 AND id=?2", params![room,id], |r| Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?)))
                .optional()?.ok_or_else(||anyhow::anyhow!("记录位置已失效，请重新打开会话"))?),
            None => None,
        };
        let mut stmt = conn.prepare("SELECT value FROM messages WHERE room=?1 AND (?2 IS NULL OR timestamp<?3 OR (timestamp=?3 AND rowid<?4)) ORDER BY timestamp DESC,rowid DESC LIMIT 201")?;
        let raw = stmt.query_map(params![room,before,cursor.map(|x|x.0).unwrap_or(0),cursor.map(|x|x.1).unwrap_or(0)], |row| row.get::<_, String>(0))?;
        let mut messages = raw.map(|item| Ok(serde_json::from_str(&item?)?)).collect::<Result<Vec<Message>>>()?;
        let has_more = messages.len() > 200;
        messages.truncate(200);
        messages.reverse();
        Ok(crate::model::HistoryPage { room_id: room.into(), messages, has_more })
    }

    /// Atomically persist the local message and its exact encrypted outbox
    /// envelope. A crash can never expose a message with no sendable copy.
    pub fn enqueue_message(
        &self,
        room: &str,
        message: &Message,
        event_id: &str,
        dedupe: &str,
        recipient: &str,
        envelope: &[u8],
        due_at: u64,
    ) -> Result<()> {
        let mut conn = self.0.lock().unwrap();
        let tx = conn.transaction()?;
        save_message_tx(&tx, room, message)?;
        enqueue_event_tx(&tx, room, event_id, dedupe, "message", recipient, envelope, due_at)?;
        tx.commit()?;
        Ok(())
    }

    pub fn save_transfer(&self, room: &str, transfer: &Transfer) -> Result<()> {
        self.0.lock().unwrap().execute(
            "INSERT INTO transfers(room,id,value) VALUES(?1,?2,?3) ON CONFLICT(room,id) DO UPDATE SET value=excluded.value",
            params![room, transfer.id, serde_json::to_string(transfer)?],
        )?;
        Ok(())
    }

    pub fn transfers(&self, room: &str) -> Result<Vec<Transfer>> {
        let conn = self.0.lock().unwrap();
        let mut stmt = conn.prepare("SELECT value FROM transfers WHERE room=?1 ORDER BY rowid ASC")?;
        let raw = stmt.query_map([room], |row| row.get::<_, String>(0))?;
        raw.map(|item| Ok(serde_json::from_str(&item?)?)).collect()
    }

    pub fn transfer(&self, room: &str, id: &str) -> Result<Option<Transfer>> {
        let conn = self.0.lock().unwrap();
        let raw: Option<String> = conn
            .query_row(
                "SELECT value FROM transfers WHERE room=?1 AND id=?2",
                params![room, id],
                |row| row.get(0),
            )
            .optional()?;
        raw.map(|raw| serde_json::from_str(&raw).map_err(Into::into)).transpose()
    }

    pub fn message(&self, room: &str, id: &str) -> Result<Option<Message>> {
        let raw: Option<String> = self.0.lock().unwrap()
            .query_row(
                "SELECT value FROM messages WHERE room=?1 AND id=?2",
                params![room, id],
                |row| row.get(0),
            )
            .optional()?;
        raw.map(|raw| serde_json::from_str(&raw).map_err(Into::into)).transpose()
    }

    pub fn has_message(&self, room: &str, id: &str) -> Result<bool> {
        let count: i64 = self.0.lock().unwrap().query_row(
            "SELECT COUNT(*) FROM messages WHERE room=?1 AND id=?2",
            params![room, id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Delivery is monotonic: a delayed mailbox "stored" acknowledgement can
    /// never overwrite a direct-session "delivered" receipt.
    #[cfg(test)]
    pub fn set_message_delivery(&self, room: &str, id: &str, delivery: &str, via: Option<&str>) -> Result<Option<Message>> {
        let conn = self.0.lock().unwrap();
        let raw: Option<String> = conn
            .query_row(
                "SELECT value FROM messages WHERE room=?1 AND id=?2",
                params![room, id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(raw) = raw else { return Ok(None) };
        let mut message: Message = serde_json::from_str(&raw)?;
        if delivery_rank(delivery) <= delivery_rank(&message.delivery) {
            return Ok(None);
        }
        message.delivery = delivery.to_owned();
        if let Some(via) = via {
            message.via = Some(via.to_owned());
        }
        conn.execute(
            "UPDATE messages SET value=?3 WHERE room=?1 AND id=?2",
            params![room, id, serde_json::to_string(&message)?],
        )?;
        Ok(Some(message))
    }

    /// Commit a peer receipt: the message becomes delivered, the now-useless
    /// mailbox copy is removed and the event is marked processed in one go.
    pub fn apply_receipt(&self, room: &str, message_id: &str, event_id: Option<&str>, via: &str) -> Result<Option<Message>> {
        let mut conn = self.0.lock().unwrap();
        let tx = conn.transaction()?;
        let raw: Option<String> = tx
            .query_row(
                "SELECT value FROM messages WHERE room=?1 AND id=?2",
                params![room, message_id],
                |row| row.get(0),
            )
            .optional()?;
        let mut changed = None;
        if let Some(raw) = raw {
            let mut message: Message = serde_json::from_str(&raw)?;
            if delivery_rank(&message.delivery) < delivery_rank("delivered") {
                message.delivery = "delivered".into();
                message.via = Some(via.into());
                tx.execute(
                    "UPDATE messages SET value=?3 WHERE room=?1 AND id=?2",
                    params![room, message_id, serde_json::to_string(&message)?],
                )?;
                changed = Some(message);
            }
        }
        tx.execute(
            "DELETE FROM outbox WHERE room=?1 AND (dedupe=?2 OR dedupe=?3)",
            params![room, message_id, format!("message:{message_id}")],
        )?;
        if let Some(event_id) = event_id { tx.execute(
            "INSERT OR IGNORE INTO seen_events(room,event_id,processed_at) VALUES(?1,?2,?3)",
            params![room, event_id, now()],
        )?; }
        tx.commit()?;
        Ok(changed)
    }

    /* ---------- encrypted outbox ---------- */

    #[allow(clippy::too_many_arguments)]
    pub fn enqueue_event(
        &self,
        room: &str,
        event_id: &str,
        dedupe: &str,
        kind: &str,
        recipient: &str,
        envelope: &[u8],
        due_at: u64,
    ) -> Result<()> {
        self.0.lock().unwrap().execute(
            "INSERT OR IGNORE INTO outbox(room,event_id,dedupe,kind,recipient,envelope,created_at,next_attempt)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![room, event_id, dedupe, kind, recipient, envelope, now(), due_at],
        )?;
        Ok(())
    }

    pub fn due_events(&self, room: &str, at: u64, limit: usize) -> Result<Vec<OutboxRow>> {
        let conn = self.0.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT event_id,kind,dedupe,envelope,attempts FROM outbox
             WHERE room=?1 AND state='pending' AND next_attempt<=?2
             ORDER BY CASE WHEN kind IN('receipt','fileComplete','fileAction') THEN 0 ELSE 1 END,created_at ASC LIMIT ?3",
        )?;
        let raw = stmt.query_map(params![room, at, limit as i64], |row| {
            Ok(OutboxRow {
                event_id: row.get(0)?,
                kind: row.get(1)?,
                dedupe: row.get(2)?,
                envelope: row.get(3)?,
                attempts: row.get(4)?,
            })
        })?;
        raw.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
    }

    pub fn pending_outbox(&self, room: &str) -> Result<usize> {
        let count: i64 = self.0.lock().unwrap().query_row(
            "SELECT COUNT(*) FROM outbox WHERE room=?1 AND state='pending'",
            [room],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// A chat event was accepted by the mailbox. Promote the persisted message
    /// to `stored` in the same transaction so the UI cannot claim delivery.
    pub fn mark_message_stored(&self, room: &str, event_id: &str) -> Result<Option<Message>> {
        let mut conn = self.0.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE outbox SET state='stored', next_attempt=0, last_error=NULL WHERE room=?1 AND event_id=?2",
            params![room, event_id],
        )?;
        let raw: Option<String> = tx
            .query_row(
                "SELECT value FROM messages WHERE room=?1 AND id=?2",
                params![room, event_id],
                |row| row.get(0),
            )
            .optional()?;
        let mut changed = None;
        if let Some(raw) = raw {
            let mut message: Message = serde_json::from_str(&raw)?;
            if delivery_rank(&message.delivery) < delivery_rank("stored") {
                message.delivery = "stored".into();
                message.via = Some("mailbox".into());
                tx.execute(
                    "UPDATE messages SET value=?3 WHERE room=?1 AND id=?2",
                    params![room, event_id, serde_json::to_string(&message)?],
                )?;
                changed = Some(message);
            }
        }
        tx.commit()?;
        Ok(changed)
    }

    /// Non-message outbox rows are deleted once the server committed them:
    /// the server keeps the durable copy, and keeping a `stored` row would
    /// suppress a later, legitimately new receipt with the same dedupe key.
    pub fn delete_event(&self, room: &str, event_id: &str) -> Result<()> {
        self.0.lock().unwrap().execute(
            "DELETE FROM outbox WHERE room=?1 AND event_id=?2",
            params![room, event_id],
        )?;
        Ok(())
    }

    pub fn reschedule_event(&self, room: &str, event_id: &str, at: u64, error: &str) -> Result<()> {
        self.0.lock().unwrap().execute(
            "UPDATE outbox SET next_attempt=?3, attempts=attempts+1, last_error=?4 WHERE room=?1 AND event_id=?2",
            params![room, event_id, at, error],
        )?;
        Ok(())
    }

    pub fn fail_event(&self, room: &str, event_id: &str, error: &str) -> Result<()> {
        self.0.lock().unwrap().execute(
            "UPDATE outbox SET state='failed', last_error=?3 WHERE room=?1 AND event_id=?2",
            params![room, event_id, error],
        )?;
        Ok(())
    }

    pub fn has_seen_event(&self, room: &str, event_id: &str) -> Result<bool> {
        let count: i64 = self.0.lock().unwrap().query_row(
            "SELECT COUNT(*) FROM seen_events WHERE room=?1 AND event_id=?2",
            params![room, event_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    pub fn mark_seen_event(&self, room: &str, event_id: &str) -> Result<()> {
        self.0.lock().unwrap().execute(
            "INSERT OR IGNORE INTO seen_events(room,event_id,processed_at) VALUES(?1,?2,?3)",
            params![room, event_id, now()],
        )?;
        Ok(())
    }

    pub fn has_quarantined_event(&self, room: &str, event_id: &str) -> Result<bool> {
        let count: i64 = self.0.lock().unwrap().query_row(
            "SELECT COUNT(*) FROM quarantined_events WHERE room=?1 AND event_id=?2",
            params![room, event_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// An unauthenticated or malformed envelope is retained for diagnostics and
    /// deliberately not acknowledged, so the server keeps its durable copy.
    pub fn quarantine_event(
        &self,
        room: &str,
        event: &crate::mailbox::IncomingEvent,
        error: &str,
    ) -> Result<()> {
        self.0.lock().unwrap().execute(
            "INSERT OR IGNORE INTO quarantined_events(room,event_id,sender,recipient,payload,error,quarantined_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![room, event.id, event.sender, event.recipient,
                base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, &event.payload),
                error, now()],
        )?;
        Ok(())
    }

    /* ---------- mailbox file metadata ---------- */

    #[allow(clippy::too_many_arguments)]
    pub fn insert_upload(&self, upload: &UploadRow, transfer: &Transfer) -> Result<()> {
        let mut conn = self.0.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO uploads(room,file_id,source_path,name,size,hash,encrypted_size,chunk_count,status,error,created_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![upload.room, upload.file_id, upload.source_path, upload.name, upload.size,
                upload.hash, upload.encrypted_size, upload.chunk_count, upload.status, upload.error, upload.created_at],
        )?;
        save_transfer_tx(&tx, &upload.room, transfer)?;
        tx.commit()?;
        Ok(())
    }

    pub fn uploads(&self, room: &str) -> Result<Vec<UploadRow>> {
        let conn = self.0.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT room,file_id,source_path,name,size,hash,encrypted_size,chunk_count,status,error,created_at
             FROM uploads WHERE room=?1 ORDER BY created_at ASC",
        )?;
        let raw = stmt.query_map([room], upload_from_row)?;
        raw.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
    }

    pub fn upload(&self, room: &str, file_id: &str) -> Result<Option<UploadRow>> {
        self.0.lock().unwrap().query_row(
            "SELECT room,file_id,source_path,name,size,hash,encrypted_size,chunk_count,status,error,created_at
             FROM uploads WHERE room=?1 AND file_id=?2",
            params![room, file_id],
            upload_from_row,
        ).optional().map_err(Into::into)
    }

    pub fn set_upload_status(&self, room: &str, file_id: &str, status: &str, error: Option<&str>) -> Result<()> {
        self.0.lock().unwrap().execute(
            "UPDATE uploads SET status=?3, error=?4 WHERE room=?1 AND file_id=?2",
            params![room, file_id, status, error],
        )?;
        Ok(())
    }

    /// Terminal upload states are final; a late `fileComplete` must never turn
    /// a cancelled/failed transfer into a success.
    pub fn complete_upload(&self, room: &str, file_id: &str, transfer: &Transfer) -> Result<bool> {
        let mut conn = self.0.lock().unwrap();
        let tx = conn.transaction()?;
        let status: Option<String> = tx
            .query_row(
                "SELECT status FROM uploads WHERE room=?1 AND file_id=?2",
                params![room, file_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(status) = status else { return Ok(false) };
        if status != "waiting" {
            return Ok(false);
        }
        tx.execute(
            "UPDATE uploads SET status='completed', error=NULL WHERE room=?1 AND file_id=?2",
            params![room, file_id],
        )?;
        save_transfer_tx(&tx, room, transfer)?;
        tx.commit()?;
        Ok(true)
    }

    /// Insert the offer and its UI transfer row together so a crash cannot
    /// produce a pending consent prompt with no durable metadata.
    pub fn insert_offer(&self, download: &DownloadRow, transfer: &Transfer) -> Result<()> {
        let mut conn = self.0.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT OR IGNORE INTO downloads(room,file_id,sender,name,size,hash,status,target_dir,temp_path,next_index,error,created_at,final_path)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,NULL)",
            params![download.room, download.file_id, download.sender, download.name, download.size,
                download.hash, download.status, download.target_dir, download.temp_path,
                download.next_index, download.error, download.created_at],
        )?;
        save_transfer_tx(&tx, &download.room, transfer)?;
        tx.commit()?;
        Ok(())
    }

    pub fn downloads(&self, room: &str) -> Result<Vec<DownloadRow>> {
        let conn = self.0.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT room,file_id,sender,name,size,hash,status,target_dir,temp_path,next_index,error,created_at,final_path
             FROM downloads WHERE room=?1 ORDER BY created_at ASC",
        )?;
        let raw = stmt.query_map([room], download_from_row)?;
        raw.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
    }

    pub fn download(&self, room: &str, file_id: &str) -> Result<Option<DownloadRow>> {
        self.0.lock().unwrap().query_row(
            "SELECT room,file_id,sender,name,size,hash,status,target_dir,temp_path,next_index,error,created_at,final_path
             FROM downloads WHERE room=?1 AND file_id=?2",
            params![room, file_id],
            download_from_row,
        ).optional().map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_download(
        &self,
        room: &str,
        file_id: &str,
        status: &str,
        target_dir: Option<&str>,
        temp_path: Option<&str>,
        next_index: u64,
        error: Option<&str>,
    ) -> Result<()> {
        self.0.lock().unwrap().execute(
            "UPDATE downloads SET status=?3,target_dir=?4,temp_path=?5,next_index=?6,error=?7
             WHERE room=?1 AND file_id=?2",
            params![room, file_id, status, target_dir, temp_path, next_index, error],
        )?;
        Ok(())
    }

    pub fn set_download_publishing(
        &self,
        room: &str,
        file_id: &str,
        target_dir: &str,
        temp_path: &str,
        next_index: u64,
        final_path: &str,
        transfer: &Transfer,
    ) -> Result<()> {
        let mut conn = self.0.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE downloads SET status='publishing',target_dir=?3,temp_path=?4,next_index=?5,error=NULL,final_path=?6
             WHERE room=?1 AND file_id=?2",
            params![room, file_id, target_dir, temp_path, next_index, final_path],
        )?;
        save_transfer_tx(&tx, room, transfer)?;
        tx.commit()?;
        Ok(())
    }

    /// Atomically mark the download complete and enqueue its receipt event.
    #[allow(clippy::too_many_arguments)]
    pub fn complete_download(
        &self,
        room: &str,
        file_id: &str,
        final_path: &str,
        event_id: &str,
        dedupe: &str,
        recipient: &str,
        envelope: &[u8],
        transfer: &Transfer,
    ) -> Result<()> {
        let mut conn = self.0.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE downloads SET status='completed',error=NULL,final_path=?3
             WHERE room=?1 AND file_id=?2",
            params![room, file_id, final_path],
        )?;
        save_transfer_tx(&tx, room, transfer)?;
        enqueue_event_tx(&tx, room, event_id, dedupe, "fileComplete", recipient, envelope, now())?;
        tx.commit()?;
        Ok(())
    }

    /// Change a transfer's durable status and enqueue an optional control event
    /// in one transaction. Used by accept/reject/cancel.
    pub fn update_download_with_event(
        &self,
        room: &str,
        file_id: &str,
        status: &str,
        error: Option<&str>,
        transfer: &Transfer,
        event: Option<(&str, &str, &str, &str, &[u8])>,
    ) -> Result<bool> {
        let mut conn = self.0.lock().unwrap();
        let tx = conn.transaction()?;
        let changed = tx.execute(
            "UPDATE downloads SET status=?3,error=?4 WHERE room=?1 AND file_id=?2
             AND status!='completed' AND (status NOT IN('cancelled','rejected') OR status=?3)",
            params![room, file_id, status, error],
        )?;
        if changed == 0 { return Ok(false); }
        save_transfer_tx(&tx, room, transfer)?;
        if let Some((event_id, dedupe, kind, recipient, envelope)) = event {
            enqueue_event_tx(&tx, room, event_id, dedupe, kind, recipient, envelope, now())?;
        }
        tx.commit()?;
        Ok(true)
    }

    pub fn update_upload_with_event(
        &self,
        room: &str,
        file_id: &str,
        status: &str,
        error: Option<&str>,
        transfer: &Transfer,
        event: Option<(&str, &str, &str, &str, &[u8])>,
    ) -> Result<bool> {
        let mut conn = self.0.lock().unwrap();
        let tx = conn.transaction()?;
        let changed = tx.execute(
            "UPDATE uploads SET status=?3,error=?4 WHERE room=?1 AND file_id=?2
             AND status!='completed' AND (status NOT IN('failed','cancelled','rejected') OR status=?3)",
            params![room, file_id, status, error],
        )?;
        if changed == 0 { return Ok(false); }
        save_transfer_tx(&tx, room, transfer)?;
        if let Some((event_id, dedupe, kind, recipient, envelope)) = event {
            enqueue_event_tx(&tx, room, event_id, dedupe, kind, recipient, envelope, now())?;
        }
        tx.commit()?;
        Ok(true)
    }
}

#[derive(Clone, Debug)]
pub struct OutboxRow {
    pub event_id: String,
    pub kind: String,
    pub dedupe: String,
    pub envelope: Vec<u8>,
    pub attempts: u32,
}

#[derive(Clone, Debug)]
pub struct UploadRow {
    pub room: String,
    pub file_id: String,
    pub source_path: String,
    pub name: String,
    pub size: u64,
    pub hash: String,
    pub encrypted_size: u64,
    pub chunk_count: u64,
    pub status: String,
    pub error: Option<String>,
    pub created_at: u64,
}

#[derive(Clone, Debug)]
pub struct DownloadRow {
    pub room: String,
    pub file_id: String,
    pub sender: String,
    pub name: String,
    pub size: u64,
    pub hash: String,
    pub status: String,
    pub target_dir: Option<String>,
    pub temp_path: Option<String>,
    pub next_index: u64,
    pub error: Option<String>,
    pub created_at: u64,
    pub final_path: Option<String>,
}

fn upload_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<UploadRow> {
    Ok(UploadRow {
        room: row.get(0)?, file_id: row.get(1)?, source_path: row.get(2)?,
        name: row.get(3)?, size: row.get(4)?, hash: row.get(5)?,
        encrypted_size: row.get(6)?, chunk_count: row.get(7)?,
        status: row.get(8)?, error: row.get(9)?, created_at: row.get(10)?,
    })
}

fn download_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DownloadRow> {
    Ok(DownloadRow {
        room: row.get(0)?, file_id: row.get(1)?, sender: row.get(2)?,
        name: row.get(3)?, size: row.get(4)?, hash: row.get(5)?,
        status: row.get(6)?, target_dir: row.get(7)?, temp_path: row.get(8)?,
        next_index: row.get(9)?, error: row.get(10)?, created_at: row.get(11)?,
        final_path: row.get(12)?,
    })
}

#[cfg(test)]
#[path = "storage_migration_tests.rs"]
mod migration_tests;
