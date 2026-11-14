//! Application module graph, dependency wiring, OpenAPI metadata, and Axum platform assembly.

use std::{
    sync::{Arc, OnceLock},
    time::Duration,
};

use ironic::{
    AxumAdapter, OpenApiAxumApplication, OpenApiAxumExt, ProviderDefinition,
    prelude::*,
    register_health_indicator,
    security::{
        CorsConfig, CorsMiddleware, RateLimitMiddleware, SecurityHeadersConfig,
        SecurityHeadersMiddleware,
    },
};
use tower::ServiceExt;

use crate::{
    infra::{
        adapters::{
            BlobStoreClient, BlobStorePort, InfrastructureRegistry, VectorStoreClient,
            VectorStorePort,
        },
        admission::AdmissionController,
        bus::MessageBus,
        cache::AppCache,
        config::ServerConfig,
        db::{BusHealthIndicator, Database, PostgresHealthIndicator, StateHealthIndicator},
        error::ServerError,
        events::AppEventBus,
        state::StateBackend,
        stream::{
            AdmissionStore, BusClient, InMemoryStreamRepository, PgStreamRepository, StreamStore,
        },
    },
    modules::{
        audit::{
            AuditModule,
            repositories::{AuditStore, InMemoryAuditRepository, PgAuditRepository},
        },
        auth::{
            AuthModule,
            repositories::{AuthStore, InMemoryAuthRepository, PgAuthRepository},
            services::{AuthService, StateStore},
        },
        autocron::{
            AutocronModule,
            repositories::{AutocronStore, InMemoryAutocronRepository, PgAutocronRepository},
        },
        gateway::{GatewayModule, controller::GatewayHttpState, services::GatewayService},
        mcp::{McpModule, controller::McpHttpState, services::McpHostService},
        nodes::{NodeHttpState, NodeRegistryService, NodeRegistryStore, NodesModule},
        sessions::{
            SessionsModule,
            repositories::{InMemorySessionRepository, PgSessionRepository, SessionStore},
        },
        swarm::{
            SwarmModule,
            repositories::{InMemorySwarmRepository, PgSwarmRepository, SwarmStore},
        },
        tasks::{
            TasksModule,
            repositories::{InMemoryTaskRepository, PgTaskRepository, TaskStore},
        },
        workflows::{
            WorkflowsModule,
            repositories::{InMemoryWorkflowRepository, PgWorkflowRepository, WorkflowStore},
        },
    },
};

/// Global infrastructure module registering default repository/bus/state wrappers.
pub struct InfraModule;

impl Module for InfraModule {
    fn definition() -> ModuleDefinition {
        ModuleDefinition::builder::<Self>()
            .provider(ProviderDefinition::value(ServerConfig::for_tests()))
            .provider(ProviderDefinition::value(AppCache::default()))
            .provider(ProviderDefinition::value(AppEventBus::default()))
            .provider(ProviderDefinition::value(StateStore::default()))
            .provider(ProviderDefinition::value(BusClient::default()))
            .provider(ProviderDefinition::value(VectorStoreClient::default()))
            .provider(ProviderDefinition::value(BlobStoreClient::default()))
            .provider(ProviderDefinition::value(AdmissionStore::default()))
            .provider(ProviderDefinition::value(AuditStore::default()))
            .provider(ProviderDefinition::value(AuthStore::default()))
            .provider(ProviderDefinition::value(NodeRegistryStore::default()))
            .provider(ProviderDefinition::value(SessionStore::default()))
            .provider(ProviderDefinition::value(TaskStore::default()))
            .provider(ProviderDefinition::value(StreamStore::default()))
            .provider(ProviderDefinition::value(SwarmStore::default()))
            .provider(ProviderDefinition::value(WorkflowStore::default()))
            .provider(ProviderDefinition::value(AutocronStore::default()))
            .export::<ServerConfig>()
            .export::<AppCache>()
            .export::<AppEventBus>()
            .export::<StateStore>()
            .export::<BusClient>()
            .export::<VectorStoreClient>()
            .export::<BlobStoreClient>()
            .export::<AdmissionStore>()
            .export::<AuditStore>()
            .export::<AuthStore>()
            .export::<NodeRegistryStore>()
            .export::<SessionStore>()
            .export::<TaskStore>()
            .export::<StreamStore>()
            .export::<SwarmStore>()
            .export::<WorkflowStore>()
            .export::<AutocronStore>()
            .build()
    }
}

/// Root Ironic application module wiring all domain modules.
#[derive(Module)]
#[module(imports = [
    HealthModule,
    InfraModule,
    AuditModule,
    AuthModule,
    NodesModule,
    SessionsModule,
    TasksModule,
    SwarmModule,
    WorkflowsModule,
    AutocronModule,
    GatewayModule,
    McpModule,
])]
pub struct AppModule;

/// Runtime infrastructure handles constructed from [`ServerConfig`].
pub struct InfrastructureHandles {
    pub config: ServerConfig,
    pub database: Option<Database>,
    pub state: Arc<dyn StateBackend>,
    pub bus: Arc<dyn MessageBus>,
    pub vector: Arc<dyn VectorStorePort>,
    pub blob: Arc<dyn BlobStorePort>,
    pub admission: Arc<AdmissionController>,
    pub audit_store: AuditStore,
    pub auth_store: AuthStore,
    pub session_store: SessionStore,
    pub task_store: TaskStore,
    pub stream_store: StreamStore,
    pub swarm_store: SwarmStore,
    pub workflow_store: WorkflowStore,
    pub autocron_store: AutocronStore,
}

impl InfrastructureHandles {
    /// Connects configured external infrastructure (PostgreSQL, Valkey, NATS) or falls back
    /// to hermetic in-memory implementations when URLs are unset in dev/test modes.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError`] when a configured datastore fails to connect or migrate.
    pub async fn from_config(config: ServerConfig) -> Result<Self, ServerError> {
        let database = match config.database_url.as_deref() {
            Some(url) => {
                Some(Database::connect_and_migrate(url, config.database_max_connections).await?)
            }
            None => None,
        };

        let registry = InfrastructureRegistry::from_config(&config).await?;
        let state = Arc::clone(&registry.state);
        let bus = Arc::clone(&registry.bus);
        let vector = Arc::clone(&registry.vector);
        let blob = Arc::clone(&registry.blob);

        let admission =
            AdmissionController::new(config.max_concurrent_global, config.max_concurrent_session);

        let (
            audit_store,
            auth_store,
            session_store,
            task_store,
            stream_store,
            swarm_store,
            workflow_store,
            autocron_store,
        ) = match database.as_ref() {
            Some(db) => {
                let pool = db.pool().clone();
                (
                    AuditStore(PgAuditRepository::new(pool.clone())),
                    AuthStore(PgAuthRepository::new(pool.clone())),
                    SessionStore(PgSessionRepository::new(pool.clone())),
                    TaskStore(PgTaskRepository::new(pool.clone())),
                    StreamStore(PgStreamRepository::new(pool.clone())),
                    SwarmStore(PgSwarmRepository::new(pool.clone())),
                    WorkflowStore(PgWorkflowRepository::new(pool.clone())),
                    AutocronStore(PgAutocronRepository::new(pool)),
                )
            }
            None => (
                AuditStore(InMemoryAuditRepository::new()),
                AuthStore(InMemoryAuthRepository::new()),
                SessionStore(InMemorySessionRepository::new()),
                TaskStore(InMemoryTaskRepository::new()),
                StreamStore(InMemoryStreamRepository::new()),
                SwarmStore(InMemorySwarmRepository::new()),
                WorkflowStore(InMemoryWorkflowRepository::new()),
                AutocronStore(InMemoryAutocronRepository::new()),
            ),
        };

        Ok(Self {
            config,
            database,
            state,
            bus,
            vector,
            blob,
            admission,
            audit_store,
            auth_store,
            session_store,
            task_store,
            stream_store,
            swarm_store,
            workflow_store,
            autocron_store,
        })
    }

    /// Creates isolated in-memory infrastructure handles for integration tests.
    #[must_use]
    pub fn in_memory(config: ServerConfig) -> Self {
        let registry = InfrastructureRegistry::in_memory();
        let state = Arc::clone(&registry.state);
        let bus = Arc::clone(&registry.bus);
        let vector = Arc::clone(&registry.vector);
        let blob = Arc::clone(&registry.blob);
        let admission =
            AdmissionController::new(config.max_concurrent_global, config.max_concurrent_session);
        Self {
            config,
            database: None,
            state,
            bus,
            vector,
            blob,
            admission,
            audit_store: AuditStore(InMemoryAuditRepository::new()),
            auth_store: AuthStore(InMemoryAuthRepository::new()),
            session_store: SessionStore(InMemorySessionRepository::new()),
            task_store: TaskStore(InMemoryTaskRepository::new()),
            stream_store: StreamStore(InMemoryStreamRepository::new()),
            swarm_store: SwarmStore(InMemorySwarmRepository::new()),
            workflow_store: WorkflowStore(InMemoryWorkflowRepository::new()),
            autocron_store: AutocronStore(InMemoryAutocronRepository::new()),
        }
    }
}

/// Builds the full Ironic + Axum + OpenAPI + WebSocket + SSE + MCP application.
///
/// # Errors
///
/// Returns [`ServerError`] when module compilation, seeding, or router construction fails.
pub async fn build_application(
    infra: InfrastructureHandles,
) -> Result<Application<OpenApiAxumApplication>, ServerError> {
    register_health_indicator(Arc::new(PostgresHealthIndicator::new(
        infra.database.clone(),
    )));
    register_health_indicator(Arc::new(StateHealthIndicator::new(Arc::clone(
        &infra.state,
    ))));
    register_health_indicator(Arc::new(BusHealthIndicator::new(Arc::clone(&infra.bus))));

    let lazy_gateway = Arc::new(OnceLock::<GatewayHttpState>::new());
    let lazy_mcp = Arc::new(OnceLock::<McpHttpState>::new());
    let lazy_nodes = Arc::new(OnceLock::<NodeHttpState>::new());

    let gw_cell = Arc::clone(&lazy_gateway);
    let mcp_cell = Arc::clone(&lazy_mcp);
    let nodes_cell = Arc::clone(&lazy_nodes);

    let openapi_config = crate::openapi::configure_openapi();

    let cors = CorsConfig::new()
        .allowed_origins(infra.config.trusted_origins.clone())
        .allowed_headers(vec![
            "Authorization",
            "Content-Type",
            "Accept",
            "Origin",
            "X-Requested-With",
            "X-API-Key",
        ])
        .allow_credentials(true)
        .max_age(86400);
    let global_ip_rate_max = infra.config.rate_limit_max_requests.saturating_mul(10);
    let rate_win = infra.config.rate_limit_window_secs;

    let enriched_openapi_cell = Arc::new(OnceLock::<String>::new());
    let doc_cell = Arc::clone(&enriched_openapi_cell);

    let rate_limiter =
        RateLimitMiddleware::new(global_ip_rate_max, rate_win).with_key_resolver(Arc::new(|ctx| {
            let req = ctx.request();
            if let Some(agent_id) = req
                .headers()
                .get("x-agent-id")
                .or_else(|| req.headers().get("x-box-id"))
                .and_then(|h| h.to_str().ok())
                .filter(|s| !s.is_empty())
            {
                return format!("agent:{agent_id}");
            }
            if let Some(api_key) = req
                .headers()
                .get("x-api-key")
                .and_then(|h| h.to_str().ok())
                .filter(|s| !s.is_empty())
            {
                return format!("key:{api_key}");
            }
            req.headers()
                .get("x-forwarded-for")
                .and_then(|v| v.to_str().ok())
                .and_then(|all| all.split(',').next_back().map(str::trim))
                .filter(|ip| !ip.is_empty())
                .unwrap_or("127.0.0.1")
                .to_owned()
        }));

    let app = Application::builder()
        .module(AppModule::definition())
        .override_provider(ProviderDefinition::value(infra.config))
        .override_provider(ProviderDefinition::value(StateStore(infra.state)))
        .override_provider(ProviderDefinition::value(BusClient(infra.bus)))
        .override_provider(ProviderDefinition::value(VectorStoreClient(infra.vector)))
        .override_provider(ProviderDefinition::value(BlobStoreClient(infra.blob)))
        .override_provider(ProviderDefinition::value(AdmissionStore(infra.admission)))
        .override_provider(ProviderDefinition::value(infra.audit_store))
        .override_provider(ProviderDefinition::value(infra.auth_store))
        .override_provider(ProviderDefinition::value(infra.session_store))
        .override_provider(ProviderDefinition::value(infra.task_store))
        .override_provider(ProviderDefinition::value(infra.stream_store))
        .override_provider(ProviderDefinition::value(infra.swarm_store))
        .override_provider(ProviderDefinition::value(infra.workflow_store))
        .override_provider(ProviderDefinition::value(infra.autocron_store))
        .middleware(SecurityHeadersMiddleware::new(
            SecurityHeadersConfig::default(),
        ))
        .middleware(rate_limiter)
        .middleware(CorsMiddleware::new(cors))
        .platform(
            AxumAdapter::new()
                .compression()
                .max_connections(10_000)
                .request_body_limit(4 * 1024 * 1024)
                .request_timeout(Duration::from_secs(30))
                .configure_router(move |router| {
                    mount_native_stream_and_mcp_routes(
                        router, gw_cell, mcp_cell, nodes_cell, doc_cell,
                    )
                })
                .with_openapi(openapi_config),
        )
        .build()
        .await
        .map_err(|err| ServerError::Internal(format!("application build failed: {err}")))?;

    {
        use http_body_util::BodyExt;

        let req = axum::http::Request::builder()
            .uri("/openapi.raw.json")
            .body(axum::body::Body::empty())
            .map_err(|e| ServerError::Internal(e.to_string()))?;
        let resp = match app.platform().router().clone().oneshot(req).await {
            Ok(r) => r,
            Err(_) => return Err(ServerError::Internal("openapi route unreachable".into())),
        };
        let Ok(collected) = resp.into_body().collect().await else {
            return Err(ServerError::Internal("failed to read openapi body".into()));
        };
        let bytes = collected.to_bytes();
        let mut spec: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|e| ServerError::Internal(e.to_string()))?;
        crate::openapi::enrich_openapi_spec(&mut spec);
        let enriched_json =
            serde_json::to_string(&spec).map_err(|e| ServerError::Internal(e.to_string()))?;
        let _ = enriched_openapi_cell.set(enriched_json);
    }

    let auth = app
        .container()
        .resolve::<AuthService>()
        .await
        .map_err(|e| ServerError::Internal(e.to_string()))?;
    let gateway = app
        .container()
        .resolve::<GatewayService>()
        .await
        .map_err(|e| ServerError::Internal(e.to_string()))?;
    let mcp = app
        .container()
        .resolve::<McpHostService>()
        .await
        .map_err(|e| ServerError::Internal(e.to_string()))?;
    let nodes = app
        .container()
        .resolve::<NodeRegistryService>()
        .await
        .map_err(|e| ServerError::Internal(e.to_string()))?;

    auth.seed_admin_if_configured().await?;

    let _ = lazy_gateway.set(GatewayHttpState {
        auth: Arc::clone(&auth),
        gateway,
    });
    let _ = lazy_mcp.set(McpHttpState { auth, mcp });
    let _ = lazy_nodes.set(NodeHttpState { service: nodes });

    Ok(app)
}

pub use crate::infra::workers::start_retention_worker;

pub use crate::infra::webui::mount_native_stream_and_mcp_routes;
