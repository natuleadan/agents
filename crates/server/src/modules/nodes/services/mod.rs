//! Service managing private-VLAN Boxer auto-registration and dynamic Traefik HTTP routes.

use std::{collections::HashMap, net::IpAddr, sync::Arc};

use agents_protocol::{BoxerNodeAnnouncement, PreviewRouteSync};
use ironic::prelude::*;
use serde_json::{Map, Value, json};
use tokio::sync::Mutex;

use crate::{
    infra::{error::ServerError, state::now_ms},
    modules::{
        audit::services::AuditService,
        auth::entities::AuthenticatedActor,
        nodes::{
            dto::{NodeHeartbeatDto, PreviewSyncDto},
            entities::{NodeListResponse, PreviewSyncAck, StoredPreviewEntry},
        },
    },
};

#[derive(Default)]
struct NodeRegistryInner {
    nodes: HashMap<String, BoxerNodeAnnouncement>,
    previews: HashMap<String, StoredPreviewEntry>,
}

/// DI state holder for active VLAN Boxer nodes and distributed Traefik preview routes.
#[derive(Clone, Default)]
pub struct NodeRegistryStore {
    inner: Arc<Mutex<NodeRegistryInner>>,
}

/// Coordinates infinite Boxer node discovery over private VLAN and cross-VPS Traefik routing.
#[derive(Injectable)]
pub struct NodeRegistryService {
    store: Arc<NodeRegistryStore>,
    audit: Arc<AuditService>,
}

impl NodeRegistryService {
    /// Registers or refreshes a private-VLAN Boxer worker node.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Validation`] if `vlan_ip` is outside `10.0.0.0/8` or loopback.
    pub async fn register_heartbeat(
        &self,
        actor: &AuthenticatedActor,
        dto: &NodeHeartbeatDto,
    ) -> Result<BoxerNodeAnnouncement, ServerError> {
        validate_vlan_ip(&dto.vlan_ip)?;
        let node = BoxerNodeAnnouncement {
            node_id: dto.node_id.trim().to_owned(),
            vlan_ip: dto.vlan_ip.trim().to_owned(),
            runtime: dto.runtime.clone(),
            isolation: dto.isolation.clone(),
            max_concurrent_boxes: dto.max_concurrent_boxes,
            active_boxes: dto.active_boxes,
            status: dto.status,
            last_heartbeat_ms: now_ms(),
            ttl_seconds: dto.ttl_seconds.unwrap_or(60),
        };
        self.store
            .inner
            .lock()
            .await
            .nodes
            .insert(node.node_id.clone(), node.clone());

        let _ = self
            .audit
            .record(
                Some(&actor.user_id),
                "node.heartbeat",
                "node",
                Some(&node.node_id),
                "success",
                json!({ "vlanIp": node.vlan_ip, "maxBoxes": node.max_concurrent_boxes }),
            )
            .await;

        Ok(node)
    }

    /// Registers a Boxer node via HMAC-SHA256 handshake and issues a dynamic session token.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Validation`] on replay attack or invalid signature.
    pub async fn register_node_hmac(
        &self,
        dto: &crate::modules::nodes::dto::NodeRegistrationDto,
        cluster_secret: &str,
    ) -> Result<crate::modules::nodes::dto::NodeRegistrationResponse, ServerError> {
        let current_time = now_ms();
        if current_time.abs_diff(dto.timestamp_ms) > 60_000 {
            return Err(ServerError::Validation(
                "registration timestamp expired or clock skew exceeds 60s".into(),
            ));
        }

        let payload = format!(
            "{}:{}:{}:{}",
            dto.node_id, dto.node_ip, dto.port, dto.timestamp_ms
        );
        let expected_sig = crate::modules::auth::services::compute_hmac_hex(
            cluster_secret.as_bytes(),
            payload.as_bytes(),
        );

        if !bool::from(subtle::ConstantTimeEq::ct_eq(
            dto.signature.to_lowercase().as_bytes(),
            expected_sig.as_bytes(),
        )) {
            return Err(ServerError::Validation(
                "invalid node registration signature".into(),
            ));
        }

        let session_token = format!("boxer-tok-{}-{}", dto.node_id, uuid::Uuid::new_v4());
        let expires_at = current_time + 3_600_000;

        let node = BoxerNodeAnnouncement {
            node_id: dto.node_id.clone(),
            vlan_ip: dto.node_ip.clone(),
            runtime: dto.runtime.clone(),
            isolation: dto.isolation.clone(),
            max_concurrent_boxes: dto.max_concurrent_boxes,
            active_boxes: dto.active_boxes,
            status: agents_protocol::NodeStatus::Online,
            last_heartbeat_ms: current_time,
            ttl_seconds: 60,
        };

        self.store
            .inner
            .lock()
            .await
            .nodes
            .insert(node.node_id.clone(), node.clone());

        Ok(crate::modules::nodes::dto::NodeRegistrationResponse {
            node_id: dto.node_id.clone(),
            session_token,
            expires_at,
            status: "registered".into(),
        })
    }

    /// Lists non-expired Boxer nodes currently advertising capacity over the VLAN.
    pub async fn list_active_nodes(&self) -> NodeListResponse {
        let now = now_ms();
        let mut guard = self.store.inner.lock().await;
        guard.nodes.retain(|_, node| {
            let expires_at = node
                .last_heartbeat_ms
                .saturating_add(node.ttl_seconds.saturating_mul(1000));
            now < expires_at
        });
        let mut nodes: Vec<BoxerNodeAnnouncement> = guard.nodes.values().cloned().collect();
        nodes.sort_by(|a, b| a.node_id.cmp(&b.node_id));
        let total = nodes.len();
        NodeListResponse { nodes, total }
    }

    /// Upserts or revokes a distributed preview route targeting a VLAN Boxer container.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Validation`] when `internal_host` is not a private `10.0.0.0/8`
    /// or loopback address, or when port/TTL invariants fail.
    pub async fn sync_preview(
        &self,
        actor: &AuthenticatedActor,
        dto: &PreviewSyncDto,
    ) -> Result<PreviewSyncAck, ServerError> {
        validate_vlan_ip(&dto.route.internal_host)?;
        if dto.route.internal_port == 0 {
            return Err(ServerError::Validation(
                "preview internalPort must be non-zero".into(),
            ));
        }

        let mut guard = self.store.inner.lock().await;
        let route_id = dto.route.route_id.clone();
        if dto.revoked {
            guard.previews.remove(&route_id);
        } else {
            guard.previews.insert(
                route_id.clone(),
                StoredPreviewEntry {
                    sync: PreviewRouteSync {
                        route: dto.route.clone(),
                        token_sha256: dto.token_sha256.clone(),
                        revoked: false,
                    },
                },
            );
        }
        let active_routes = guard.previews.len();
        drop(guard);

        let _ = self
            .audit
            .record(
                Some(&actor.user_id),
                if dto.revoked {
                    "preview.revoke"
                } else {
                    "preview.publish"
                },
                "preview",
                Some(&route_id),
                "success",
                json!({
                    "internalHost": dto.route.internal_host,
                    "internalPort": dto.route.internal_port
                }),
            )
            .await;

        Ok(PreviewSyncAck {
            route_id,
            active: !dto.revoked,
            active_routes,
        })
    }

    /// Emits Traefik HTTP dynamic provider JSON bridging public requests to VLAN Boxer upstreams.
    pub async fn render_traefik_http_config(&self) -> Value {
        let now = now_ms();
        let mut guard = self.store.inner.lock().await;
        guard.previews.retain(|_, entry| {
            let expires_at = entry
                .sync
                .route
                .created_at_ms
                .saturating_add(entry.sync.route.ttl_seconds.saturating_mul(1000));
            !entry.sync.revoked && now < expires_at
        });

        let mut routers = Map::new();
        let mut services = Map::new();
        let mut middlewares = Map::new();

        for (id, entry) in &guard.previews {
            let route = &entry.sync.route;
            let service_name = format!("boxer-svc-{id}");
            let router_name = format!("boxer-preview-{id}");
            let safe_slug: String = route
                .slug
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
                .collect();

            let mut router_obj = Map::new();
            router_obj.insert(
                "rule".to_owned(),
                json!(format!("PathPrefix(`/preview/{safe_slug}`)")),
            );
            router_obj.insert("entryPoints".to_owned(), json!(["web", "websecure"]));
            router_obj.insert("service".to_owned(), json!(service_name));

            let mut mw_list = vec![format!("strip-prefix-{id}")];
            middlewares.insert(
                format!("strip-prefix-{id}"),
                json!({
                    "stripPrefix": {
                        "prefixes": [format!("/preview/{safe_slug}")]
                    }
                }),
            );

            if let Some(digest) = &entry.sync.token_sha256 {
                let token_mw = format!("token-hdr-{id}");
                middlewares.insert(
                    token_mw.clone(),
                    json!({
                        "headers": {
                            "customRequestHeaders": {
                                "X-Preview-Token-Sha256": digest
                            }
                        }
                    }),
                );
                mw_list.push(token_mw);
            }
            router_obj.insert(
                "middlewares".to_owned(),
                Value::Array(mw_list.into_iter().map(Value::String).collect()),
            );

            routers.insert(router_name, Value::Object(router_obj));
            services.insert(
                service_name,
                json!({
                    "loadBalancer": {
                        "servers": [{
                            "url": format!("http://{}:{}", route.internal_host, route.internal_port)
                        }]
                    }
                }),
            );
        }

        json!({
            "http": {
                "routers": routers,
                "services": services,
                "middlewares": middlewares
            }
        })
    }
}

/// Ensures worker announcements and preview upstreams remain confined to `10.0.0.0/8` or loopback.
fn validate_vlan_ip(raw: &str) -> Result<IpAddr, ServerError> {
    let ip: IpAddr = raw.trim().parse().map_err(|_| {
        ServerError::Validation(format!(
            "address `{raw}` must be a valid private VLAN IP address"
        ))
    })?;
    let allowed = match ip {
        IpAddr::V4(v4) => v4.is_loopback() || v4.octets()[0] == 10,
        IpAddr::V6(v6) => v6.is_loopback(),
    };
    if !allowed {
        return Err(ServerError::Validation(format!(
            "refusing non-VLAN address `{raw}` (requires 10.0.0.0/8 or loopback)"
        )));
    }
    Ok(ip)
}
