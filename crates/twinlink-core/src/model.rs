use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub display_name: String,
    pub download_dir: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: String,
    pub text: String,
    pub outgoing: bool,
    pub timestamp: u64,
    /// `pending` (only on this device), `stored` (encrypted mailbox copy
    /// accepted) or `delivered` (peer acknowledged). Incoming messages are
    /// always delivered; legacy rows without acknowledgement metadata are
    /// `unknown` and must never be promoted on deserialization.
    #[serde(default = "default_delivery")]
    pub delivery: String,
    /// `direct` or `mailbox`; absent while still pending locally.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
}

fn default_delivery() -> String {
    // Phase 1 had no durable acknowledgement. Missing metadata is not proof
    // that a legacy outgoing message reached the peer; never invent a receipt.
    "unknown".into()
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPage {
    pub room_id: String,
    pub messages: Vec<Message>,
    pub has_more: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transfer {
    pub id: String,
    pub name: String,
    pub size: u64,
    pub transferred: u64,
    pub outgoing: bool,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Retry countdown shown by the UI while the supervisor backs off. Attempt 1 is
/// the first retry after the initial failure.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconnectStatus {
    pub attempt: u32,
    pub next_retry_ms: u64,
}

/// Non-secret room identity shown in the room-details panel.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomInfo {
    pub role: String,
    pub room_id: String,
    pub network_name: String,
    pub virtual_ip: String,
    pub peer_virtual_ip: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub profile: Profile,
    pub connection: String,
    pub peer_name: Option<String>,
    pub messages: Vec<Message>,
    pub transfers: Vec<Transfer>,
    pub engine_available: bool,
    pub preview: bool,
    /// Persisted manual disconnect: automatic reconnect is suppressed.
    pub paused: bool,
    pub reconnect: Option<ReconnectStatus>,
    /// Non-secret configured server, if one was imported.
    pub server: Option<crate::pairing::ServerSummary>,
    /// Selected room and its derived addresses. Never contains the seed.
    pub room: Option<RoomInfo>,
    /// Direct/relay is only real with an EasyTier RPC/CLI answer; until that is
    /// wired up the truthful value is "unknown" and the UI must not guess.
    pub route: String,
    /// Offline mailbox reachability, deliberately separate from peer
    /// reachability. Server availability is not evidence the peer is online.
    pub mailbox: MailboxInfo,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MailboxInfo {
    /// `notConfigured`, `connecting`, `online`, `retrying`, `error`, `paused`.
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub pending: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AppEvent {
    State {
        connection: String,
        #[serde(rename = "peerName")]
        peer_name: Option<String>,
        paused: bool,
        reconnect: Option<ReconnectStatus>,
        room: Option<RoomInfo>,
    },
    Server {
        server: Option<crate::pairing::ServerSummary>,
    },
    Message {
        message: Message,
    },
    Transfer {
        transfer: Transfer,
    },
    Mailbox {
        state: String,
        message: Option<String>,
        pending: usize,
    },
    Error {
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_outgoing_history_does_not_invent_delivery_receipts() {
        let message: Message = serde_json::from_value(serde_json::json!({
            "id": "11111111-1111-4111-8111-111111111111",
            "text": "legacy", "outgoing": true, "timestamp": 1
        })).unwrap();
        assert_eq!(message.delivery, "unknown");
        assert!(message.via.is_none());
    }
}
