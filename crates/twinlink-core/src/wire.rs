use anyhow::{bail, Context, Result};
use hmac::{Hmac, Mac};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use uuid::Uuid;

const MAX_FRAME: usize = 128 * 1024;
const VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Control {
    Chat { id: Uuid, text: String, #[serde(default)] timestamp: u64 },
    Receipt { id: Uuid },
    Offer { id: Uuid, name: String, size: u64, hash: String },
    Accept { id: Uuid },
    Cancel { id: Uuid },
    Finish { id: Uuid },
    Complete { id: Uuid },
    Ping,
    Pong,
}

pub enum Frame {
    Control(Control),
    Chunk { id: Uuid, bytes: Vec<u8> },
}

pub async fn write_frame<W: AsyncWrite + Unpin>(writer: &mut W, frame: &Frame) -> Result<()> {
    let mut bytes = Vec::new();
    match frame {
        Frame::Control(control) => { bytes.push(0); bytes.extend(serde_json::to_vec(control)?); }
        Frame::Chunk { id, bytes: chunk } => {
            if chunk.len() > crate::files::CHUNK_SIZE { bail!("数据块过大"); }
            bytes.push(1); bytes.extend(id.as_bytes()); bytes.extend(chunk);
        }
    }
    if bytes.len() > MAX_FRAME { bail!("消息过大"); }
    writer.write_u32(bytes.len() as u32).await?;
    writer.write_all(&bytes).await?;
    Ok(())
}

pub async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Frame> {
    let len = reader.read_u32().await? as usize;
    if len == 0 || len > MAX_FRAME { bail!("无效的数据帧长度"); }
    let mut bytes = vec![0; len];
    reader.read_exact(&mut bytes).await?;
    match bytes[0] {
        0 => Ok(Frame::Control(serde_json::from_slice(&bytes[1..])?)),
        1 if len >= 17 && len <= 17 + crate::files::CHUNK_SIZE => Ok(Frame::Chunk {
            id: Uuid::from_slice(&bytes[1..17])?, bytes: bytes[17..].to_vec(),
        }),
        _ => bail!("无效的数据帧"),
    }
}

// Authentication is additional to EasyTier's encrypted private overlay.
// Random challenges prevent replay; different role labels prevent reflection.
#[derive(Serialize, Deserialize)]
struct Hello { version: u32, name: String, nonce: String, proof: String }

fn proof(secret: &str, label: &[u8], server: &[u8], client: &[u8], name: &str) -> Hmac<Sha256> {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key size");
    mac.update(b"twinlink/v1/"); mac.update(label); mac.update(server); mac.update(client);
    mac.update(&(name.len() as u32).to_be_bytes()); mac.update(name.as_bytes());
    mac
}

async fn send_hello<S: AsyncWrite + Unpin>(stream: &mut S, hello: &Hello) -> Result<()> {
    let bytes = serde_json::to_vec(hello)?;
    stream.write_u32(bytes.len() as u32).await?;
    stream.write_all(&bytes).await?;
    Ok(())
}

async fn read_hello<S: AsyncRead + Unpin>(stream: &mut S) -> Result<Hello> {
    let len = stream.read_u32().await?;
    if len > 1024 { bail!("握手消息过大"); }
    let mut bytes = vec![0; len as usize];
    stream.read_exact(&mut bytes).await?;
    let hello: Hello = serde_json::from_slice(&bytes)?;
    if hello.version != VERSION || hello.name.trim().is_empty() || hello.name.chars().count() > 64 || hello.name.chars().any(char::is_control) {
        bail!("对方版本或设备名称无效");
    }
    Ok(hello)
}

pub async fn authenticate<S: AsyncRead + AsyncWrite + Unpin>(stream: &mut S, secret: &str, name: &str, server_side: bool) -> Result<String> {
    let mut nonce = [0; 32];
    rand::thread_rng().fill_bytes(&mut nonce);
    if server_side {
        send_hello(stream, &Hello { version: VERSION, name: name.into(), nonce: hex::encode(nonce), proof: String::new() }).await?;
        let peer = read_hello(stream).await?;
        let other = hex::decode(&peer.nonce)?;
        if other.len() != 32 { bail!("无效的握手随机数"); }
        proof(secret, b"client", &nonce, &other, &peer.name).verify_slice(&hex::decode(&peer.proof)?).context("房间密码不匹配")?;
        let signed = proof(secret, b"server", &nonce, &other, name).finalize().into_bytes();
        send_hello(stream, &Hello { version: VERSION, name: name.into(), nonce: hex::encode(nonce), proof: hex::encode(signed) }).await?;
        Ok(peer.name)
    } else {
        let server = read_hello(stream).await?;
        let server_nonce = hex::decode(&server.nonce)?;
        if server_nonce.len() != 32 { bail!("无效的握手随机数"); }
        let signed = proof(secret, b"client", &server_nonce, &nonce, name).finalize().into_bytes();
        send_hello(stream, &Hello { version: VERSION, name: name.into(), nonce: hex::encode(nonce), proof: hex::encode(signed) }).await?;
        let authenticated = read_hello(stream).await?;
        proof(secret, b"server", &server_nonce, &nonce, &authenticated.name).verify_slice(&hex::decode(&authenticated.proof)?).context("房间密码不匹配")?;
        Ok(authenticated.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn rejects_oversize_before_allocating_body() {
        let (mut tx, mut rx) = tokio::io::duplex(8);
        tx.write_u32(u32::MAX).await.unwrap();
        assert!(read_frame(&mut rx).await.is_err());
    }
    #[tokio::test]
    async fn rejects_incorrect_room_secret() {
        let (mut a, mut b) = tokio::io::duplex(4096);
        let (left, right) = tokio::join!(async move {
            let result = authenticate(&mut a, "secret-a", "A", true).await;
            drop(a);
            result
        }, async move {
            let result = authenticate(&mut b, "secret-b", "B", false).await;
            drop(b);
            result
        });
        assert!(left.is_err() || right.is_err());
    }
}
