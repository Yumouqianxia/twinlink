//! Consent-gated file bytes on the authenticated, P2P-only EasyTier channel.
//! Only metadata/receipts use the mailbox while a direct session is healthy.
use crate::{
    crypto,
    files::{self, CHUNK_SIZE},
    mailbox::Body,
    model::{now, AppEvent, Transfer},
    storage::{DownloadRow, UploadRow},
    wire::{Control, Frame},
    Core,
};
use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::mpsc,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Default)]
pub(crate) struct DirectState {
    pub supported: bool,
    pub cancel: CancellationToken,
    incoming: HashMap<Uuid, Incoming>,
}
impl DirectState {
    pub(crate) fn cancel_file(&mut self, id: Uuid) {
        self.incoming.remove(&id);
    }
}
struct Incoming {
    temp: tempfile::NamedTempFile,
    file: tokio::fs::File,
    hash: Sha256,
    received: u64,
    progress_at: Instant,
}

impl Core {
    pub(crate) async fn direct_offer(&self, path: &Path) -> Result<Option<Transfer>> {
        let (selection, generation, sender) = {
            let inner = self.inner.lock().await;
            let Some(sender) = inner.sender.clone() else {
                return Ok(None);
            };
            if !self.direct.lock().await.supported {
                return Ok(None);
            }
            (
                inner.room.clone().context("请先配对")?,
                inner.generation,
                sender,
            )
        };
        let name = path
            .file_name()
            .and_then(|v| v.to_str())
            .context("文件名不是有效 UTF-8")?
            .to_owned();
        files::validate_name(&name)?;
        // No P2P size cap and no whole-file buffer or encrypted spool copy.
        let (size, hash) = files::hash_file(path).await?;
        let room = selection.spec()?.room_id;
        let _operation = self.operations.lock().await;
        anyhow::ensure!(
            self.generation_is_current(&room, generation).await,
            "房间已改变，请重新发送"
        );
        if sender.is_closed() {
            return Ok(None);
        }
        let id = Uuid::new_v4();
        let transfer = Transfer {
            id: id.to_string(),
            name: name.clone(),
            size,
            transferred: 0,
            outgoing: true,
            status: "waiting".into(),
            via: Some("direct".into()),
            error: None,
        };
        self.store.insert_upload(
            &UploadRow {
                room: room.clone(),
                file_id: id.to_string(),
                source_path: path.to_string_lossy().into_owned(),
                name: name.clone(),
                size,
                hash: hash.clone(),
                encrypted_size: 0,
                chunk_count: 0,
                status: "directWaiting".into(),
                error: None,
                created_at: now(),
            },
            &transfer,
        )?;
        self.emit_in_room(
            &room,
            AppEvent::Transfer {
                transfer: transfer.clone(),
            },
        )
        .await;
        if sender
            .try_send(Frame::Control(Control::Offer {
                id,
                name,
                size,
                hash,
            }))
            .is_err()
        {
            self.recover_direct_room(&room).await?;
            return Ok(self.store.transfer(&room, &id.to_string())?);
        }
        Ok(Some(transfer))
    }

    // Called under operations, as are mailbox intake and all user actions.
    pub(crate) async fn direct_receive_offer(
        &self,
        room: &str,
        id: Uuid,
        name: String,
        size: u64,
        hash: String,
    ) -> Result<()> {
        files::validate_name(&name)?;
        anyhow::ensure!(
            hash.len() == 64
                && hash
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
            "文件校验信息无效"
        );
        if self.store.download(room, &id.to_string())?.is_some() {
            return Ok(());
        }
        let transfer = Transfer {
            id: id.to_string(),
            name: name.clone(),
            size,
            transferred: 0,
            outgoing: false,
            status: "offered".into(),
            via: Some("direct".into()),
            error: None,
        };
        self.store.insert_offer(
            &DownloadRow {
                room: room.into(),
                file_id: id.to_string(),
                sender: "peer".into(),
                name,
                size,
                hash,
                status: "directOffered".into(),
                target_dir: None,
                temp_path: None,
                next_index: 0,
                error: None,
                created_at: now(),
                final_path: None,
            },
            &transfer,
        )?;
        self.emit_in_room(room, AppEvent::Transfer { transfer })
            .await;
        if self.inner.lock().await.profile.auto_accept_files {
            self.direct_accept(room, id).await?;
        }
        Ok(())
    }

    pub(crate) async fn direct_accept(&self, room: &str, id: Uuid) -> Result<()> {
        let row = self
            .store
            .download(room, &id.to_string())?
            .context("未知文件请求")?;
        anyhow::ensure!(
            row.status == "directOffered",
            "直连已断开，请等待离线暂存或让对方重发"
        );
        let (directory, sender) = {
            let inner = self.inner.lock().await;
            (
                PathBuf::from(&inner.profile.download_dir),
                inner.sender.clone().context("对方已离线")?,
            )
        };
        anyhow::ensure!(
            directory.is_absolute() && directory.is_dir(),
            "下载目录不可用，请在设置中重新选择"
        );
        let directory = std::fs::canonicalize(directory)?;
        let temp = tempfile::Builder::new()
            .prefix(".twinlink-")
            .suffix(".part")
            .tempfile_in(&directory)?;
        crate::state::restrict_file(temp.path())?;
        let file = tokio::fs::File::from_std(temp.as_file().try_clone()?);
        self.store.set_download(
            room,
            &id.to_string(),
            "directReceiving",
            Some(&directory.to_string_lossy()),
            Some(&temp.path().to_string_lossy()),
            0,
            None,
        )?;
        self.direct.lock().await.incoming.insert(
            id,
            Incoming {
                temp,
                file,
                hash: Sha256::new(),
                received: 0,
                progress_at: Instant::now(),
            },
        );
        self.update_transfer(room, &id.to_string(), "downloading", 0, row.size, None)
            .await;
        if sender
            .try_send(Frame::Control(Control::Accept { id }))
            .is_err()
        {
            self.recover_direct_room(room).await?;
            bail!("对方已离线，等待服务器暂存");
        }
        Ok(())
    }

    pub(crate) async fn direct_start_send(
        self: &Arc<Self>,
        room: &str,
        generation: u64,
        id: Uuid,
        sender: mpsc::Sender<Frame>,
    ) -> Result<()> {
        let Some(row) = self.store.upload(room, &id.to_string())? else {
            return Ok(());
        };
        if row.status != "directWaiting" {
            return Ok(());
        }
        self.store
            .set_upload_status(room, &row.file_id, "directSending", None)?;
        self.update_transfer(room, &row.file_id, "sending", 0, row.size, None)
            .await;
        let cancel = self.direct.lock().await.cancel.clone();
        let core = self.clone();
        let room = room.to_owned();
        tokio::spawn(async move {
            let result = tokio::select! {
                biased;
                _ = cancel.cancelled() => return,
                result = core.direct_stream(&room, generation, id, &row, &sender) => result,
            };
            if let Err(error) = result {
                let _operation = core.operations.lock().await;
                if core.generation_is_current(&room, generation).await
                    && matches!(core.store.upload(&room, &row.file_id), Ok(Some(r)) if r.status == "directSending")
                {
                    let _ = core.store.set_upload_status(
                        &room,
                        &row.file_id,
                        "failed",
                        Some(&error.to_string()),
                    );
                    core.update_transfer(
                        &room,
                        &row.file_id,
                        "failed",
                        0,
                        row.size,
                        Some(error.to_string()),
                    )
                    .await;
                    let _ = core.notify_file_failure(&room, &row.file_id).await;
                    let _ = sender.try_send(Frame::Control(Control::Cancel { id }));
                }
            }
        });
        Ok(())
    }

    async fn direct_stream(
        &self,
        room: &str,
        generation: u64,
        id: Uuid,
        row: &UploadRow,
        sender: &mpsc::Sender<Frame>,
    ) -> Result<()> {
        let mut file = tokio::fs::File::open(&row.source_path)
            .await
            .context("无法重新打开所选文件")?;
        anyhow::ensure!(
            file.metadata().await?.is_file() && file.metadata().await?.len() == row.size,
            "源文件已改变，请重新发送"
        );
        let mut buffer = vec![0; CHUNK_SIZE];
        let mut sent = 0u64;
        let mut hash = Sha256::new();
        let mut progress_at = Instant::now();
        loop {
            let read = file.read(&mut buffer).await?;
            if read == 0 {
                break;
            }
            let _operation = self.operations.lock().await;
            anyhow::ensure!(
                self.generation_is_current(room, generation).await,
                "房间已改变"
            );
            anyhow::ensure!(
                matches!(self.store.upload(room, &row.file_id)?, Some(r) if r.status == "directSending"),
                "发送已取消"
            );
            sent = sent.checked_add(read as u64).context("文件长度无效")?;
            anyhow::ensure!(sent <= row.size, "源文件已改变，请重新发送");
            hash.update(&buffer[..read]);
            drop(_operation);
            // Bounded queue provides backpressure; IPC never sees the bytes.
            sender
                .send(Frame::Chunk {
                    id,
                    bytes: buffer[..read].to_vec(),
                })
                .await
                .context("直连已断开")?;
            if progress_at.elapsed() >= Duration::from_millis(250) {
                let _operation = self.operations.lock().await;
                anyhow::ensure!(
                    matches!(self.store.upload(room, &row.file_id)?, Some(r) if r.status == "directSending"),
                    "发送已取消"
                );
                self.update_transfer(room, &row.file_id, "sending", sent, row.size, None)
                    .await;
                progress_at = Instant::now();
            }
        }
        anyhow::ensure!(
            sent == row.size && hex::encode(hash.finalize()) == row.hash,
            "源文件已改变，请重新发送"
        );
        let _operation = self.operations.lock().await;
        anyhow::ensure!(
            self.generation_is_current(room, generation).await,
            "房间已改变"
        );
        anyhow::ensure!(
            matches!(self.store.upload(room, &row.file_id)?, Some(r) if r.status == "directSending"),
            "发送已取消"
        );
        // Waiting for a durable peer completion, not treating sent bytes as saved.
        self.store
            .set_upload_status(room, &row.file_id, "directFinishing", None)?;
        self.update_transfer(room, &row.file_id, "waiting", sent, row.size, None)
            .await;
        drop(_operation);
        sender
            .send(Frame::Control(Control::Finish { id }))
            .await
            .context("直连已断开")?;
        Ok(())
    }

    pub(crate) async fn direct_chunk(&self, room: &str, id: Uuid, bytes: Vec<u8>) -> Result<()> {
        let row = self
            .store
            .download(room, &id.to_string())?
            .context("未收到文件邀请")?;
        // Unaccepted, cancelled or delayed chunks must never write a file.
        if row.status != "directReceiving" {
            return Ok(());
        }
        let mut direct = self.direct.lock().await;
        let incoming = direct.incoming.get_mut(&id).context("接收会话已结束")?;
        let received = incoming
            .received
            .checked_add(bytes.len() as u64)
            .context("文件长度无效")?;
        anyhow::ensure!(received <= row.size, "对方发送的内容超过文件声明长度");
        incoming.file.write_all(&bytes).await?;
        incoming.hash.update(&bytes);
        incoming.received = received;
        let progress = incoming.progress_at.elapsed() >= Duration::from_millis(250);
        if progress {
            incoming.progress_at = Instant::now();
        }
        drop(direct);
        if progress {
            self.update_transfer(
                room,
                &id.to_string(),
                "downloading",
                received,
                row.size,
                None,
            )
            .await;
        }
        Ok(())
    }

    pub(crate) async fn direct_finish(
        &self,
        room: &str,
        id: Uuid,
        sender: &mpsc::Sender<Frame>,
    ) -> Result<()> {
        let row = self
            .store
            .download(room, &id.to_string())?
            .context("未知文件请求")?;
        if row.status != "directReceiving" {
            return Ok(());
        }
        let Some(incoming) = self.direct.lock().await.incoming.remove(&id) else {
            return Ok(());
        };
        if incoming.received != row.size || hex::encode(incoming.hash.finalize()) != row.hash {
            self.store.set_download(
                room,
                &row.file_id,
                "failed",
                None,
                None,
                0,
                Some("文件校验失败"),
            )?;
            self.update_transfer(
                room,
                &row.file_id,
                "failed",
                0,
                row.size,
                Some("文件校验失败".into()),
            )
            .await;
            self.notify_file_failure(room, &row.file_id).await?;
            let _ = sender.try_send(Frame::Control(Control::Cancel { id }));
            return Ok(());
        }
        incoming.file.sync_all().await?;
        drop(incoming.file);
        let directory = PathBuf::from(row.target_dir.as_ref().context("下载目录缺失")?);
        let mut transfer = self
            .store
            .transfer(room, &row.file_id)?
            .context("文件记录缺失")?;
        transfer.status = "publishing".into();
        transfer.transferred = row.size;
        // Journal before the no-clobber publication, so restart can recover a
        // file saved before its completion receipt was recorded.
        let mut temp = incoming.temp;
        let mut final_path = None;
        for index in 0..10000 {
            let path = files::destination(&directory, &row.name, index);
            self.store.set_download_publishing(
                room,
                &row.file_id,
                &directory.to_string_lossy(),
                &temp.path().to_string_lossy(),
                0,
                &path.to_string_lossy(),
                &transfer,
            )?;
            match temp.persist_noclobber(&path) {
                Ok(_) => {
                    final_path = Some(path);
                    break;
                }
                Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                    temp = error.file;
                }
                Err(error) => return Err(error.error.into()),
            }
        }
        let path = final_path.context("同名文件过多，请更改接收目录")?;
        let (role, peer, seed) = self.current_identity(room).await?;
        let event_id = Uuid::new_v4().to_string();
        let envelope = crypto::seal_event(
            &seed,
            room,
            &role,
            &peer,
            &event_id,
            &Body::FileComplete { id: id.to_string() }.encode()?,
        )?;
        transfer.status = "completed".into();
        self.store.complete_download(
            room,
            &row.file_id,
            &path.to_string_lossy(),
            &event_id,
            &format!("file-complete:{id}"),
            &peer,
            &envelope,
            &transfer,
        )?;
        self.emit_in_room(room, AppEvent::Transfer { transfer })
            .await;
        let _ = sender.try_send(Frame::Control(Control::Complete { id }));
        self.mailbox_wake.notify_one();
        Ok(())
    }

    pub(crate) async fn direct_complete(&self, room: &str, id: Uuid) -> Result<()> {
        let Some(row) = self.store.upload(room, &id.to_string())? else {
            return Ok(());
        };
        if row.status != "directFinishing" {
            return Ok(());
        }
        let mut transfer = self
            .store
            .transfer(room, &row.file_id)?
            .context("文件记录缺失")?;
        transfer.status = "completed".into();
        transfer.transferred = row.size;
        transfer.error = None;
        if self.store.complete_upload(room, &row.file_id, &transfer)? {
            self.emit_in_room(room, AppEvent::Transfer { transfer })
                .await;
        }
        Ok(())
    }

    pub(crate) async fn direct_cancel(
        &self,
        room: &str,
        id: Uuid,
        rejected: bool,
        notify: bool,
    ) -> Result<()> {
        let status = if rejected { "rejected" } else { "cancelled" };
        let mut transfer = self
            .store
            .transfer(room, &id.to_string())?
            .context("未知文件传输")?;
        anyhow::ensure!(
            !notify || !rejected || !transfer.outgoing,
            "只能拒收对方文件"
        );
        if ["completed", "publishing", "cancelled", "rejected", "failed"]
            .contains(&transfer.status.as_str())
        {
            return Ok(());
        }
        if notify {
            // Durable authenticated rejection/cancellation survives a dropped
            // P2P control frame, so fallback cannot resurrect a refused offer.
            self.mailbox_cancel_file(id, rejected).await?;
            self.direct.lock().await.incoming.remove(&id);
            if let Some(sender) = self.inner.lock().await.sender.clone() {
                let control = if rejected {
                    Control::Reject { id }
                } else {
                    Control::Cancel { id }
                };
                let _ = sender.try_send(Frame::Control(control));
            }
            return Ok(());
        }
        if transfer.outgoing {
            self.store
                .set_upload_status(room, &id.to_string(), status, None)?;
        } else {
            self.store
                .set_download(room, &id.to_string(), status, None, None, 0, None)?;
            self.direct.lock().await.incoming.remove(&id);
        }
        transfer.status = status.into();
        transfer.error = None;
        self.store.save_transfer(room, &transfer)?;
        self.emit_in_room(room, AppEvent::Transfer { transfer })
            .await;
        Ok(())
    }

    pub(crate) async fn recover_direct_room(&self, room: &str) -> Result<()> {
        self.store.recover_direct(room)?;
        for transfer in self.store.transfers(room)? {
            self.emit_in_room(room, AppEvent::Transfer { transfer })
                .await;
        }
        self.mailbox_wake.notify_one();
        Ok(())
    }
}
