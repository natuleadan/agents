//! Public ingress HTTP controller for dynamic webhooks and rendered slug outputs.

use std::sync::Arc;

use ironic::{HeaderName, HeaderValue, HttpStatus, prelude::*};

use crate::modules::workflows::{
    dto::WebhookPayloadDto, entities::WorkflowRunEntity, services::WorkflowService,
};

/// Public webhook trigger controller for dynamic `/webhooks/:slug` ingress.
#[controller("/webhooks")]
#[derive(Injectable)]
pub struct WebhookIngressController {
    service: Arc<WorkflowService>,
}

#[routes]
impl WebhookIngressController {
    #[post("/:slug")]
    #[api(
        summary = "Trigger dynamic workflow via webhook slug",
        tag = "Workflows"
    )]
    #[body(json = WebhookPayloadDto)]
    #[resp(200, "Workflow run triggered by webhook", json = WorkflowRunEntity)]
    async fn trigger_slug(
        &self,
        #[param] slug: String,
        #[body] dto: Option<WebhookPayloadDto>,
    ) -> Result<Json<WorkflowRunEntity>, HttpError> {
        let input = dto
            .and_then(|d| d.payload)
            .unwrap_or_else(|| serde_json::json!({}));
        let run = self.service.trigger_by_webhook_slug(&slug, input).await?;
        Ok(Json(run))
    }
}

/// Public renderer controller for `/render/:slug` dynamic HTML and artifacts.
#[controller("/render")]
#[derive(Injectable)]
pub struct SlugRenderController {
    service: Arc<WorkflowService>,
}

#[routes]
impl SlugRenderController {
    #[get("/:slug")]
    #[api(
        summary = "Get dynamically rendered workflow artifact or page",
        tag = "Workflows"
    )]
    #[resp(200, "Rendered HTML content")]
    async fn render_slug(&self, #[param] slug: String) -> Result<Response, HttpError> {
        let rendered = self.service.get_render(&slug).await?;
        let mut resp = Response::bytes(HttpStatus::OK, rendered.content.into_bytes());
        let content_type = HeaderValue::from_str(&rendered.content_type)
            .unwrap_or_else(|_| HeaderValue::from_static("text/html; charset=utf-8"));
        resp.headers_mut()
            .insert(HeaderName::from_static("content-type"), content_type);
        Ok(resp)
    }
}
