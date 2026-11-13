//! Ironic HTTP replay controller plus native Axum authenticated WebSocket and SSE endpoints.

use std::{
    convert::Infallible,
    sync::{Arc, OnceLock},
};

use agents_protocol::{ApiKeyScope, EventEnvelope};
use axum::{
    Router,
    extract::{
        Path, Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::get,
};
use futures_util::{SinkExt, StreamExt};
use ironic::prelude::*;

use crate::{
    infra::error::validate_dto,
    modules::{
        auth::{
            entities::AuthenticatedActor,
            guards::{AuthGuard, CurrentActor, extract_credential, require_scope},
            services::AuthService,
        },
        gateway::{
            dto::{StreamReplayQuery, WsClientCommand, WsServerFrame},
            services::GatewayService,
        },
    },
};

/// Shared dependencies injected into the native Axum WebSocket/SSE router.
#[derive(Clone)]
pub struct GatewayHttpState {
    pub auth: Arc<AuthService>,
    pub gateway: Arc<GatewayService>,
}

#[controller("/api/v1/sessions")]
#[guard(AuthGuard)]
#[derive(Injectable)]
pub struct GatewayController {
    service: Arc<GatewayService>,
}

#[routes]
impl GatewayController {
    #[get("/:id/events")]
    #[api(
        summary = "Replay ordered session stream events",
        tag = "Gateway",
        security = "bearerAuth"
    )]
    #[resp(200, "Ordered stream envelopes", json = Vec<EventEnvelope>)]
    async fn replay(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[param] id: String,
        #[query] query: StreamReplayQuery,
    ) -> Result<Json<Vec<EventEnvelope>>, HttpError> {
        validate_dto(&query)?;
        require_scope(&actor, ApiKeyScope::Read)?;
        let events = self
            .service
            .replay_events(
                &actor,
                &id,
                query.after_seq.unwrap_or(0),
                query.limit.unwrap_or(200),
            )
            .await?;
        Ok(Json(events))
    }
}

/// Builds native Axum routes for bidirectional WebSocket and SSE streaming.
pub fn gateway_stream_router(state: GatewayHttpState) -> Router {
    Router::new()
        .route("/api/v1/sessions/{id}/ws", get(ws_upgrade_handler))
        .route(
            "/api/v1/sessions/{id}/events/stream",
            get(sse_stream_handler),
        )
        .with_state(state)
}

/// Builds native Axum routes backed by a lazily populated [`GatewayHttpState`].
pub fn gateway_lazy_router(cell: Arc<OnceLock<GatewayHttpState>>) -> Router {
    Router::new()
        .route("/api/v1/sessions/{id}/ws", get(ws_upgrade_lazy))
        .route("/api/v1/sessions/{id}/events/stream", get(sse_stream_lazy))
        .with_state(cell)
}

async fn ws_upgrade_lazy(
    State(cell): State<Arc<OnceLock<GatewayHttpState>>>,
    path: Path<String>,
    query: Query<StreamReplayQuery>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let Some(state) = cell.get().cloned() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "gateway initializing").into_response();
    };
    ws_upgrade_handler(State(state), path, query, headers, ws).await
}

async fn sse_stream_lazy(
    State(cell): State<Arc<OnceLock<GatewayHttpState>>>,
    path: Path<String>,
    query: Query<StreamReplayQuery>,
    headers: HeaderMap,
) -> Response {
    let Some(state) = cell.get().cloned() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "gateway initializing").into_response();
    };
    sse_stream_handler(State(state), path, query, headers).await
}

async fn authenticate_ws_or_sse(
    auth: &AuthService,
    headers: &HeaderMap,
    query: &StreamReplayQuery,
) -> Result<AuthenticatedActor, (StatusCode, &'static str)> {
    let raw_token = if let Ok(header_tok) = extract_credential(headers) {
        header_tok
    } else if let Some(q_tok) = query
        .token
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        q_tok.to_owned()
    } else {
        return Err((
            StatusCode::UNAUTHORIZED,
            "missing authentication credentials",
        ));
    };

    auth.authenticate_token(&raw_token)
        .await
        .map_err(|_| (StatusCode::UNAUTHORIZED, "invalid or expired credentials"))
}

async fn ws_upgrade_handler(
    State(state): State<GatewayHttpState>,
    Path(session_id): Path<String>,
    Query(query): Query<StreamReplayQuery>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let actor = match authenticate_ws_or_sse(&state.auth, &headers, &query).await {
        Ok(actor) => actor,
        Err((status, msg)) => return (status, msg).into_response(),
    };

    let rx = match state.gateway.subscribe_session(&actor, &session_id).await {
        Ok(rx) => rx,
        Err(crate::infra::error::ServerError::NotFound(_)) => {
            return (StatusCode::NOT_FOUND, "session not found").into_response();
        }
        Err(_) => return (StatusCode::FORBIDDEN, "access denied").into_response(),
    };

    let after_seq = query.after_seq.unwrap_or(0);
    ws.on_upgrade(move |socket| {
        handle_ws_socket(socket, state.gateway, actor, session_id, after_seq, rx)
    })
}

async fn handle_ws_socket(
    socket: WebSocket,
    gateway: Arc<GatewayService>,
    actor: AuthenticatedActor,
    session_id: String,
    after_seq: u64,
    mut rx: tokio::sync::broadcast::Receiver<EventEnvelope>,
) {
    let (mut sender, mut receiver) = socket.split();
    let mut paused = false;
    let mut last_seq = after_seq;

    if let Ok(backlog) = gateway
        .replay_events(&actor, &session_id, after_seq, 500)
        .await
    {
        for ev in backlog {
            last_seq = last_seq.max(ev.seq);
            if send_frame(&mut sender, &WsServerFrame::Event(ev))
                .await
                .is_err()
            {
                return;
            }
        }
    }

    loop {
        tokio::select! {
            incoming = receiver.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<WsClientCommand>(&text) {
                            Ok(cmd) => {
                                let replies = gateway
                                    .handle_ws_command(&actor, &session_id, cmd, &mut paused, &mut last_seq)
                                    .await;
                                for frame in replies {
                                    if send_frame(&mut sender, &frame).await.is_err() {
                                        return;
                                    }
                                }
                            }
                            Err(err) => {
                                let _ = send_frame(&mut sender, &WsServerFrame::Error {
                                    code: "invalid_frame".to_owned(),
                                    message: err.to_string(),
                                }).await;
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => return,
                    _ => {}
                }
            }
            live = rx.recv() => {
                match live {
                    Ok(ev) if !paused && ev.seq > last_seq => {
                        last_seq = ev.seq;
                        if send_frame(&mut sender, &WsServerFrame::Event(ev)).await.is_err() {
                            return;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                    _ => {}
                }
            }
        }
    }
}

async fn send_frame(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    frame: &WsServerFrame,
) -> Result<(), ()> {
    let json = serde_json::to_string(frame).map_err(|_| ())?;
    sender
        .send(Message::Text(json.into()))
        .await
        .map_err(|_| ())
}

async fn sse_stream_handler(
    State(state): State<GatewayHttpState>,
    Path(session_id): Path<String>,
    Query(query): Query<StreamReplayQuery>,
    headers: HeaderMap,
) -> Response {
    let actor = match authenticate_ws_or_sse(&state.auth, &headers, &query).await {
        Ok(actor) => actor,
        Err((status, msg)) => return (status, msg).into_response(),
    };

    let last_event_id_seq = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok());
    let after_seq = query.after_seq.or(last_event_id_seq).unwrap_or(0);

    let backlog = match state
        .gateway
        .replay_events(&actor, &session_id, after_seq, 500)
        .await
    {
        Ok(items) => items,
        Err(crate::infra::error::ServerError::NotFound(_)) => {
            return (StatusCode::NOT_FOUND, "session not found").into_response();
        }
        Err(_) => return (StatusCode::FORBIDDEN, "access denied").into_response(),
    };

    let rx = match state.gateway.subscribe_session(&actor, &session_id).await {
        Ok(rx) => rx,
        Err(_) => return (StatusCode::FORBIDDEN, "access denied").into_response(),
    };

    let backlog_stream = futures_util::stream::iter(backlog.into_iter().map(|ev| {
        let data = serde_json::to_string(&ev).unwrap_or_default();
        Ok::<Event, Infallible>(Event::default().id(ev.seq.to_string()).data(data))
    }));

    let live_stream =
        futures_util::stream::unfold((rx, after_seq), |(mut rx, mut min_seq)| async move {
            loop {
                match rx.recv().await {
                    Ok(ev) if ev.seq > min_seq => {
                        min_seq = ev.seq;
                        let data = serde_json::to_string(&ev).unwrap_or_default();
                        return Some((
                            Ok::<Event, Infallible>(
                                Event::default().id(ev.seq.to_string()).data(data),
                            ),
                            (rx, min_seq),
                        ));
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
                }
            }
        });

    Sse::new(backlog_stream.chain(live_stream))
        .keep_alive(KeepAlive::default())
        .into_response()
}
