//! Outbound credential proxy injecting upstream provider tokens strictly
//! outside sandbox environments while enforcing egress host allowlists.

use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use axum::{
    Json, Router,
    body::Body,
    extract::State,
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use bytes::Bytes;
use http::{Request, header};
use http_body_util::{BodyExt, Full};
use hyper_util::{
    client::legacy::{Client, connect::HttpConnector},
    rt::TokioExecutor,
};
use serde::{Deserialize, Serialize};
use tokio::{net::TcpListener, task::JoinHandle};

use crate::{config::is_private_or_loopback_ip, error::BoxerError};

/// Upstream credential binding mapped per allowed target host inside Boxer supervisor memory.
#[derive(Debug, Clone)]
pub struct CredentialRule {
    pub host: String,
    pub header_name: String,
    pub header_value: String,
}

#[derive(Clone)]
struct ProxyState {
    network_egress_enabled: bool,
    allowed_hosts: Arc<Vec<String>>,
    credentials: Arc<HashMap<String, (String, String)>>,
    http: Client<HttpConnector, Full<Bytes>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyForwardRequest {
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub body: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProxyErrorBody {
    error: String,
}

/// Running outbound credential proxy bound strictly on private VLAN or loopback.
pub struct OutboundCredentialProxy {
    addr: SocketAddr,
    task: JoinHandle<()>,
}

impl OutboundCredentialProxy {
    /// Starts the credential-injecting outbound proxy on `bind_addr`.
    ///
    /// # Errors
    ///
    /// Refuses public non-private/loopback bind addresses and returns socket bind errors.
    pub async fn start(
        bind_addr: SocketAddr,
        network_egress_enabled: bool,
        allowed_hosts: Vec<String>,
        rules: Vec<CredentialRule>,
    ) -> Result<Self, BoxerError> {
        if !is_private_or_loopback_ip(bind_addr.ip()) {
            return Err(BoxerError::Proxy(format!(
                "refusing public proxy bind address `{bind_addr}`"
            )));
        }

        let mut credentials = HashMap::new();
        for rule in rules {
            credentials.insert(
                rule.host.to_ascii_lowercase(),
                (rule.header_name, rule.header_value),
            );
        }

        let mut connector = HttpConnector::new();
        connector.enforce_http(false);
        let http = Client::builder(TokioExecutor::new()).build(connector);

        let state = ProxyState {
            network_egress_enabled,
            allowed_hosts: Arc::new(
                allowed_hosts
                    .into_iter()
                    .map(|h| h.to_ascii_lowercase())
                    .collect(),
            ),
            credentials: Arc::new(credentials),
            http,
        };

        let listener = TcpListener::bind(bind_addr).await?;
        let local_addr = listener.local_addr()?;

        let app = Router::new()
            .route("/forward", post(handle_forward))
            .with_state(state);

        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        Ok(Self {
            addr: local_addr,
            task,
        })
    }

    #[must_use]
    pub fn endpoint_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    #[must_use]
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }
}

impl Drop for OutboundCredentialProxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn handle_forward(
    State(state): State<ProxyState>,
    _headers: HeaderMap,
    Json(req): Json<ProxyForwardRequest>,
) -> Response {
    if !state.network_egress_enabled {
        return json_error(
            StatusCode::FORBIDDEN,
            "network egress is disabled for this sandbox",
        );
    }

    let parsed: http::Uri = match req.url.parse() {
        Ok(uri) => uri,
        Err(err) => {
            return json_error(
                StatusCode::BAD_REQUEST,
                &format!("invalid target url: {err}"),
            );
        }
    };

    let Some(host) = parsed.host().map(str::to_ascii_lowercase) else {
        return json_error(StatusCode::BAD_REQUEST, "target url missing host");
    };

    if is_blocked_metadata_host(&host) {
        return json_error(
            StatusCode::FORBIDDEN,
            "metadata endpoints are blocked by SSRF policy",
        );
    }

    if !is_host_allowed(&host, &state.allowed_hosts) {
        return json_error(
            StatusCode::FORBIDDEN,
            &format!("egress to host `{host}` is not in allowlist"),
        );
    }

    let method = match http::Method::from_bytes(req.method.as_bytes()) {
        Ok(m) => m,
        Err(_) => return json_error(StatusCode::BAD_REQUEST, "invalid HTTP method"),
    };

    let mut builder = Request::builder().method(method).uri(parsed);
    for (k, v) in &req.headers {
        let lower = k.to_ascii_lowercase();
        if lower == "authorization" || lower == "host" || lower.starts_with("x-boxer-") {
            continue;
        }
        if let (Ok(name), Ok(val)) = (
            HeaderName::from_bytes(k.as_bytes()),
            HeaderValue::from_str(v),
        ) {
            builder = builder.header(name, val);
        }
    }

    if let Some((hdr_name, hdr_val)) = state.credentials.get(&host)
        && let (Ok(name), Ok(val)) = (
            HeaderName::from_bytes(hdr_name.as_bytes()),
            HeaderValue::from_str(hdr_val),
        )
    {
        builder = builder.header(name, val);
    }

    let body_bytes = req.body.map(Bytes::from).unwrap_or_default();
    let outbound = match builder.body(Full::new(body_bytes)) {
        Ok(r) => r,
        Err(err) => {
            return json_error(
                StatusCode::BAD_REQUEST,
                &format!("failed constructing outbound request: {err}"),
            );
        }
    };

    let upstream = match state.http.request(outbound).await {
        Ok(resp) => resp,
        Err(err) => {
            return json_error(
                StatusCode::BAD_GATEWAY,
                &format!("upstream request error: {err}"),
            );
        }
    };

    let status = upstream.status();
    let upstream_headers = upstream.headers().clone();
    let bytes = match upstream.into_body().collect().await {
        Ok(c) => c.to_bytes(),
        Err(err) => {
            return json_error(
                StatusCode::BAD_GATEWAY,
                &format!("upstream body read error: {err}"),
            );
        }
    };

    let mut response = Response::builder().status(status);
    if let Some(ct) = upstream_headers.get(header::CONTENT_TYPE) {
        response = response.header(header::CONTENT_TYPE, ct);
    }
    response
        .body(Body::from(bytes))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn is_blocked_metadata_host(host: &str) -> bool {
    matches!(
        host,
        "169.254.169.254" | "169.254.170.2" | "metadata.google.internal" | "metadata"
    )
}

fn is_host_allowed(host: &str, allowlist: &[String]) -> bool {
    allowlist.iter().any(|allowed| {
        if let Some(suffix) = allowed.strip_prefix("*.") {
            host == suffix || host.ends_with(&format!(".{suffix}"))
        } else {
            host == allowed
        }
    })
}

fn json_error(status: StatusCode, msg: &str) -> Response {
    (
        status,
        Json(ProxyErrorBody {
            error: msg.to_owned(),
        }),
    )
        .into_response()
}
