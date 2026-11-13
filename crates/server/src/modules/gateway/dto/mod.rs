//! WebSocket bidirectional control frames and SSE replay DTOs.

use agents_protocol::{EventEnvelope, HandshakeRequest, HandshakeResponse};
use garde::Validate;
use ironic::OpenApiSchema;
use serde::{Deserialize, Serialize};

/// Query parameters for replaying or streaming session events after a given sequence number.
#[derive(Debug, Clone, Default, Deserialize, Validate, OpenApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct StreamReplayQuery {
    #[garde(skip)]
    pub after_seq: Option<u64>,
    #[garde(range(min = 1, max = 1000))]
    pub limit: Option<u32>,
    /// Optional token for browser WebSocket upgrades that cannot set custom headers.
    #[garde(length(max = 2048))]
    pub token: Option<String>,
}

/// Client-to-server WebSocket command frame.
#[derive(Debug, Clone, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum WsClientCommand {
    Handshake(HandshakeRequest),
    Replay { after_seq: u64 },
    Steer { task_id: String, content: String },
    FollowUp { content: String },
    Pause,
    Resume,
    Ping,
}

/// Server-to-client WebSocket response/event frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum WsServerFrame {
    Handshake(HandshakeResponse),
    Event(EventEnvelope),
    Ack { action: String, id: String },
    Error { code: String, message: String },
    Pong,
}
