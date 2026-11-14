//! Thin Ironic HTTP controller for singleton installation identity and indefinite interaction ledger.

use std::sync::Arc;

use agents_protocol::ApiKeyScope;
use ironic::prelude::*;

use crate::{
    infra::{
        error::{ErrorResponse, validate_dto},
        observability::PerRouteTimeoutInterceptor,
    },
    modules::auth::guards::{AuthGuard, ScopeGuard},
};

use super::{
    entities::{
        InstallationIdentityRecord, LedgerListResponse, LedgerQueryDto,
        UpdateInstallationIdentityDto,
    },
    services::IdentityService,
};

#[controller("/api/v1/identity")]
#[guard(AuthGuard)]
#[derive(Injectable)]
pub struct IdentityController {
    service: Arc<IdentityService>,
}

#[routes]
impl IdentityController {
    #[get("/installation")]
    #[guard(ScopeGuard::new(ApiKeyScope::Read))]
    #[interceptor(PerRouteTimeoutInterceptor::secs(5))]
    #[api(
        summary = "Get active singleton installation identity",
        tag = "Identity",
        security = "bearerAuth"
    )]
    #[resp(200, "Active installation identity", json = InstallationIdentityRecord)]
    #[resp(401, "Missing or invalid credentials", json = ErrorResponse)]
    async fn get_installation(
        &self,
    ) -> Result<Json<Option<InstallationIdentityRecord>>, HttpError> {
        let identity = self.service.resolve_installation_identity().await?;
        Ok(Json(identity))
    }

    #[put("/installation")]
    #[guard(ScopeGuard::new(ApiKeyScope::Admin))]
    #[interceptor(PerRouteTimeoutInterceptor::secs(5))]
    #[api(
        summary = "Upsert singleton installation identity in database",
        tag = "Identity",
        security = "bearerAuth"
    )]
    #[body(json = UpdateInstallationIdentityDto)]
    #[resp(200, "Updated installation identity", json = InstallationIdentityRecord)]
    #[resp(400, "Invalid installation identity format", json = ErrorResponse)]
    #[resp(403, "Admin scope required", json = ErrorResponse)]
    async fn update_installation(
        &self,
        #[body] dto: UpdateInstallationIdentityDto,
    ) -> Result<Json<InstallationIdentityRecord>, HttpError> {
        validate_dto(&dto)?;
        let updated = self.service.update_installation_identity(&dto).await?;
        Ok(Json(updated))
    }

    #[get("/ledger")]
    #[guard(ScopeGuard::new(ApiKeyScope::Admin))]
    #[interceptor(PerRouteTimeoutInterceptor::secs(5))]
    #[api(
        summary = "Query indefinite agent interaction ledger",
        tag = "Identity",
        security = "bearerAuth"
    )]
    #[resp(200, "Agent interaction ledger entries", json = LedgerListResponse)]
    #[resp(403, "Admin scope required", json = ErrorResponse)]
    async fn list_ledger(
        &self,
        #[query] query: LedgerQueryDto,
    ) -> Result<Json<LedgerListResponse>, HttpError> {
        validate_dto(&query)?;
        let list = self.service.list_ledger(query).await?;
        Ok(Json(list))
    }
}
