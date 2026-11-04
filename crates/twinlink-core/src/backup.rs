//! Portable local backups. No source paths, attachment bytes, encrypted spools,
//! outboxes or received-file paths are imported or replayed.
use crate::{
    model::{now, Message, Transfer},
    pairing::{ServerConfig, StoredRoom},
    Core, Desired,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::PathBuf,
};
use uuid::Uuid;

pub(crate) const MAX_BACKUP_BYTES: u64 = 256 * 1024 * 1024;
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Archive {
    pub format: String,
    pub version: u32,
    pub exported_at: u64,
    pub display_name: String,
    pub server: Option<ServerConfig>,
    pub room: Option<StoredRoom>,
    pub history_room: String,
    pub messages: Vec<MessageRecord>,
    pub transfers: Vec<TransferRecord>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MessageRecord {
    pub room: String,
    pub message: Message,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TransferRecord {
    pub room: String,
    pub transfer: Transfer,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub messages_added: usize,
    pub transfers_added: usize,
    pub restored_connection: bool,
}

fn valid_room(room: &str) -> bool {
    room.len() == 64
        && room
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
impl Archive {
    fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.format == "twinlink-backup" && self.version == 1,
            "不支持此备份版本"
        );
        anyhow::ensure!(
            !self.display_name.trim().is_empty()
                && self.display_name.chars().count() <= 64
                && !self.display_name.chars().any(char::is_control),
            "备份设备名称无效"
        );
        if let Some(server) = &self.server {
            server.validate().context("备份服务器配置无效")?;
        }
        if let Some(room) = &self.room {
            room.validate().context("备份房间配置无效")?;
        }
        anyhow::ensure!(
            self.history_room.is_empty() || valid_room(&self.history_room),
            "备份会话标识无效"
        );
        for row in &self.messages {
            anyhow::ensure!(
                valid_room(&row.room)
                    && Uuid::parse_str(&row.message.id).is_ok()
                    && row.message.timestamp <= i64::MAX as u64
                    && !row.message.text.trim().is_empty()
                    && row.message.text.len() <= 16 * 1024
                    && ["pending", "stored", "delivered", "unknown"]
                        .contains(&row.message.delivery.as_str())
                    && row
                        .message
                        .via
                        .as_deref()
                        .is_none_or(|v| ["direct", "mailbox"].contains(&v)),
                "备份聊天记录无效"
            );
        }
        for row in &self.transfers {
            anyhow::ensure!(
                valid_room(&row.room)
                    && Uuid::parse_str(&row.transfer.id).is_ok()
                    && row.transfer.transferred <= row.transfer.size
                    && [
                        "preparing",
                        "uploading",
                        "sending",
                        "waiting",
                        "offered",
                        "accepted",
                        "downloading",
                        "publishing",
                        "completed",
                        "rejected",
                        "cancelled",
                        "failed"
                    ]
                    .contains(&row.transfer.status.as_str())
                    && row.transfer.error.as_ref().is_none_or(|e| e.len() <= 8192)
                    && row
                        .transfer
                        .via
                        .as_deref()
                        .is_none_or(|v| ["direct", "mailbox"].contains(&v)),
                "备份文件记录无效"
            );
            crate::files::validate_name(&row.transfer.name)?;
        }
        Ok(())
    }
}

impl Core {
    pub async fn export_data(self: &std::sync::Arc<Self>, path: PathBuf) -> Result<()> {
        let _operation = self.operations.lock().await;
        let saved = self.saved.lock().unwrap().clone();
        let (name, history_room) = {
            let inner = self.inner.lock().await;
            (
                inner.profile.display_name.clone(),
                inner.history_room.clone(),
            )
        };
        let core = self.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            anyhow::ensure!(path.is_absolute(), "请选择备份保存位置");
            let (messages, transfers) = core.store.backup_records()?;
            let archive = Archive {
                format: "twinlink-backup".into(),
                version: 1,
                exported_at: now(),
                display_name: name,
                server: saved.server,
                room: saved.room,
                history_room,
                messages,
                transfers,
            };
            let parent = path.parent().context("备份路径无效")?;
            let mut temp = tempfile::Builder::new()
                .prefix(".twinlink-backup-")
                .tempfile_in(parent)?;
            crate::state::restrict_file(temp.path())?;
            serde_json::to_writer(&mut temp, &archive)?;
            anyhow::ensure!(
                temp.as_file().metadata()?.len() <= MAX_BACKUP_BYTES,
                "备份超过 256 MiB，请分批整理历史记录"
            );
            temp.flush()?;
            temp.as_file().sync_all()?;
            temp.persist_noclobber(path)
                .map_err(|e| e.error)
                .context("备份未保存：目标文件已存在或目录不可写，请选择新文件名")?;
            Ok(())
        })
        .await??;
        Ok(())
    }

    pub async fn import_data(self: &std::sync::Arc<Self>, path: PathBuf) -> Result<ImportReport> {
        // Parse and validate the complete input before changing any local data.
        let mut archive = tokio::task::spawn_blocking(move || -> Result<Archive> {
            let mut file = std::fs::File::open(path).context("无法打开备份文件")?;
            anyhow::ensure!(file.metadata()?.is_file(), "请选择备份文件");
            let mut bytes = Vec::new();
            (&mut file)
                .take(MAX_BACKUP_BYTES + 1)
                .read_to_end(&mut bytes)?;
            anyhow::ensure!(
                bytes.len() as u64 <= MAX_BACKUP_BYTES,
                "备份文件超过 256 MiB"
            );
            let archive: Archive = serde_json::from_slice(&bytes)
                .map_err(|_| anyhow::anyhow!("备份格式无效，未修改本地数据"))?;
            archive.validate()?;
            Ok(archive)
        })
        .await??;
        let _operation = self.operations.lock().await;
        let old_saved = self.saved.lock().unwrap().clone();
        let (mut profile, old_history) = {
            let inner = self.inner.lock().await;
            anyhow::ensure!(
                inner.paused || inner.room.is_none(),
                "请先暂停连接再导入备份"
            );
            (inner.profile.clone(), inner.history_room.clone())
        };
        let mut saved = old_saved.clone();
        // Merge history without replacing this device's existing room or paths.
        let restored_connection = saved.room.is_none() && archive.room.is_some();
        if saved.server.is_none() {
            saved.server = archive.server.take();
        }
        if restored_connection {
            saved.room = archive.room.take();
            saved.paused = true;
        }
        profile.display_name = archive.display_name.clone();
        profile.auto_accept_files = false;
        let history = if restored_connection {
            saved.room.as_ref().unwrap().spec()?.room_id
        } else if old_history.is_empty() {
            archive.history_room.clone()
        } else {
            old_history
        };
        for row in &mut archive.messages {
            // Unsent backup history is history, never an instruction to send.
            if row.message.outgoing && row.message.delivery == "pending" {
                row.message.delivery = "unknown".into();
            }
        }
        for row in &mut archive.transfers {
            if !["completed", "cancelled", "rejected", "failed"]
                .contains(&row.transfer.status.as_str())
            {
                row.transfer.status = "cancelled".into();
                row.transfer.error = Some("备份不包含附件本体或进行中的传输，请重新发送".into());
            }
        }
        self.state_file.save(&saved)?;
        let result =
            self.store
                .import_records(&archive.messages, &archive.transfers, &profile, &history);
        let (messages_added, transfers_added) = match result {
            Ok(counts) => counts,
            Err(error) => {
                self.state_file
                    .save(&old_saved)
                    .context("导入失败且配置恢复失败，请重新启动后检查设置")?;
                return Err(error).context("未能导入备份记录");
            }
        };
        *self.saved.lock().unwrap() = saved.clone();
        let generation = {
            let mut inner = self.inner.lock().await;
            inner.profile = profile;
            inner.room = saved.room.clone();
            inner.paused = saved.paused;
            inner.history_room = history;
            inner.generation = inner.generation.wrapping_add(1);
            inner.generation
        };
        self.desired.send_replace(Desired {
            room: saved.room,
            paused: saved.paused,
            generation,
        });
        self.emit_state().await;
        Ok(ImportReport {
            messages_added,
            transfers_added,
            restored_connection,
        })
    }
}
