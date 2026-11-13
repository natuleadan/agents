//! Ephemeral Traefik preview route publisher over private VLAN with TTL expiry,
//! optional bearer token enforcement, and RAII `DropGuard` deregistration.

use std::{
    collections::HashMap,
    net::IpAddr,
    path::{Path, PathBuf},
    sync::Arc,
};

use agents_protocol::TraefikPreviewRoute;
use tokio::sync::Mutex;

use crate::{
    artifacts::sha256_hex,
    config::{is_vlan_10_or_loopback, now_ms},
    error::BoxerError,
    leases::{CleanupDropGuard, SandboxResources},
};

/// Input descriptor requested by a sandbox agent exposing an HTTP/HTML preview.
#[derive(Debug, Clone)]
pub struct PreviewPublishRequest {
    pub task_id: String,
    pub slug: String,
    pub internal_host: String,
    pub internal_port: u16,
    pub ttl_seconds: u64,
    pub preview_token: Option<String>,
}

/// Active route entry plus metadata tracked by the preview registry.
#[derive(Debug, Clone)]
pub struct ActivePreviewRecord {
    pub route: TraefikPreviewRoute,
    pub route_file: Option<PathBuf>,
    pub token_sha256: Option<String>,
}

/// Sanitizes an arbitrary label into a DNS-safe lowercase slug (`[a-z0-9-]`).
#[must_use]
pub fn sanitize_slug(raw: &str, fallback_suffix: &str) -> String {
    let mut slug: String = raw
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    while slug.contains("--") {
        slug = slug.replace("--", "-");
    }
    let trimmed = slug.trim_matches('-');
    let base = if trimmed.is_empty() {
        format!("app-{fallback_suffix}")
    } else {
        trimmed.to_owned()
    };
    base.chars().take(48).collect()
}

/// Validates that a preview upstream IP stays inside private VLAN `10.0.0.0/8` or loopback.
///
/// # Errors
///
/// Returns [`BoxerError::Preview`] when the host is invalid or non-VLAN.
pub fn validate_preview_upstream(host: &str, port: u16) -> Result<(), BoxerError> {
    if port == 0 {
        return Err(BoxerError::Preview("internal_port must be non-zero".into()));
    }
    let ip: IpAddr = host.parse().map_err(|_| {
        BoxerError::Preview(format!(
            "internal_host `{host}` must be a private VLAN IP address"
        ))
    })?;
    if !is_vlan_10_or_loopback(ip) {
        return Err(BoxerError::Preview(format!(
            "refusing non-VLAN preview upstream `{host}` (requires 10.0.0.0/8 or loopback)"
        )));
    }
    Ok(())
}

/// Manages ephemeral Traefik dynamic routes written to disk for live previews.
#[derive(Clone)]
pub struct TraefikFileProvider {
    domain: String,
    dynamic_dir: PathBuf,
    routes: Arc<Mutex<HashMap<String, ActivePreviewRecord>>>,
}

impl TraefikFileProvider {
    /// Creates a new Traefik dynamic file provider under `dynamic_dir`.
    ///
    /// # Errors
    ///
    /// Returns [`BoxerError::Io`] if `dynamic_dir` cannot be created.
    pub fn new(domain: impl Into<String>, dynamic_dir: PathBuf) -> Result<Self, BoxerError> {
        std::fs::create_dir_all(&dynamic_dir)?;
        Ok(Self {
            domain: domain.into(),
            dynamic_dir,
            routes: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Publishes an ephemeral Traefik route file and returns both the wire [`TraefikPreviewRoute`]
    /// and a RAII [`CleanupDropGuard`] that removes the route file when dropped.
    ///
    /// # Errors
    ///
    /// Returns [`BoxerError::Preview`] on invalid VLAN host, port `0`, or TTL `0`.
    pub async fn publish(
        &self,
        req: PreviewPublishRequest,
    ) -> Result<
        (
            TraefikPreviewRoute,
            CleanupDropGuard<SandboxResources, fn(SandboxResources)>,
        ),
        BoxerError,
    > {
        validate_preview_upstream(&req.internal_host, req.internal_port)?;
        if req.ttl_seconds == 0 {
            return Err(BoxerError::Preview(
                "preview ttl_seconds must be > 0".into(),
            ));
        }

        let route_id = uuid::Uuid::new_v4().to_string();
        let short_id: String = route_id.chars().take(8).collect();
        let slug = format!("{}-{short_id}", sanitize_slug(&req.slug, &short_id));
        let host_fqdn = format!("{slug}.{}", self.domain);
        let public_url = format!("https://{host_fqdn}");
        let requires_token = req
            .preview_token
            .as_deref()
            .is_some_and(|t| !t.trim().is_empty());
        let token_sha256 = req
            .preview_token
            .as_deref()
            .filter(|t| !t.trim().is_empty())
            .map(|t| sha256_hex(t.as_bytes()));

        let route = TraefikPreviewRoute {
            route_id: route_id.clone(),
            task_id: req.task_id.clone(),
            slug: slug.clone(),
            internal_host: req.internal_host.clone(),
            internal_port: req.internal_port,
            public_url,
            ttl_seconds: req.ttl_seconds,
            requires_token,
            created_at_ms: now_ms(),
        };

        let route_file = self.route_file_path(&route_id);
        let yaml = render_traefik_yaml(&route, &host_fqdn, token_sha256.as_deref());
        write_atomic(&route_file, yaml.as_bytes())?;

        self.routes.lock().await.insert(
            route_id.clone(),
            ActivePreviewRecord {
                route: route.clone(),
                route_file: Some(route_file.clone()),
                token_sha256,
            },
        );

        let mut res = SandboxResources::new(&req.task_id);
        res.preview_route_file = Some(route_file);

        fn cleanup_preview(res: SandboxResources) {
            res.cleanup_sync();
        }

        Ok((route, CleanupDropGuard::new(res, cleanup_preview)))
    }

    /// Explicitly revokes an active preview route by ID and removes its YAML file.
    pub async fn revoke(&self, route_id: &str) -> Result<bool, BoxerError> {
        let removed = self.routes.lock().await.remove(route_id);
        let Some(rec) = removed else {
            return Ok(false);
        };
        if let Some(path) = rec.route_file
            && path.exists()
        {
            std::fs::remove_file(path)?;
        }
        Ok(true)
    }

    /// Sweeps expired routes whose `created_at_ms + ttl_seconds * 1000 <= current_ms`.
    pub async fn sweep_expired(&self, current_ms: u64) -> Result<usize, BoxerError> {
        let mut expired_ids = Vec::new();
        {
            let guard = self.routes.lock().await;
            for (id, rec) in guard.iter() {
                let expires_at = rec
                    .route
                    .created_at_ms
                    .saturating_add(rec.route.ttl_seconds.saturating_mul(1000));
                if current_ms >= expires_at {
                    expired_ids.push(id.clone());
                }
            }
        }
        let mut count = 0;
        for id in expired_ids {
            if self.revoke(&id).await? {
                count += 1;
            }
        }
        Ok(count)
    }

    #[must_use]
    pub fn route_file_path(&self, route_id: &str) -> PathBuf {
        self.dynamic_dir.join(format!("preview-{route_id}.yml"))
    }

    pub async fn active_count(&self) -> usize {
        self.routes.lock().await.len()
    }
}

fn write_atomic(target: &Path, bytes: &[u8]) -> Result<(), BoxerError> {
    let tmp = target.with_extension("yml.tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(tmp, target)?;
    Ok(())
}

fn render_traefik_yaml(
    route: &TraefikPreviewRoute,
    host_fqdn: &str,
    token_sha256: Option<&str>,
) -> String {
    let router_name = format!("preview-{}", route.route_id);
    let service_name = format!("svc-{}", route.route_id);
    let middleware_name = format!("mw-{}", route.route_id);

    let mut out = format!(
        "http:\n  routers:\n    {router_name}:\n      rule: \"Host(`{host_fqdn}`)\"\n      entryPoints:\n        - websecure\n      service: {service_name}\n"
    );
    if route.requires_token {
        out.push_str(&format!(
            "      middlewares:\n        - {middleware_name}\n"
        ));
    }
    out.push_str(&format!(
        "  services:\n    {service_name}:\n      loadBalancer:\n        servers:\n          - url: \"http://{}:{}\"\n",
        route.internal_host, route.internal_port
    ));
    if let Some(digest) = token_sha256 {
        out.push_str(&format!(
            "  middlewares:\n    {middleware_name}:\n      headers:\n        customRequestHeaders:\n          X-Preview-Token-Sha256: \"{digest}\"\n"
        ));
    }
    out
}
