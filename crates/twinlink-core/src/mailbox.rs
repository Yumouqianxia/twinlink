//! Private HTTPS mailbox protocol v1: plaintext validation and transport.
//!
//! The implementation follows `docs/SERVER-PROTOCOL.md` exactly. Only the
//! configured CA is *added* to the normal trust store; hostname and certificate
//! verification always stay enabled, HTTP is refused, redirects are never
//! followed, and response bodies are bounded. Authentication headers are marked
//! sensitive in the HTTP stack and never appear in errors or logs.

use crate::crypto::{self, peer_role, MAX_EVENT_ENVELOPE, TAG_LEN};
use crate::files::{validate_name, MAX_FILE_SIZE};
use crate::pairing::{RoomSpec, ServerConfig};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use reqwest::{
    header::{HeaderName, HeaderValue, AUTHORIZATION, CONTENT_TYPE},
    redirect::Policy,
    Certificate, Client, Method,
};
use serde::{Deserialize, Serialize};
use std::{fmt, sync::Arc, time::Duration};
use url::Url;
use uuid::Uuid;

pub const CHUNK_BYTES: u64 = 1_048_592;
pub const MAX_FILE_BYTES: u64 = 1_073_741_824;
pub const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_EVENT_PAGE: usize = 64;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const USER_AGENT: &str = "TwinLink/0.2";

/* ---------- strict plaintext bodies ---------- */

#[derive(Clone, Debug, PartialEq)]
pub enum Body {
    Message {
        id: String,
        text: String,
        timestamp: u64,
    },
    Receipt {
        message_id: String,
    },
    Offer {
        id: String,
        name: String,
        size: u64,
        hash: String,
    },
    FileComplete {
        id: String,
    },
    Cancel {
        id: String,
    },
    Reject {
        id: String,
    },
}

fn canonical_uuid(value: &str) -> Result<()> {
    let parsed = Uuid::parse_str(value).context("标识不是有效 UUID")?;
    if parsed.to_string() != value {
        bail!("标识不是规范 UUID");
    }
    Ok(())
}

fn exact_keys(map: &serde_json::Map<String, serde_json::Value>, expected: &[&str]) -> Result<()> {
    if map.len() != expected.len() || expected.iter().any(|key| !map.contains_key(*key)) {
        bail!("离线消息字段无效");
    }
    Ok(())
}

fn lower_hex_64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

impl Body {
    pub fn encode(&self) -> Result<Vec<u8>> {
        let value = match self {
            Body::Message {
                id,
                text,
                timestamp,
            } => serde_json::json!({
                "kind": "message",
                "message": { "id": id, "text": text, "timestamp": timestamp }
            }),
            Body::Receipt { message_id } => {
                serde_json::json!({ "kind": "receipt", "messageId": message_id })
            }
            Body::Offer {
                id,
                name,
                size,
                hash,
            } => serde_json::json!({
                "kind": "offer",
                "transfer": { "id": id, "name": name, "size": size, "hash": hash }
            }),
            Body::FileComplete { id } => serde_json::json!({ "kind": "fileComplete", "id": id }),
            Body::Cancel { id } => serde_json::json!({ "kind": "cancel", "id": id }),
            Body::Reject { id } => serde_json::json!({ "kind": "reject", "id": id }),
        };
        let bytes = serde_json::to_vec(&value)?;
        if bytes.len() > crypto::MAX_EVENT_PLAINTEXT {
            bail!("离线消息内容过大");
        }
        Ok(bytes)
    }

    /// Parse only the documented variants and reject extra fields. Validation
    /// happens before any local persistence or acknowledgement.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value: serde_json::Value =
            serde_json::from_slice(bytes).context("离线消息不是有效 JSON")?;
        let object = value.as_object().context("离线消息不是 JSON 对象")?;
        let kind = object
            .get("kind")
            .and_then(|value| value.as_str())
            .context("离线消息缺少类型")?;
        match kind {
            "message" => {
                exact_keys(object, &["kind", "message"])?;
                let message = object["message"].as_object().context("消息字段无效")?;
                exact_keys(message, &["id", "text", "timestamp"])?;
                let id = message["id"].as_str().context("消息标识无效")?.to_owned();
                canonical_uuid(&id)?;
                let text = message["text"]
                    .as_str()
                    .context("消息正文无效")?
                    .to_owned();
                if text.trim().is_empty() || text.len() > 16 * 1024 {
                    bail!("消息正文长度无效");
                }
                let timestamp = message["timestamp"].as_u64().context("消息时间无效")?;
                Ok(Body::Message {
                    id,
                    text,
                    timestamp,
                })
            }
            "receipt" => {
                exact_keys(object, &["kind", "messageId"])?;
                let message_id = object["messageId"]
                    .as_str()
                    .context("回执标识无效")?
                    .to_owned();
                canonical_uuid(&message_id)?;
                Ok(Body::Receipt { message_id })
            }
            "offer" => {
                exact_keys(object, &["kind", "transfer"])?;
                let transfer = object["transfer"].as_object().context("文件信息无效")?;
                exact_keys(transfer, &["id", "name", "size", "hash"])?;
                let id = transfer["id"].as_str().context("文件标识无效")?.to_owned();
                canonical_uuid(&id)?;
                let name = transfer["name"].as_str().context("文件名无效")?.to_owned();
                validate_name(&name)?;
                let size = transfer["size"].as_u64().context("文件大小无效")?;
                if size > MAX_FILE_SIZE.min(MAX_FILE_BYTES) {
                    bail!("文件超过 1 GiB 限制");
                }
                let hash = transfer["hash"]
                    .as_str()
                    .context("文件校验值无效")?
                    .to_owned();
                if !lower_hex_64(&hash) {
                    bail!("文件校验值无效");
                }
                Ok(Body::Offer {
                    id,
                    name,
                    size,
                    hash,
                })
            }
            "fileComplete" | "cancel" | "reject" => {
                exact_keys(object, &["kind", "id"])?;
                let id = object["id"].as_str().context("文件标识无效")?.to_owned();
                canonical_uuid(&id)?;
                Ok(match kind {
                    "fileComplete" => Body::FileComplete { id },
                    "cancel" => Body::Cancel { id },
                    _ => Body::Reject { id },
                })
            }
            _ => bail!("不支持的离线消息类型"),
        }
    }
}

/* ---------- wire JSON ---------- */

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IncomingEvent {
    pub id: String,
    pub sender: String,
    pub recipient: String,
    pub payload: String,
    pub created_at: u64,
    pub expires_at: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EventPage {
    events: Vec<IncomingEvent>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedEvent {
    pub id: String,
    pub created_at: u64,
    pub expires_at: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegisteredRoom {
    room_id: String,
    ttl_seconds: u64,
    chunk_bytes: u64,
    max_file_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileManifest {
    pub id: String,
    pub sender: String,
    pub encrypted_size: u64,
    pub chunk_count: u64,
    pub chunk_bytes: u64,
    pub received: Vec<u64>,
    pub complete: bool,
    pub expires_at: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChunkStored {
    index: u64,
    sha256: String,
}

/* ---------- errors ---------- */

#[derive(Clone, Debug)]
pub struct MailboxError {
    pub status: Option<u16>,
    pub code: String,
    pub message: String,
}

impl MailboxError {
    fn network(message: impl Into<String>) -> Self {
        Self {
            status: None,
            code: "network".into(),
            message: message.into(),
        }
    }
    fn protocol(message: impl Into<String>) -> Self {
        Self {
            status: None,
            code: "protocol".into(),
            message: message.into(),
        }
    }
    pub(crate) fn api(status: u16, code: impl Into<String>) -> Self {
        let code = code.into();
        // Treat an error body as untrusted input too: it must never echo a
        // credential, arbitrary HTML or a remote pathname into the UI/logs.
        // A charset/length filter alone would still expose a reflected 64-hex
        // room token. Only display the documented, fixed machine codes.
        let code = if matches!(code.as_str(),
            "invalid_request" | "invalid_id" | "invalid_room" | "room_forbidden"
            | "invalid_room_token" | "room_exists" | "room_limit" | "unauthorized"
            | "invalid_payload" | "payload_size" | "noncanonical_payload" | "event_conflict"
            | "queue_full" | "file_missing_or_expired" | "file_size" | "invalid_last_chunk"
            | "file_conflict" | "storage_full" | "disk_space" | "invalid_chunk_index"
            | "chunk_size" | "chunk_conflict" | "already_complete" | "upload_incomplete"
            | "content_length_required" | "invalid_content_length" | "body_size"
            | "incomplete_body" | "json_required" | "invalid_json" | "not_found"
            | "sender_required" | "chunk_missing" | "stored_chunk_corrupt" | "internal_error"
            | "redirect_refused" | "request_failed" | "quota_exceeded" | "rate_limited") {
            code
        } else { "request_failed".into() };
        Self {
            message: format!("离线信箱返回 {status}（{code}）"),
            status: Some(status),
            code,
        }
    }
    pub fn retryable(&self) -> bool {
        match self.status {
            None => !matches!(self.code.as_str(), "protocol" | "spool"),
            Some(status) => {
                status == 408 || status == 425 || status == 429 || (status >= 500 && status != 507)
            }
        }
    }
    pub fn is_auth(&self) -> bool {
        matches!(self.status, Some(401 | 403))
    }
    pub fn is_missing(&self) -> bool {
        self.status == Some(404)
    }
    pub fn is_conflict(&self) -> bool {
        self.status == Some(409)
    }
    pub fn is_quota(&self) -> bool {
        self.status == Some(507) || self.code == "quota_exceeded"
    }
}

impl fmt::Display for MailboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for MailboxError {}

/* ---------- transport ---------- */

#[derive(Clone)]
pub struct HttpRequest {
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl fmt::Debug for HttpRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpRequest")
            .field("method", &self.method)
            .field("url", &self.url)
            .field("header_names", &self.headers.iter().map(|(name, _)| name).collect::<Vec<_>>())
            .field("body_bytes", &self.body.len())
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

#[async_trait]
pub trait Transport: Send + Sync {
    async fn request(&self, request: HttpRequest) -> Result<HttpResponse, MailboxError>;
}

struct HttpTransport {
    client: Client,
}

impl HttpTransport {
    fn new(server: &ServerConfig) -> Result<Self, MailboxError> {
        if !server.mailbox_url.starts_with("https://") {
            return Err(MailboxError::protocol("离线信箱必须使用 HTTPS"));
        }
        let certificate = Certificate::from_pem(server.mailbox_ca_pem.as_bytes())
            .map_err(|_| MailboxError::protocol("服务器 CA 证书无效"))?;
        let client = Client::builder()
            .use_rustls_tls()
            .https_only(true)
            .redirect(Policy::none())
            .timeout(REQUEST_TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT)
            // Keep the platform/webpki roots AND add the configured deployment
            // CA. Certificate-chain and hostname verification stay enforced.
            .tls_built_in_root_certs(true)
            .add_root_certificate(certificate)
            .user_agent(USER_AGENT)
            .build()
            .map_err(|_| MailboxError::protocol("无法初始化离线信箱连接"))?;
        Ok(Self { client })
    }
}

fn sensitive_header(name: &str) -> bool {
    name.eq_ignore_ascii_case("authorization") || name.eq_ignore_ascii_case("x-room-token")
}

#[async_trait]
impl Transport for HttpTransport {
    async fn request(&self, request: HttpRequest) -> Result<HttpResponse, MailboxError> {
        let method = Method::from_bytes(request.method.as_bytes())
            .map_err(|_| MailboxError::protocol("请求方法无效"))?;
        let mut builder = self.client.request(method, &request.url);
        for (name, value) in &request.headers {
            let header = HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| MailboxError::protocol("请求头无效"))?;
            let mut value =
                HeaderValue::from_str(value).map_err(|_| MailboxError::protocol("请求头无效"))?;
            if sensitive_header(name) {
                value.set_sensitive(true);
            }
            builder = builder.header(header, value);
        }
        if !request.body.is_empty() || request.method == "POST" || request.method == "PUT" {
            builder = builder.body(request.body);
        }
        let mut response = builder.send().await.map_err(|error| {
            if error.is_timeout() {
                MailboxError::network("连接离线信箱超时")
            } else if error.is_connect() {
                MailboxError::network("无法连接离线信箱（网络或证书校验失败）")
            } else {
                MailboxError::network("离线信箱请求失败")
            }
        })?;
        let status = response.status().as_u16();
        if (300..400).contains(&status) {
            return Err(MailboxError::api(status, "redirect_refused"));
        }
        if let Some(length) = response.content_length() {
            if length > MAX_RESPONSE_BYTES as u64 {
                return Err(MailboxError::protocol("离线信箱响应过大"));
            }
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| MailboxError::network("读取离线信箱响应失败"))?
        {
            if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
                return Err(MailboxError::protocol("离线信箱响应过大"));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(HttpResponse { status, body })
    }
}

/* ---------- client ---------- */

#[derive(Clone)]
pub struct MailboxClient {
    base: Url,
    access_token: String,
    room_token: String,
    pub room_id: String,
    pub role: String,
    transport: Arc<dyn Transport>,
}

impl fmt::Debug for MailboxClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MailboxClient")
            .field("base", &self.base)
            .field("room_id", &self.room_id)
            .field("role", &self.role)
            .field("access_token", &"<redacted>")
            .field("room_token", &"<redacted>")
            .finish()
    }
}

impl MailboxClient {
    pub fn new(server: &ServerConfig, spec: &RoomSpec) -> Result<Self, MailboxError> {
        server.validate().map_err(|_| MailboxError::protocol("服务器配置无效"))?;
        crypto::validate_role(&spec.stored.role)
            .map_err(|_| MailboxError::protocol("房间角色无效"))?;
        if server.access_token.len() < 32 {
            return Err(MailboxError::protocol("服务器访问令牌长度无效"));
        }
        if !lower_hex_64(&spec.room_id) {
            return Err(MailboxError::protocol("房间标识无效"));
        }
        let base =
            Url::parse(&server.mailbox_url).map_err(|_| MailboxError::protocol("离线信箱地址无效"))?;
        if base.scheme() != "https"
            || base.host().is_none()
            || base.path() != "/"
            || base.query().is_some()
            || base.fragment().is_some()
            || !base.username().is_empty()
            || base.password().is_some()
        {
            return Err(MailboxError::protocol("离线信箱地址无效"));
        }
        let room_token = crypto::room_token(&spec.stored.seed)
            .map_err(|_| MailboxError::protocol("无法派生房间令牌"))?;
        Ok(Self {
            base,
            access_token: server.access_token.clone(),
            room_token,
            room_id: spec.room_id.clone(),
            role: spec.stored.role.clone(),
            transport: Arc::new(HttpTransport::new(server)?),
        })
    }

    #[cfg(test)]
    pub(crate) fn with_transport(
        base: &str,
        access_token: &str,
        spec: &RoomSpec,
        transport: Arc<dyn Transport>,
    ) -> Self {
        Self {
            base: Url::parse(base).unwrap(),
            access_token: access_token.into(),
            room_token: crypto::room_token(&spec.stored.seed).unwrap(),
            room_id: spec.room_id.clone(),
            role: spec.stored.role.clone(),
            transport,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base.as_str().trim_end_matches('/'), path)
    }

    fn auth_headers(&self, room: bool) -> Vec<(String, String)> {
        let mut headers = vec![(
            AUTHORIZATION.as_str().to_owned(),
            format!("Bearer {}", self.access_token),
        )];
        if room {
            headers.push(("X-Room-Token".into(), self.room_token.clone()));
        }
        headers
    }

    async fn call(
        &self,
        method: &'static str,
        path: &str,
        authenticated: bool,
        room: bool,
        body: Option<Vec<u8>>,
    ) -> Result<HttpResponse, MailboxError> {
        let mut headers = if authenticated {
            self.auth_headers(room)
        } else {
            Vec::new()
        };
        if body.is_some() {
            headers.push((CONTENT_TYPE.as_str().to_owned(), "application/json".to_owned()));
        }
        let response = self
            .transport
            .request(HttpRequest {
                method,
                url: self.url(path),
                headers,
                body: body.unwrap_or_default(),
            })
            .await?;
        if !(200..300).contains(&response.status) {
            let code = serde_json::from_slice::<serde_json::Value>(&response.body)
                .ok()
                .and_then(|value| value.get("error").and_then(|v| v.as_str()).map(str::to_owned))
                .filter(|code| {
                    !code.is_empty()
                        && code.len() <= 64
                        && code
                            .bytes()
                            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
                })
                .unwrap_or_else(|| "request_failed".into());
            return Err(MailboxError::api(response.status, code));
        }
        Ok(response)
    }

    fn json<T: for<'de> Deserialize<'de>>(response: &HttpResponse) -> Result<T, MailboxError> {
        serde_json::from_slice(&response.body)
            .map_err(|_| MailboxError::protocol("离线信箱响应格式无效"))
    }

    pub async fn register(&self) -> Result<(), MailboxError> {
        let body = serde_json::to_vec(&serde_json::json!({
            "roomId": self.room_id,
            "roomToken": self.room_token,
        }))
        .map_err(|_| MailboxError::protocol("无法编码房间注册请求"))?;
        let response = self.call("POST", "/v1/rooms", true, false, Some(body)).await?;
        let room: RegisteredRoom = Self::json(&response)?;
        if room.room_id != self.room_id
            || room.ttl_seconds == 0
            || room.chunk_bytes != CHUNK_BYTES
            || room.max_file_bytes < MAX_FILE_BYTES
        {
            return Err(MailboxError::protocol("离线信箱协议版本不兼容"));
        }
        Ok(())
    }

    pub async fn health(&self) -> Result<(), MailboxError> {
        let response = self.call("GET", "/health", false, false, None).await?;
        let value: serde_json::Value = Self::json(&response)?;
        if value.get("service").and_then(|v| v.as_str()) != Some("twinlink-mailbox")
            || value.get("version").and_then(|v| v.as_u64()) != Some(1) {
            return Err(MailboxError::protocol("离线信箱服务不兼容"));
        }
        Ok(())
    }

    pub async fn post_event(
        &self,
        event_id: &str,
        payload: &[u8],
    ) -> Result<CreatedEvent, MailboxError> {
        canonical_uuid(event_id).map_err(|_| MailboxError::protocol("事件标识无效"))?;
        if !(1 + crypto::EVENT_NONCE_LEN + TAG_LEN..=MAX_EVENT_ENVELOPE).contains(&payload.len()) {
            return Err(MailboxError::protocol("离线事件密文长度无效"));
        }
        let recipient =
            peer_role(&self.role).map_err(|_| MailboxError::protocol("房间角色无效"))?;
        let envelope = URL_SAFE_NO_PAD.encode(payload);
        let body = serde_json::to_vec(&serde_json::json!({
            "id": event_id,
            "sender": self.role,
            "recipient": recipient,
            "payload": envelope,
        }))
        .map_err(|_| MailboxError::protocol("无法编码离线消息"))?;
        let response = self
            .call(
                "POST",
                &format!("/v1/rooms/{}/events", self.room_id),
                true,
                true,
                Some(body),
            )
            .await?;
        let created: CreatedEvent = Self::json(&response)?;
        if created.id != event_id || created.expires_at <= created.created_at {
            return Err(MailboxError::protocol("离线信箱回执无效"));
        }
        Ok(created)
    }

    pub async fn list_events(&self) -> Result<Vec<IncomingEvent>, MailboxError> {
        let path = format!("/v1/rooms/{}/events?recipient={}", self.room_id, self.role);
        let response = self.call("GET", &path, true, true, None).await?;
        let page: EventPage = Self::json(&response)?;
        if page.events.len() > MAX_EVENT_PAGE {
            return Err(MailboxError::protocol("离线信箱事件分页过大"));
        }
        for event in &page.events {
            if canonical_uuid(&event.id).is_err()
                || event.sender != peer_role(&self.role).unwrap_or("")
                || event.recipient != self.role
                || event.expires_at <= event.created_at
            {
                return Err(MailboxError::protocol("离线信箱事件字段无效"));
            }
            let envelope = URL_SAFE_NO_PAD
                .decode(&event.payload)
                .map_err(|_| MailboxError::protocol("离线消息编码无效"))?;
            if !(1 + crypto::EVENT_NONCE_LEN + TAG_LEN..=MAX_EVENT_ENVELOPE).contains(&envelope.len()) {
                return Err(MailboxError::protocol("离线消息超过限制"));
            }
        }
        Ok(page.events)
    }

    pub async fn delete_event(&self, event_id: &str) -> Result<(), MailboxError> {
        canonical_uuid(event_id).map_err(|_| MailboxError::protocol("事件标识无效"))?;
        let path = format!(
            "/v1/rooms/{}/events/{}?recipient={}",
            self.room_id, event_id, self.role
        );
        self.call("DELETE", &path, true, true, None).await?;
        Ok(())
    }

    pub async fn declare_file(
        &self,
        id: &str,
        encrypted_size: u64,
        chunk_count: u64,
    ) -> Result<FileManifest, MailboxError> {
        canonical_uuid(id).map_err(|_| MailboxError::protocol("文件标识无效"))?;
        let body = serde_json::to_vec(&serde_json::json!({
            "id": id,
            "sender": self.role,
            "encryptedSize": encrypted_size,
            "chunkCount": chunk_count,
        }))
        .map_err(|_| MailboxError::protocol("无法编码文件声明"))?;
        let response = self
            .call(
                "POST",
                &format!("/v1/rooms/{}/files", self.room_id),
                true,
                true,
                Some(body),
            )
            .await?;
        let manifest = Self::manifest_from(response, id, encrypted_size, chunk_count)?;
        if manifest.sender != self.role {
            return Err(MailboxError::protocol("服务器文件所属角色不一致"));
        }
        Ok(manifest)
    }

    pub async fn file_manifest(&self, id: &str) -> Result<FileManifest, MailboxError> {
        canonical_uuid(id).map_err(|_| MailboxError::protocol("文件标识无效"))?;
        let response = self
            .call(
                "GET",
                &format!("/v1/rooms/{}/files/{}", self.room_id, id),
                true,
                true,
                None,
            )
            .await?;
        let manifest: FileManifest = Self::json(&response)?;
        Self::validate_manifest(&manifest)?;
        if manifest.id != id {
            return Err(MailboxError::protocol("服务器文件清单标识不一致"));
        }
        Ok(manifest)
    }

    fn manifest_from(
        response: HttpResponse,
        id: &str,
        encrypted_size: u64,
        chunk_count: u64,
    ) -> Result<FileManifest, MailboxError> {
        let manifest: FileManifest = Self::json(&response)?;
        Self::validate_manifest(&manifest)?;
        if manifest.id != id
            || manifest.encrypted_size != encrypted_size
            || manifest.chunk_count != chunk_count
            || !matches!(manifest.sender.as_str(), "host" | "join")
        {
            return Err(MailboxError::protocol("服务器文件清单与声明不一致"));
        }
        Ok(manifest)
    }

    fn validate_manifest(manifest: &FileManifest) -> Result<(), MailboxError> {
        const MAX_CHUNKS: u64 = MAX_FILE_BYTES / crypto::FILE_CHUNK_BYTES as u64;
        // Check counts before multiplication/subtraction, including hostile
        // u64 values. An empty file is one authenticated 16-byte block.
        if canonical_uuid(&manifest.id).is_err()
            || !matches!(manifest.sender.as_str(), "host" | "join")
            || manifest.chunk_bytes != CHUNK_BYTES
            || manifest.chunk_count == 0
            || manifest.chunk_count > MAX_CHUNKS
            || manifest.encrypted_size < TAG_LEN as u64
            || manifest.encrypted_size > MAX_FILE_BYTES + manifest.chunk_count * TAG_LEN as u64
            || manifest.encrypted_size.div_ceil(CHUNK_BYTES) != manifest.chunk_count
            || manifest.encrypted_size - (manifest.chunk_count - 1) * CHUNK_BYTES < TAG_LEN as u64
            || manifest.received.len() > manifest.chunk_count as usize
            || manifest
                .received
                .iter()
                .any(|index| *index >= manifest.chunk_count)
        {
            return Err(MailboxError::protocol("服务器文件清单无效"));
        }
        let indices: std::collections::HashSet<_> = manifest.received.iter().copied().collect();
        if indices.len() != manifest.received.len()
            || (manifest.complete && indices.len() != manifest.chunk_count as usize) {
            return Err(MailboxError::protocol("服务器文件分块清单无效"));
        }
        Ok(())
    }

    pub async fn put_chunk(
        &self,
        id: &str,
        index: u64,
        ciphertext: &[u8],
    ) -> Result<(), MailboxError> {
        canonical_uuid(id).map_err(|_| MailboxError::protocol("文件标识无效"))?;
        if !(TAG_LEN..=CHUNK_BYTES as usize).contains(&ciphertext.len())
            || index >= MAX_FILE_BYTES / crypto::FILE_CHUNK_BYTES as u64 {
            return Err(MailboxError::protocol("文件分块过大"));
        }
        let mut headers = self.auth_headers(true);
        headers.push((
            CONTENT_TYPE.as_str().to_owned(),
            "application/octet-stream".to_owned(),
        ));
        headers.push(("X-Sender-Role".into(), self.role.clone()));
        let response = self
            .transport
            .request(HttpRequest {
                method: "PUT",
                url: self.url(&format!(
                    "/v1/rooms/{}/files/{}/chunks/{}",
                    self.room_id, id, index
                )),
                headers,
                body: ciphertext.to_vec(),
            })
            .await?;
        if !(200..300).contains(&response.status) {
            let code = serde_json::from_slice::<serde_json::Value>(&response.body)
                .ok()
                .and_then(|value| value.get("error").and_then(|v| v.as_str()).map(str::to_owned))
                .unwrap_or_else(|| "request_failed".into());
            return Err(MailboxError::api(response.status, code));
        }
        let stored: ChunkStored = Self::json(&response)?;
        let digest = crypto::sha256_hex(ciphertext);
        if stored.index != index || stored.sha256 != digest {
            return Err(MailboxError::protocol("服务器文件分块回执不一致"));
        }
        Ok(())
    }

    pub async fn complete_file(&self, id: &str) -> Result<FileManifest, MailboxError> {
        canonical_uuid(id).map_err(|_| MailboxError::protocol("文件标识无效"))?;
        let response = self
            .call(
                "POST",
                &format!("/v1/rooms/{}/files/{}/complete", self.room_id, id),
                true,
                true,
                Some(Vec::new()),
            )
            .await?;
        let manifest: FileManifest = Self::json(&response)?;
        Self::validate_manifest(&manifest)?;
        if !manifest.complete || manifest.id != id || manifest.sender != self.role {
            return Err(MailboxError::protocol("服务器未确认文件完整"));
        }
        Ok(manifest)
    }

    pub async fn get_chunk(&self, id: &str, index: u64) -> Result<Vec<u8>, MailboxError> {
        canonical_uuid(id).map_err(|_| MailboxError::protocol("文件标识无效"))?;
        if index >= MAX_FILE_BYTES / crypto::FILE_CHUNK_BYTES as u64 {
            return Err(MailboxError::protocol("文件分块序号无效"));
        }
        let response = self
            .call(
                "GET",
                &format!("/v1/rooms/{}/files/{}/chunks/{}", self.room_id, id, index),
                true,
                true,
                None,
            )
            .await?;
        if response.body.len() > CHUNK_BYTES as usize {
            return Err(MailboxError::protocol("服务器文件分块过大"));
        }
        Ok(response.body)
    }

    pub async fn delete_file(&self, id: &str) -> Result<(), MailboxError> {
        canonical_uuid(id).map_err(|_| MailboxError::protocol("文件标识无效"))?;
        self.call(
            "DELETE",
            &format!("/v1/rooms/{}/files/{}", self.room_id, id),
            true,
            true,
            None,
        )
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Noop;

    #[async_trait]
    impl Transport for Noop {
        async fn request(&self, _request: HttpRequest) -> Result<HttpResponse, MailboxError> {
            Err(MailboxError::network("noop"))
        }
    }

    fn body(value: serde_json::Value) -> Vec<u8> {
        serde_json::to_vec(&value).unwrap()
    }

    #[test]
    fn rejects_extra_or_malformed_plaintext_fields() {
        let message = body(serde_json::json!({
            "kind":"message",
            "message":{"id":"11111111-1111-4111-8111-111111111111","text":"hi","timestamp":1},
            "extra": true
        }));
        assert!(Body::decode(&message).is_err());
        let message = body(serde_json::json!({
            "kind":"message",
            "message":{"id":"not-a-uuid","text":"hi","timestamp":1}
        }));
        assert!(Body::decode(&message).is_err());
        let offer = body(serde_json::json!({
            "kind":"offer",
            "transfer":{"id":"11111111-1111-4111-8111-111111111111","name":"../x","size":1,"hash":"0".repeat(64)}
        }));
        assert!(Body::decode(&offer).is_err());
        let offer = body(serde_json::json!({
            "kind":"offer",
            "transfer":{"id":"11111111-1111-4111-8111-111111111111","name":"x.txt","size":MAX_FILE_BYTES + 1,"hash":"0".repeat(64)}
        }));
        assert!(Body::decode(&offer).is_err());
    }

    #[test]
    fn message_body_round_trips() {
        let body = Body::Message {
            id: "11111111-1111-4111-8111-111111111111".into(),
            text: "你好".into(),
            timestamp: 42,
        };
        assert_eq!(Body::decode(&body.encode().unwrap()).unwrap(), body);
    }

    #[test]
    fn production_client_refuses_plain_http() {
        let server = crate::pairing::test_server("twinlink-mailbox-http-test");
        let mut room = crate::pairing::StoredRoom::create(server, "host").unwrap();
        let spec = room.spec().unwrap();
        room.server.mailbox_url = "http://127.0.0.1:9".into();
        assert!(MailboxClient::new(&room.server, &spec).is_err());
    }

    #[test]
    fn debug_never_prints_tokens() {
        let server = crate::pairing::test_server("twinlink-mailbox-debug-test");
        let room = crate::pairing::StoredRoom::create(server, "host").unwrap();
        let spec = room.spec().unwrap();
        let client =
            MailboxClient::with_transport("https://127.0.0.1:9/", "token", &spec, Arc::new(Noop));
        let debug = format!("{client:?}");
        assert!(!debug.contains(&room.server.access_token));
        assert!(!debug.contains(&client.room_token));
        assert!(debug.contains("redacted"));
    }

    #[test]
    fn file_manifest_validation_bounds() {
        let valid = FileManifest {
            id: "22222222-2222-4222-8222-222222222222".into(),
            sender: "host".into(),
            encrypted_size: 1_048_592,
            chunk_count: 1,
            chunk_bytes: CHUNK_BYTES,
            received: vec![0],
            complete: false,
            expires_at: 1,
        };
        assert!(MailboxClient::validate_manifest(&valid).is_ok());
        let mut invalid = valid.clone();
        invalid.received = vec![1];
        assert!(MailboxClient::validate_manifest(&invalid).is_err());
        let mut invalid = valid;
        invalid.chunk_bytes = 123;
        assert!(MailboxClient::validate_manifest(&invalid).is_err());
    }

    #[test]
    fn rejects_hostile_counts_duplicate_parts_and_incomplete_completion() {
        let valid = FileManifest {
            id: "22222222-2222-4222-8222-222222222222".into(), sender: "host".into(),
            encrypted_size: 16, chunk_count: 1, chunk_bytes: CHUNK_BYTES,
            received: vec![], complete: false, expires_at: 1,
        };
        for (size, count) in [(u64::MAX, u64::MAX), (16, u64::MAX), (0, 1), (CHUNK_BYTES + 1, 2)] {
            let mut m = valid.clone(); m.encrypted_size = size; m.chunk_count = count;
            assert!(MailboxClient::validate_manifest(&m).is_err());
        }
        let mut m = valid.clone(); m.received = vec![0, 0];
        assert!(MailboxClient::validate_manifest(&m).is_err());
        let mut m = valid; m.complete = true;
        assert!(MailboxClient::validate_manifest(&m).is_err());
        m.received = vec![0];
        assert!(MailboxClient::validate_manifest(&m).is_ok());
    }

    #[test]
    fn debug_and_remote_errors_cannot_echo_credentials_or_bodies() {
        let request = HttpRequest {
            method: "POST", url: "https://example.invalid/v1/rooms".into(),
            headers: vec![("Authorization".into(), "Bearer DO-NOT-PRINT-TOKEN".into()),
                          ("X-Room-Token".into(), "DO-NOT-PRINT-ROOM".into())],
            body: b"DO-NOT-PRINT-CONTENT".to_vec(),
        };
        let debug = format!("{request:?}");
        assert!(!debug.contains("DO-NOT-PRINT"));
        for text in ["Bearer DO-NOT-PRINT-TOKEN", "<html>internal page</html>", "", &"a".repeat(100), &"b".repeat(64)] {
            let error = MailboxError::api(500, text);
            assert_eq!(error.code, "request_failed");
        }
        assert!(!MailboxError::api(507, "storage_full").retryable());
        assert!(!MailboxError::protocol("invalid").retryable());
        assert!(MailboxError::api(500, "internal_error").retryable());
        assert!(MailboxError::api(429, "rate_limited").retryable());
    }
}
