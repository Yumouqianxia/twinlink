//! Cryptographic primitives for the private mailbox protocol.
//!
//! Every key is purpose-separated. The mailbox labels deliberately do **not**
//! reuse the `twinlink/v1\0` prefix of the pairing `mac()` helper: the server
//! contract derives its own keys from the same 32-byte room seed and the two
//! derivations must never be interchangeable.

use anyhow::{bail, Context, Result};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305, XNonce,
};
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const EVENT_VERSION: u8 = 1;
pub const EVENT_NONCE_LEN: usize = 24;
pub const TAG_LEN: usize = 16;
pub const MAX_EVENT_PLAINTEXT: usize = 64 * 1024;
pub const MAX_EVENT_ENVELOPE: usize = 96 * 1024;
/// Plaintext bytes per file chunk. The ciphertext chunk is 16 bytes larger.
pub const FILE_CHUNK_BYTES: usize = 1_048_576;

const ROOM_TOKEN_LABEL: &[u8] = b"twinlink/mailbox/auth/v1";
const EVENT_KEY_LABEL: &[u8] = b"twinlink/mailbox/events/v1";
const FILE_KEY_PREFIX: &str = "twinlink/mailbox/file/v1/";

fn is_lower_hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn seed_bytes(seed: &str) -> Result<[u8; 32]> {
    if !is_lower_hex(seed, 64) {
        bail!("房间种子格式无效");
    }
    let bytes = hex::decode(seed).context("房间种子格式无效")?;
    bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("房间种子格式无效"))
}

/// Mailbox room IDs are 64 lowercase hex characters derived from the seed,
/// **not** UUIDs. Every AAD construction validates this before encrypting.
fn validate_room_id(room_id: &str) -> Result<()> {
    if !is_lower_hex(room_id, 64) {
        bail!("房间标识无效");
    }
    Ok(())
}

fn canonical_uuid(value: &str, message: &str) -> Result<Uuid> {
    let parsed = Uuid::parse_str(value).context(message.to_owned())?;
    if parsed.to_string() != value {
        bail!("{message}");
    }
    Ok(parsed)
}

fn label_mac(seed: &str, label: &[u8]) -> Result<[u8; 32]> {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&seed_bytes(seed)?)
        .map_err(|_| anyhow::anyhow!("房间种子无效"))?;
    mac.update(label);
    Ok(mac.finalize().into_bytes().into())
}

/// `X-Room-Token`: 64 lowercase hex characters.
pub fn room_token(seed: &str) -> Result<String> {
    Ok(hex::encode(label_mac(seed, ROOM_TOKEN_LABEL)?))
}

pub fn event_key(seed: &str) -> Result<[u8; 32]> {
    label_mac(seed, EVENT_KEY_LABEL)
}

pub fn file_key(seed: &str, file_id: &str) -> Result<[u8; 32]> {
    // `file_id` is a canonical UUID supplied by the sender. Reject anything
    // else so the AAD and the key label can never disagree.
    canonical_uuid(file_id, "文件标识无效")?;
    label_mac(seed, format!("{FILE_KEY_PREFIX}{file_id}").as_bytes())
}

pub fn file_nonce(file_id: &str, index: u64) -> Result<[u8; EVENT_NONCE_LEN]> {
    let id = canonical_uuid(file_id, "文件标识无效")?;
    let mut nonce = [0u8; EVENT_NONCE_LEN];
    nonce[..16].copy_from_slice(id.as_bytes());
    nonce[16..].copy_from_slice(&index.to_le_bytes());
    Ok(nonce)
}

fn event_aad(room_id: &str, sender: &str, recipient: &str, event_id: &str) -> Vec<u8> {
    format!("twinlink/mailbox/v1|{room_id}|{sender}|{recipient}|{event_id}").into_bytes()
}

/// AAD uses the **whole file's** plaintext size and SHA-256, identical for
/// every chunk. Per-chunk values are deliberately not part of the AAD; the
/// expected chunk length is derived from `total` and `index` separately.
fn file_aad(
    room_id: &str,
    file_id: &str,
    index: u64,
    total_plaintext_size: u64,
    full_plaintext_sha256: &str,
) -> Vec<u8> {
    format!(
        "twinlink/file/v1|{room_id}|{file_id}|{index}|{total_plaintext_size}|{full_plaintext_sha256}"
    )
    .into_bytes()
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Seal a mailbox event: `[version][nonce:24][ciphertext+tag]`.
pub fn seal_event(
    seed: &str,
    room_id: &str,
    sender: &str,
    recipient: &str,
    event_id: &str,
    plaintext: &[u8],
) -> Result<Vec<u8>> {
    if plaintext.len() > MAX_EVENT_PLAINTEXT {
        bail!("离线消息内容过大");
    }
    canonical_uuid(event_id, "事件标识无效")?;
    validate_room_id(room_id)?;
    validate_role(sender)?;
    validate_role(recipient)?;
    if sender == recipient {
        bail!("离线消息的发送者与接收者必须不同");
    }
    let cipher = XChaCha20Poly1305::new((&event_key(seed)?).into());
    let mut nonce = [0u8; EVENT_NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce);
    let mut envelope = Vec::with_capacity(1 + EVENT_NONCE_LEN + plaintext.len() + TAG_LEN);
    envelope.push(EVENT_VERSION);
    envelope.extend_from_slice(&nonce);
    let sealed = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad: &event_aad(room_id, sender, recipient, event_id),
            },
        )
        .map_err(|_| anyhow::anyhow!("离线消息加密失败"))?;
    envelope.extend_from_slice(&sealed);
    if envelope.len() > MAX_EVENT_ENVELOPE {
        bail!("离线消息加密后超过服务器限制");
    }
    Ok(envelope)
}

pub fn open_event(
    seed: &str,
    room_id: &str,
    sender: &str,
    recipient: &str,
    event_id: &str,
    envelope: &[u8],
) -> Result<Vec<u8>> {
    if envelope.len() < 1 + EVENT_NONCE_LEN + TAG_LEN || envelope.len() > MAX_EVENT_ENVELOPE {
        bail!("离线消息长度无效");
    }
    if envelope[0] != EVENT_VERSION {
        bail!("不支持的离线消息版本");
    }
    canonical_uuid(event_id, "事件标识无效")?;
    validate_room_id(room_id)?;
    validate_role(sender)?;
    validate_role(recipient)?;
    if sender == recipient {
        bail!("离线消息的发送者与接收者必须不同");
    }
    let cipher = XChaCha20Poly1305::new((&event_key(seed)?).into());
    let nonce = XNonce::from_slice(&envelope[1..1 + EVENT_NONCE_LEN]);
    cipher
        .decrypt(
            nonce,
            Payload {
                msg: &envelope[1 + EVENT_NONCE_LEN..],
                aad: &event_aad(room_id, sender, recipient, event_id),
            },
        )
        .map_err(|_| anyhow::anyhow!("离线消息校验失败"))
}

/// Encrypt one plaintext file chunk.
///
/// `total_plaintext_size` and `full_plaintext_sha256` describe the **entire
/// immutable source file** and are identical in every chunk's AAD. The caller
/// must hash the complete source first, keep it immutable in the spool, and
/// verify it again before uploading any chunk.
pub fn seal_file_chunk(
    seed: &str,
    room_id: &str,
    file_id: &str,
    index: u64,
    total_plaintext_size: u64,
    full_plaintext_sha256: &str,
    plaintext: &[u8],
) -> Result<Vec<u8>> {
    validate_room_id(room_id)?;
    canonical_uuid(file_id, "文件标识无效")?;
    if full_plaintext_sha256.len() != 64 || !is_lower_hex(full_plaintext_sha256, 64) {
        bail!("文件校验值无效");
    }
    let expected = chunk_plaintext_size(total_plaintext_size, index)?;
    if plaintext.len() != expected || plaintext.len() > FILE_CHUNK_BYTES {
        bail!("文件分块长度与总大小不一致");
    }
    let nonce = file_nonce(file_id, index)?;
    let cipher = XChaCha20Poly1305::new((&file_key(seed, file_id)?).into());
    let sealed = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad: &file_aad(
                    room_id,
                    file_id,
                    index,
                    total_plaintext_size,
                    full_plaintext_sha256,
                ),
            },
        )
        .map_err(|_| anyhow::anyhow!("文件加密失败"))?;
    Ok(sealed)
}

pub fn open_file_chunk(
    seed: &str,
    room_id: &str,
    file_id: &str,
    index: u64,
    total_plaintext_size: u64,
    full_plaintext_sha256: &str,
    ciphertext: &[u8],
) -> Result<Vec<u8>> {
    validate_room_id(room_id)?;
    canonical_uuid(file_id, "文件标识无效")?;
    if !is_lower_hex(full_plaintext_sha256, 64) {
        bail!("文件校验值无效");
    }
    let expected = chunk_plaintext_size(total_plaintext_size, index)?;
    if ciphertext.len() != expected + TAG_LEN {
        bail!("文件分块参数无效");
    }
    let nonce = file_nonce(file_id, index)?;
    let cipher = XChaCha20Poly1305::new((&file_key(seed, file_id)?).into());
    let plaintext = cipher
        .decrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: ciphertext,
                aad: &file_aad(
                    room_id,
                    file_id,
                    index,
                    total_plaintext_size,
                    full_plaintext_sha256,
                ),
            },
        )
        .map_err(|_| anyhow::anyhow!("文件分块校验失败"))?;
    if plaintext.len() != expected {
        bail!("文件分块校验失败");
    }
    Ok(plaintext)
}

pub fn validate_role(role: &str) -> Result<()> {
    if role != "host" && role != "join" {
        bail!("房间角色无效");
    }
    Ok(())
}

pub fn peer_role(role: &str) -> Result<&'static str> {
    match role {
        "host" => Ok("join"),
        "join" => Ok("host"),
        _ => bail!("房间角色无效"),
    }
}

/// Number of chunks for a plaintext size. An empty file still has one
/// authenticated (16-byte) chunk.
pub fn chunk_count(size: u64) -> Result<u64> {
    let size = usize::try_from(size).context("文件过大")?;
    if size == 0 {
        return Ok(1);
    }
    Ok(((size + FILE_CHUNK_BYTES - 1) / FILE_CHUNK_BYTES) as u64)
}

pub fn encrypted_size(size: u64) -> Result<u64> {
    Ok(size
        .checked_add(chunk_count(size)? * TAG_LEN as u64)
        .context("文件过大")?)
}

/// Plaintext size of chunk `index` for a file of `size` bytes.
pub fn chunk_plaintext_size(size: u64, index: u64) -> Result<usize> {
    let count = chunk_count(size)?;
    if index >= count {
        bail!("文件分块序号无效");
    }
    let start = index * FILE_CHUNK_BYTES as u64;
    Ok((size - start).min(FILE_CHUNK_BYTES as u64) as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};

    const SEED: &str =
        "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
    const ROOM: &str =
        "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
    const FILE: &str = "22222222-2222-4222-8222-222222222222";
    const EVENT: &str = "33333333-3333-4333-8333-333333333333";

    fn fixture() -> serde_json::Value {
        serde_json::from_str(include_str!("../tests/fixtures/mailbox-v1-vectors.json"))
            .unwrap()
    }

    #[test]
    fn key_derivation_matches_protocol_vectors() {
        assert_eq!(
            room_token(SEED).unwrap(),
            "1923663bf0fbbc222b8724653c4864b3c7b72d1d1fc3bfc67653cff33a55d578"
        );
        assert_eq!(
            hex::encode(event_key(SEED).unwrap()),
            "9a7e17eadf280d7127da69408a196c71e5ac56dd50bd8c517f3c8ff089bba8af"
        );
        assert_eq!(
            hex::encode(file_key(SEED, "11111111-1111-4111-8111-111111111111").unwrap()),
            "e7a4591b0f4b53eac8ec6abafe3611be7c4ff26945f3d29642dce15f9c8aead1"
        );
    }

    #[test]
    fn opens_independent_libsodium_event_vector() {
        let fixture = fixture();
        let event = &fixture["event"];
        let room = fixture["roomId"].as_str().unwrap();
        let envelope = URL_SAFE_NO_PAD
            .decode(event["envelopeBase64url"].as_str().unwrap())
            .unwrap();
        assert_eq!(envelope[0], 1);
        assert_eq!(
            open_event(
                fixture["seed"].as_str().unwrap(),
                room,
                event["sender"].as_str().unwrap(),
                event["recipient"].as_str().unwrap(),
                event["id"].as_str().unwrap(),
                &envelope,
            )
            .unwrap(),
            event["plaintextUtf8"].as_str().unwrap().as_bytes()
        );
        // Re-sealing uses a fresh random nonce, so compare decrypted plaintext.
        let resealed = seal_event(
            fixture["seed"].as_str().unwrap(),
            room,
            event["sender"].as_str().unwrap(),
            event["recipient"].as_str().unwrap(),
            event["id"].as_str().unwrap(),
            event["plaintextUtf8"].as_str().unwrap().as_bytes(),
        )
        .unwrap();
        assert_eq!(
            open_event(
                fixture["seed"].as_str().unwrap(),
                room,
                event["sender"].as_str().unwrap(),
                event["recipient"].as_str().unwrap(),
                event["id"].as_str().unwrap(),
                &resealed,
            )
            .unwrap(),
            event["plaintextUtf8"].as_str().unwrap().as_bytes()
        );
    }

    #[test]
    fn opens_independent_libsodium_second_chunk_with_total_aad() {
        let fixture = fixture();
        let file = &fixture["file"];
        let room = fixture["roomId"].as_str().unwrap();
        let total = file["totalPlaintextSize"].as_u64().unwrap();
        let hash = file["fullPlaintextSha256"].as_str().unwrap();
        let index = file["index"].as_u64().unwrap();
        let plaintext = hex::decode(file["chunkPlaintextHex"].as_str().unwrap()).unwrap();
        let ciphertext = hex::decode(file["chunkCiphertextHex"].as_str().unwrap()).unwrap();

        // 2,097,169 bytes = two full chunks plus 17 bytes in the third.
        assert_eq!(chunk_count(total).unwrap(), 3);
        assert_eq!(chunk_plaintext_size(total, index).unwrap(), 17);
        assert_eq!(file_aad(room, FILE, index, total, hash), file["aadUtf8"].as_str().unwrap().as_bytes());

        assert_eq!(
            open_file_chunk(
                fixture["seed"].as_str().unwrap(),
                room,
                FILE,
                index,
                total,
                hash,
                &ciphertext,
            )
            .unwrap(),
            plaintext
        );
        assert_eq!(
            seal_file_chunk(
                fixture["seed"].as_str().unwrap(),
                room,
                FILE,
                index,
                total,
                hash,
                &plaintext,
            )
            .unwrap(),
            ciphertext
        );
        // Using per-chunk size/hash in AAD must fail.
        assert!(open_file_chunk(
            fixture["seed"].as_str().unwrap(),
            room,
            FILE,
            index,
            plaintext.len() as u64,
            &sha256_hex(&plaintext),
            &ciphertext,
        )
        .is_err());
        // Wrong index or truncated chunk must fail.
        assert!(open_file_chunk(
            fixture["seed"].as_str().unwrap(),
            room,
            FILE,
            index + 1,
            total,
            hash,
            &ciphertext,
        )
        .is_err());
    }

    #[test]
    fn events_round_trip_and_reject_tampering_or_wrong_aad() {
        let plaintext = br#"{"kind":"message"}"#;
        let envelope = seal_event(SEED, ROOM, "host", "join", EVENT, plaintext).unwrap();
        assert_eq!(envelope[0], 1);
        assert_eq!(
            open_event(SEED, ROOM, "host", "join", EVENT, &envelope).unwrap(),
            plaintext
        );
        assert!(open_event(SEED, ROOM, "join", "host", EVENT, &envelope).is_err());
        assert!(
            open_event(SEED, ROOM, "host", "join", EVENT, &envelope[..envelope.len() - 1])
                .is_err()
        );
        let mut changed = envelope.clone();
        let last = changed.len() - 1;
        changed[last] ^= 1;
        assert!(open_event(SEED, ROOM, "host", "join", EVENT, &changed).is_err());
    }

    #[test]
    fn rejects_same_sender_and_recipient_and_bad_room_ids() {
        assert!(seal_event(SEED, ROOM, "host", "host", EVENT, b"x").is_err());
        assert!(seal_event(SEED, "room", "host", "join", EVENT, b"x").is_err());
        // A UUID is not a mailbox room ID.
        assert!(seal_event(SEED, EVENT, "host", "join", EVENT, b"x").is_err());
        assert!(seal_event(
            SEED,
            "ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789",
            "host",
            "join",
            EVENT,
            b"x"
        )
        .is_err());
    }

    #[test]
    fn file_chunks_bind_total_size_and_full_hash() {
        let data = vec![7u8; FILE_CHUNK_BYTES];
        let total = data.len() as u64;
        let hash = sha256_hex(&data);
        let sealed = seal_file_chunk(SEED, ROOM, FILE, 0, total, &hash, &data).unwrap();
        assert_eq!(sealed.len(), FILE_CHUNK_BYTES + TAG_LEN);
        assert_eq!(
            open_file_chunk(SEED, ROOM, FILE, 0, total, &hash, &sealed).unwrap(),
            data
        );
        assert!(open_file_chunk(SEED, ROOM, FILE, 1, total, &hash, &sealed).is_err());
        assert!(open_file_chunk(
            SEED,
            ROOM,
            FILE,
            0,
            total,
            &sha256_hex(b"other"),
            &sealed
        )
        .is_err());
    }

    #[test]
    fn empty_file_still_has_one_authenticated_chunk() {
        assert_eq!(chunk_count(0).unwrap(), 1);
        assert_eq!(encrypted_size(0).unwrap(), 16);
        assert_eq!(chunk_plaintext_size(0, 0).unwrap(), 0);
        let hash = sha256_hex(b"");
        let sealed = seal_file_chunk(SEED, ROOM, FILE, 0, 0, &hash, &[]).unwrap();
        assert_eq!(sealed.len(), 16);
        assert_eq!(
            open_file_chunk(SEED, ROOM, FILE, 0, 0, &hash, &sealed).unwrap(),
            Vec::<u8>::new()
        );
    }

    #[test]
    fn malformed_seeds_ids_and_lengths_are_rejected() {
        assert!(room_token("abc").is_err());
        assert!(file_key(SEED, "not-a-uuid").is_err());
        // Non-canonical (uppercase) UUID must not be accepted.
        assert!(file_key(SEED, "22222222-2222-4222-8222-22222222222A").is_err());
        assert!(chunk_plaintext_size(0, 1).is_err());
        assert!(seal_file_chunk(SEED, ROOM, FILE, 0, 10, &sha256_hex(b"x"), b"short").is_err());
    }
}
