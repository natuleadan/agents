//! Thin Ironic HTTP controller exposing admin audit log inspection and chain verification.

use std::sync::Arc;

use agents_protocol::ApiKeyScope;
use ironic::{Pagination, Response, prelude::*};

use crate::{
    infra::{
        error::{ErrorResponse, validate_dto},
        observability::{PerRouteTimeoutInterceptor, slice_paginated},
    },
    modules::{
        audit::{
            dto::{AuditQueryDto, AuditVerificationReport},
            entities::AuditLogRecord,
            services::AuditService,
        },
        auth::{
            entities::AuthenticatedActor,
            guards::{AuthGuard, CurrentActor, ScopeGuard, require_scope},
        },
    },
    openapi::PaginatedPageResponse,
};

#[controller("/api/v1/audit")]
#[guard(AuthGuard)]
#[derive(Injectable)]
pub struct AuditController {
    service: Arc<AuditService>,
}

#[routes]
impl AuditController {
    #[get]
    #[guard(ScopeGuard::new(ApiKeyScope::Admin))]
    #[interceptor(PerRouteTimeoutInterceptor::secs(5))]
    #[api(
        summary = "List audit log entries",
        tag = "Audit",
        security = "bearerAuth"
    )]
    #[resp(200, "Audit log entries", json = Vec<AuditLogRecord>)]
    #[resp(401, "Unauthorized", json = ErrorResponse)]
    #[resp(403, "Admin scope required", json = ErrorResponse)]
    #[resp(408, "Request timeout", json = ErrorResponse)]
    #[resp(422, "Validation error", json = ErrorResponse)]
    #[resp(500, "Internal server error", json = ErrorResponse)]
    async fn list(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[query] query: AuditQueryDto,
    ) -> Result<Json<Vec<AuditLogRecord>>, HttpError> {
        validate_dto(&query)?;
        require_scope(&actor, ApiKeyScope::Admin)?;
        let entries = self.service.list(&query).await?;
        Ok(Json(entries))
    }

    #[get("/page")]
    #[guard(ScopeGuard::new(ApiKeyScope::Admin))]
    #[interceptor(PerRouteTimeoutInterceptor::secs(5))]
    #[api(
        summary = "List audit log entries with native offset pagination",
        tag = "Audit",
        security = "bearerAuth"
    )]
    #[resp(200, "Paginated audit logs", json = PaginatedPageResponse)]
    #[resp(401, "Unauthorized", json = ErrorResponse)]
    #[resp(403, "Admin scope required", json = ErrorResponse)]
    #[resp(408, "Request timeout", json = ErrorResponse)]
    #[resp(500, "Internal server error", json = ErrorResponse)]
    async fn list_page(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
        #[decorator(Pagination)] page: Pagination,
    ) -> Result<Response, HttpError> {
        require_scope(&actor, ApiKeyScope::Admin)?;
        let query = AuditQueryDto::default();
        let entries = self.service.list(&query).await?;
        let (items, total) = slice_paginated(&entries, page.offset(), page.limit());
        Response::paginated(&items, total, page.offset(), page.limit())
    }

    #[get("/verify")]
    #[guard(ScopeGuard::new(ApiKeyScope::Admin))]
    #[interceptor(PerRouteTimeoutInterceptor::secs(5))]
    #[api(
        summary = "Verify audit hash chain integrity",
        tag = "Audit",
        security = "bearerAuth"
    )]
    #[resp(200, "Audit verification report", json = AuditVerificationReport)]
    #[resp(401, "Unauthorized", json = ErrorResponse)]
    #[resp(403, "Admin scope required", json = ErrorResponse)]
    #[resp(500, "Internal server error", json = ErrorResponse)]
    async fn verify(
        &self,
        #[decorator(CurrentActor)] actor: AuthenticatedActor,
    ) -> Result<Json<AuditVerificationReport>, HttpError> {
        require_scope(&actor, ApiKeyScope::Admin)?;
        let report = self.service.verify_chain().await?;
        Ok(Json(report))
    }
}
