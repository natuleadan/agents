//! Gateway business service handling handshake negotiation, stream replay, live subscriptions,
//! steering routing, and pause/resume reconciliation.

use std::sync::Arc;

use agents_protocol::{
    API_GENERATION, Capability, CodecId, EventEnvelope, HandshakeRequest, HandshakeResponse,
    StreamPayload,
};
use ironic::prelude::*;
use tokio::sync::broadcast;

use crate::{
    infra::{
        error::ServerError,
        stream::{BusClient, StreamStore, emit_stream_event},
    },
    modules::{
        auth::entities::AuthenticatedActor,
        gateway::dto::{WsClientCommand, WsServerFrame},
        sessions::services::SessionService,
        tasks::{
            dto::{FollowUpDto, SteerTaskDto},
            services::TaskService,
        },
    },
};

/// Coordinates WebSocket rooms, SSE streams, and reconnect reconciliation.
#[derive(Injectable)]
pub struct GatewayService {
    sessions: Arc<SessionService>,
    tasks: Arc<TaskService>,
    streams: Arc<StreamStore>,
    bus: Arc<BusClient>,
}

impl GatewayService {
    /// Negotiates protocol generation, codecs, and server capabilities.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Validation`] when the requested API generation is unsupported.
    pub fn negotiate_handshake(
        &self,
        req: &HandshakeRequest,
    ) -> Result<HandshakeResponse, ServerError> {
        let supported_gen = API_GENERATION;
        if req.api_generation != supported_gen {
            return Err(ServerError::Validation(format!(
                "unsupported API generation {}; server supports {supported_gen}",
                req.api_generation
            )));
        }

        let supported_codecs = [CodecId::EventsV1, CodecId::SnapshotV1];
        let active_codecs: Vec<CodecId> = if req.requested_codecs.is_empty() {
            supported_codecs.to_vec()
        } else {
            let matched: Vec<CodecId> = req
                .requested_codecs
                .iter()
                .filter(|c| supported_codecs.contains(c))
                .cloned()
                .collect();
            if matched.is_empty() {
                vec![CodecId::EventsV1]
            } else {
                matched
            }
        };

        let supported_caps = [
            Capability::Streaming,
            Capability::LiveSteering,
            Capability::Swarm,
            Capability::McpHost,
            Capability::Autocron,
            Capability::TraefikPreview,
        ];
        let capabilities: Vec<Capability> = if req.requested_capabilities.is_empty() {
            supported_caps.to_vec()
        } else {
            req.requested_capabilities
                .iter()
                .filter(|cap| supported_caps.contains(cap))
                .cloned()
                .collect()
        };

        Ok(HandshakeResponse {
            server_version: env!("CARGO_PKG_VERSION").to_owned(),
            api_generation: supported_gen,
            min_supported_generation: supported_gen,
            active_codecs,
            capabilities,
        })
    }

    /// Replays persisted `EventEnvelope`s strictly newer than `after_seq` for an owned session.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Forbidden`] on cross-user access or [`ServerError::NotFound`].
    pub async fn replay_events(
        &self,
        actor: &AuthenticatedActor,
        session_id: &str,
        after_seq: u64,
        limit: u32,
    ) -> Result<Vec<EventEnvelope>, ServerError> {
        let _ = self.sessions.get_owned(actor, session_id).await?;
        self.streams
            .0
            .list_after_seq(session_id, after_seq, limit)
            .await
    }

    /// Emits a new ordered event envelope into a session stream.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Forbidden`] or [`ServerError::Internal`].
    pub async fn emit_event(
        &self,
        actor: &AuthenticatedActor,
        session_id: &str,
        task_id: Option<&str>,
        payload: StreamPayload,
    ) -> Result<EventEnvelope, ServerError> {
        let _ = self.sessions.get_owned(actor, session_id).await?;
        emit_stream_event(&self.streams, &self.bus, session_id, task_id, payload).await
    }

    /// Verifies ownership and returns a live broadcast receiver for session events.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Forbidden`] or [`ServerError::NotFound`].
    pub async fn subscribe_session(
        &self,
        actor: &AuthenticatedActor,
        session_id: &str,
    ) -> Result<broadcast::Receiver<EventEnvelope>, ServerError> {
        let _ = self.sessions.get_owned(actor, session_id).await?;
        Ok(self.bus.0.subscribe_session_stream(session_id).await)
    }

    /// Processes a single incoming WebSocket client command.
    pub async fn handle_ws_command(
        &self,
        actor: &AuthenticatedActor,
        session_id: &str,
        cmd: WsClientCommand,
        paused: &mut bool,
        last_delivered_seq: &mut u64,
    ) -> Vec<WsServerFrame> {
        match cmd {
            WsClientCommand::Handshake(req) => match self.negotiate_handshake(&req) {
                Ok(resp) => vec![WsServerFrame::Handshake(resp)],
                Err(err) => vec![WsServerFrame::Error {
                    code: "unsupported_generation".to_owned(),
                    message: err.to_string(),
                }],
            },
            WsClientCommand::Replay { after_seq } => {
                match self.replay_events(actor, session_id, after_seq, 500).await {
                    Ok(events) => events
                        .into_iter()
                        .map(|ev| {
                            *last_delivered_seq = (*last_delivered_seq).max(ev.seq);
                            WsServerFrame::Event(ev)
                        })
                        .collect(),
                    Err(err) => vec![WsServerFrame::Error {
                        code: err.rpc_code().as_str().to_owned(),
                        message: err.to_string(),
                    }],
                }
            }
            WsClientCommand::Steer { task_id, content } => {
                match self
                    .tasks
                    .steer(actor, &task_id, &SteerTaskDto { content })
                    .await
                {
                    Ok(msg) => vec![WsServerFrame::Ack {
                        action: "steer".to_owned(),
                        id: msg.message_id,
                    }],
                    Err(err) => vec![WsServerFrame::Error {
                        code: err.rpc_code().as_str().to_owned(),
                        message: err.to_string(),
                    }],
                }
            }
            WsClientCommand::FollowUp { content } => {
                match self
                    .tasks
                    .queue_follow_up(actor, session_id, &FollowUpDto { content })
                    .await
                {
                    Ok(msg) => vec![WsServerFrame::Ack {
                        action: "followUp".to_owned(),
                        id: msg.message_id,
                    }],
                    Err(err) => vec![WsServerFrame::Error {
                        code: err.rpc_code().as_str().to_owned(),
                        message: err.to_string(),
                    }],
                }
            }
            WsClientCommand::Pause => {
                *paused = true;
                vec![WsServerFrame::Ack {
                    action: "pause".to_owned(),
                    id: session_id.to_owned(),
                }]
            }
            WsClientCommand::Resume => {
                *paused = false;
                let mut frames = vec![WsServerFrame::Ack {
                    action: "resume".to_owned(),
                    id: session_id.to_owned(),
                }];
                if let Ok(missed) = self
                    .replay_events(actor, session_id, *last_delivered_seq, 500)
                    .await
                {
                    for ev in missed {
                        *last_delivered_seq = (*last_delivered_seq).max(ev.seq);
                        frames.push(WsServerFrame::Event(ev));
                    }
                }
                frames
            }
            WsClientCommand::Ping => vec![WsServerFrame::Pong],
        }
    }
}
