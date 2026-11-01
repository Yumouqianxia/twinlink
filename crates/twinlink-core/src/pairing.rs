//! Pairing codes and room credentials.
//!
//! Two codes exist:
//!
//! * `TL-SRV1.` + base64url(JSON [`ServerConfig`]) — one-time server import. It
//!   contains the access token, so it is a credential and must be handled as one.
//! * `TL1.` + base64url(JSON invite) — a room invite. The 32-byte seed inside is
//!   the room key: anyone holding the code can join. Base64url is an encoding,
//!   not encryption.
//!
//! Everything a room needs (network name, network secret, virtual addresses) is
//! derived from the seed with purpose-separated HMAC-SHA256, so two different
//! seeds produce two different rooms without asking the user for addresses.

use std::fmt;

use anyhow::{bail, Context, Result};
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine as _;
use hmac::{Hmac, Mac};
use rand::{rngs::OsRng, RngCore};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use url::Url;

pub const SERVER_PREFIX: &str = "TL-SRV1.";
pub const INVITE_PREFIX: &str = "TL1.";

/// Setup codes carry a PEM chain plus a token; keep a hard ceiling so a pasted
/// file or a corrupted clipboard cannot make the app work through megabytes.
const MAX_SERVER_CODE: usize = 32 * 1024;
const MAX_INVITE_CODE: usize = 16 * 1024;
const MAX_RELAY_URL: usize = 256;
const MAX_MAILBOX_URL: usize = 256;
const MAX_CA_PEM: usize = 16 * 1024;
const MAX_TOKEN: usize = 512;

/* ---------- server configuration ---------- */

/// The coordinator-provided server bundle. Field names match the wire contract
/// (`serde` camelCase) so the setup code is exactly what the server team emits.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServerConfig {
    pub relay_url: String,
    pub mailbox_url: String,
    pub mailbox_ca_pem: String,
    pub access_token: String,
    pub network_prefix: String,
}

/// Secrets never appear in logs. `Debug` is redacted by hand and is the only
/// formatting path used by events and errors.
impl fmt::Debug for ServerConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServerConfig")
            .field("relay_url", &self.relay_url)
            .field("mailbox_url", &self.mailbox_url)
            .field("mailbox_ca_pem", &format_args!("<redacted {} bytes>", self.mailbox_ca_pem.len()))
            .field("access_token", &"<redacted>")
            .field("network_prefix", &self.network_prefix)
            .finish()
    }
}

/// Non-secret view for the UI snapshot. The token and PEM stay in the core.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerSummary {
    pub relay_url: String,
    pub mailbox_url: String,
    /// Hostname shown for the mailbox certificate; never the token.
    pub mailbox_host: String,
}

impl ServerConfig {
    pub fn validate(&self) -> Result<()> {
        validate_relay(&self.relay_url)?;
        validate_mailbox(&self.mailbox_url)?;
        validate_ca_pem(&self.mailbox_ca_pem)?;

        if !(32..=MAX_TOKEN).contains(&self.access_token.len()) {
            bail!("服务器访问令牌长度无效");
        }
        if !self.access_token.bytes().all(|b| b.is_ascii_graphic()) {
            bail!("服务器访问令牌包含无效字符");
        }

        let prefix = &self.network_prefix;
        if !(16..=48).contains(&prefix.len()) {
            bail!("服务器网络前缀长度无效（应为 16–48 个字符）");
        }
        if !prefix.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') {
            bail!("服务器网络前缀只能包含小写字母、数字和连字符");
        }
        if prefix.starts_with('-') || prefix.ends_with('-') || prefix.contains("--") {
            bail!("服务器网络前缀格式无效");
        }
        Ok(())
    }

    pub fn summary(&self) -> ServerSummary {
        ServerSummary {
            relay_url: self.relay_url.clone(),
            mailbox_url: self.mailbox_url.clone(),
            mailbox_host: Url::parse(&self.mailbox_url)
                .ok()
                .and_then(|url| url.host_str().map(str::to_owned))
                .unwrap_or_else(|| "未知".into()),
        }
    }
}

fn validate_relay(value: &str) -> Result<()> {
    if value.len() < 8 || value.len() > MAX_RELAY_URL {
        bail!("联机服务器地址长度无效");
    }
    let url = Url::parse(value).context("联机服务器地址不是有效 URL")?;
    if !matches!(url.scheme(), "tcp" | "udp" | "ws" | "wss" | "quic") {
        bail!("联机服务器地址必须使用 tcp、udp、ws、wss 或 quic");
    }
    if url.host_str().is_none_or(str::is_empty) {
        bail!("联机服务器地址缺少主机名");
    }
    if url.port().is_none() {
        bail!("联机服务器地址必须包含端口");
    }
    if !url.username().is_empty() || url.password().is_some() {
        bail!("联机服务器地址不能包含账号信息");
    }
    if url.query().is_some() || url.fragment().is_some() {
        bail!("联机服务器地址不能包含查询参数");
    }
    if url.path() != "/" && !url.path().is_empty() {
        bail!("联机服务器地址不能包含路径");
    }
    Ok(())
}

fn validate_mailbox(value: &str) -> Result<()> {
    if value.len() < 9 || value.len() > MAX_MAILBOX_URL {
        bail!("信箱服务器地址长度无效");
    }
    let url = Url::parse(value).context("信箱服务器地址不是有效 URL")?;
    // Deliberately HTTPS-only: the certificate is validated against this host.
    if url.scheme() != "https" {
        bail!("信箱服务器地址必须使用 https");
    }
    if url.host_str().is_none_or(str::is_empty) {
        bail!("信箱服务器地址缺少主机名");
    }
    if !url.username().is_empty() || url.password().is_some() {
        bail!("信箱服务器地址不能包含账号信息");
    }
    if url.query().is_some() || url.fragment().is_some() {
        bail!("信箱服务器地址不能包含查询参数");
    }
    if url.path() != "/" && !url.path().is_empty() {
        bail!("信箱服务器地址必须是根地址");
    }
    Ok(())
}

fn validate_ca_pem(pem: &str) -> Result<()> {
    if pem.len() < 64 || pem.len() > MAX_CA_PEM {
        bail!("服务器证书长度无效");
    }
    if pem.bytes().any(|b| b != b'\n' && b != b'\r' && b != b'\t' && !(0x20..=0x7e).contains(&b)) {
        bail!("服务器证书包含无效字符");
    }

    let begin = "-----BEGIN CERTIFICATE-----";
    let end = "-----END CERTIFICATE-----";
    let mut rest = pem;
    let mut blocks = 0usize;
    while let Some(start) = rest.find(begin) {
        let body_start = start + begin.len();
        let Some(body_len) = rest[body_start..].find(end) else {
            bail!("服务器证书格式不完整");
        };
        let compact: String = rest[body_start..body_start + body_len]
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        if compact.len() < 24 {
            bail!("服务器证书内容过短");
        }
        let der = STANDARD
            .decode(compact.as_bytes())
            .or_else(|_| URL_SAFE_NO_PAD.decode(compact.as_bytes()))
            .context("服务器证书不是有效的 PEM")?;
        if der.len() < 16 || der[0] != 0x30 {
            bail!("服务器证书不是有效的 DER 证书");
        }
        blocks += 1;
        rest = &rest[body_start + body_len + end.len()..];
    }
    if blocks == 0 {
        bail!("服务器证书缺少 CERTIFICATE 块");
    }
    Ok(())
}

/* ---------- code framing ---------- */

fn decode_body<'a>(input: &'a str, prefix: &str, max: usize, label: &str) -> Result<&'a str> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        bail!("请输入{label}");
    }
    if trimmed.len() > max {
        bail!("{label}过长，可能不是有效的配对码");
    }
    let Some(body) = trimmed.strip_prefix(prefix) else {
        bail!("{label}格式无法识别，应以 {prefix} 开头");
    };
    if body.is_empty() {
        bail!("{label}内容为空");
    }
    if !body.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'=') {
        bail!("{label}包含无效字符");
    }
    if body[..body.len().saturating_sub(2)].contains('=') {
        bail!("{label}的填充位置不正确");
    }
    Ok(body)
}

fn decode_json<T: DeserializeOwned>(body: &str, label: &str) -> Result<T> {
    let cleaned = body.trim_end_matches('=');
    let bytes = URL_SAFE_NO_PAD.decode(cleaned.as_bytes()).with_context(|| format!("{label}无法解码"))?;
    // `deny_unknown_fields` on every payload rejects extra fields such as
    // `customParam` or an arguments array before any process is spawned.
    serde_json::from_slice(&bytes).with_context(|| format!("{label}内容不完整或包含未知字段"))
}

/* ---------- server setup code ---------- */

pub fn encode_server_code(config: &ServerConfig) -> Result<String> {
    config.validate()?;
    let body = serde_json::to_vec(config)?;
    Ok(format!("{SERVER_PREFIX}{}", URL_SAFE_NO_PAD.encode(body)))
}

pub fn parse_server_code(input: &str) -> Result<ServerConfig> {
    let body = decode_body(input, SERVER_PREFIX, MAX_SERVER_CODE, "服务器设置码")?;
    let config: ServerConfig = decode_json(body, "服务器设置码")?;
    config.validate().context("服务器设置码无效")?;
    Ok(config)
}

/* ---------- rooms ---------- */

/// Persisted room selection. `seed` is the bearer key, so it is stored with
/// restricted file permissions and redacted from `Debug`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StoredRoom {
    /// 64 lowercase hex characters.
    pub seed: String,
    /// `host` or `join`.
    pub role: String,
    pub server: ServerConfig,
}

impl fmt::Debug for StoredRoom {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoredRoom")
            .field("seed", &"<redacted>")
            .field("role", &self.role)
            .field("server", &self.server)
            .finish()
    }
}

/// Fully derived, ready-to-spawn room parameters. Also secret-bearing.
pub struct RoomSpec {
    pub stored: StoredRoom,
    /// 64 lowercase hex; the stable identity used for history and the future mailbox.
    pub room_id: String,
    pub network_name: String,
    pub network_secret: String,
    pub virtual_ip: String,
    pub peer_ip: String,
}

impl fmt::Debug for RoomSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RoomSpec")
            .field("stored", &self.stored)
            .field("room_id", &self.room_id)
            .field("network_name", &self.network_name)
            .field("network_secret", &"<redacted>")
            .field("virtual_ip", &self.virtual_ip)
            .field("peer_ip", &self.peer_ip)
            .finish()
    }
}

impl StoredRoom {
    /// Create a room as host. The seed comes from the operating system CSPRNG.
    pub fn create(server: ServerConfig, role: &str) -> Result<Self> {
        server.validate()?;
        let mut seed = [0u8; 32];
        OsRng.fill_bytes(&mut seed);
        let room = Self { seed: hex::encode(seed), role: role.to_owned(), server };
        room.validate()?;
        Ok(room)
    }

    /// Import a peer's invite. Always joins; the invite never dictates a role,
    /// executable arguments or addresses.
    pub fn from_invite(code: &str) -> Result<Self> {
        let body = decode_body(code, INVITE_PREFIX, MAX_INVITE_CODE, "配对码")?;
        let invite: Invite = decode_json(body, "配对码")?;
        if invite.v != 1 {
            bail!("配对码版本不受支持，请升级应用");
        }
        let room = Self { seed: invite.seed, role: "join".into(), server: invite.server };
        room.validate().context("配对码无效")?;
        Ok(room)
    }

    pub fn invite_code(&self) -> Result<String> {
        self.validate()?;
        let invite = Invite { v: 1, seed: self.seed.clone(), server: self.server.clone() };
        Ok(format!("{INVITE_PREFIX}{}", URL_SAFE_NO_PAD.encode(serde_json::to_vec(&invite)?)))
    }

    pub fn validate(&self) -> Result<()> {
        if self.seed.len() != 64
            || !self.seed.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        {
            bail!("房间种子格式无效");
        }
        if self.role != "host" && self.role != "join" {
            bail!("房间角色无效");
        }
        self.server.validate()?;
        Ok(())
    }

    fn seed_bytes(&self) -> Result<[u8; 32]> {
        let bytes = hex::decode(&self.seed).context("房间种子格式无效")?;
        bytes.try_into().map_err(|_| anyhow::anyhow!("房间种子格式无效"))
    }

    pub fn spec(&self) -> Result<RoomSpec> {
        self.validate()?;
        let seed = self.seed_bytes()?;
        let room_id = hex::encode(mac(&seed, b"room-id"));
        let network_secret = hex::encode(mac(&seed, b"network-secret"));
        let ip = mac(&seed, b"virtual-ip");
        // A seed-derived private /24 keeps simultaneous rooms on different
        // addresses without asking the user for one. .1 is always host, .2 join.
        let subnet = 100 + u16::from(ip[0]) % 100;
        let host_ip = format!("10.{subnet}.{}.1", ip[1]);
        let join_ip = format!("10.{subnet}.{}.2", ip[1]);
        let (virtual_ip, peer_ip) = if self.role == "host" {
            (host_ip, join_ip)
        } else {
            (join_ip, host_ip)
        };
        let network_name = format!("{}-{room_id}", self.server.network_prefix);
        Ok(RoomSpec {
            stored: self.clone(),
            room_id,
            network_name,
            network_secret,
            virtual_ip,
            peer_ip,
        })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Invite {
    v: u8,
    seed: String,
    server: ServerConfig,
}

/// Purpose-separated HMAC-SHA256. Changing the label prefix invalidates every
/// previously derived value, so it is versioned explicitly.
fn mac(seed: &[u8; 32], purpose: &[u8]) -> [u8; 32] {
    let mut mac = Hmac::<Sha256>::new_from_slice(seed).expect("HMAC accepts 32-byte keys");
    mac.update(b"twinlink/v1\0");
    mac.update(purpose);
    mac.finalize().into_bytes().into()
}

#[cfg(test)]
pub(crate) fn test_server(prefix: &str) -> ServerConfig {
    ServerConfig {
        relay_url: "tcp://45.76.70.254:11010".into(),
        mailbox_url: "https://45.76.70.254:18443".into(),
        mailbox_ca_pem: test_pem(),
        access_token: "test-token-0123456789-abcdefghijklmnop".into(),
        network_prefix: prefix.into(),
    }
}

#[cfg(test)]
pub(crate) fn test_pem() -> String {
    // Public fixture only. The disposable generating private key is not in
    // the repository. Use real X.509 bytes so HTTP-client construction tests
    // exercise the TLS stack rather than accepting a DER-shaped placeholder.
    include_str!("../tests/fixtures/test-ca.pem").to_owned()
}
