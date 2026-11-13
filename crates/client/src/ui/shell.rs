use std::sync::Arc;

use agents_protocol::{EventEnvelope, InteractionMode, TokenPair};
use agents_styles::prelude::*;

use super::{data_views, login, panels};
use crate::core::ClientError;
use crate::core::backend::{
    ActionReceipt, BackendTarget, ControlAction, SandboxProvider, SessionSummary, TaskSubmission,
};
use crate::core::state::{ActiveView, AppAction, AppState, AuthState, PreviewEntry, dispatch};
use crate::core::store::{
    ChunkCoalescer, CoalesceOutcome, DraftRecord, DraftStore, IdempotentActionQueue,
    OfflineStartupState, SecretStore,
};

/// Builds the complete responsive NSH [`NshNode`] tree tailored to `metrics.target`.
#[must_use]
pub fn render_app_shell(state: &AppState, metrics: SurfaceMetrics) -> NshNode {
    let system_dark = metrics.target.idiom() != Idiom::Web;
    let theme = Theme::with_mode(state.theme_mode, system_dark);
    let compact = metrics.prefers_bottom_sheet();
    let p = theme.palette();

    let header = panels::build_header_bar(state, &theme, compact);
    let nav = panels::build_navigation(state, metrics, &theme);
    let workspace = panels::build_workspace_panel(state, &theme);
    let cluster_metrics = data_views::build_cluster_and_metrics_panel(state, &theme);
    let preview = panels::build_preview_panel(state, &theme);

    let center_col = NshNode::view("center-column")
        .direction(FlexDirection::Column)
        .gap(theme.spacing.md)
        .child(workspace)
        .child(cluster_metrics);

    let body = if compact {
        NshNode::view("mobile-shell")
            .direction(FlexDirection::Column)
            .gap(theme.spacing.sm)
            .child(center_col)
            .child(preview)
            .child(nav)
    } else {
        NshNode::view("desktop-shell")
            .direction(FlexDirection::Row)
            .gap(theme.spacing.md)
            .child(nav)
            .child(center_col)
            .child(preview)
    };

    let target_tag = NshNode::text(
        "target-platform-meta",
        format!(
            "target:{} ({:?})",
            metrics.target.as_str(),
            metrics.responsive_class()
        ),
    )
    .fg(p.muted_foreground);

    let mut root = NshNode::view("app-root")
        .direction(FlexDirection::Column)
        .bg(p.background)
        .fg(p.foreground)
        .child(header);

    if state.active_view == ActiveView::Onboarding
        || state.login.auth_state != AuthState::Authenticated
    {
        root = root.child(login::build_login_screen(state, metrics, &theme));
    }

    root = root.child(body);

    if matches!(
        state.active_view,
        ActiveView::CommandPalette | ActiveView::UpdateModal
    ) {
        root = root.child(data_views::build_command_and_update_dialogs(state, &theme));
    }

    root.child(target_tag)
}

/// High-level client orchestrator tying [`SandboxProvider`], stores, queues, and UI state.
pub struct ClientController {
    provider: Arc<dyn SandboxProvider>,
    secrets: Arc<dyn SecretStore>,
    drafts: DraftStore,
    queue: IdempotentActionQueue,
    coalescer: ChunkCoalescer,
    tokens: Option<TokenPair>,
    pub state: AppState,
}

impl ClientController {
    #[must_use]
    pub fn new(
        provider: Arc<dyn SandboxProvider>,
        secrets: Arc<dyn SecretStore>,
        target: BackendTarget,
    ) -> Self {
        let mut state = AppState::default();
        dispatch(
            &mut state,
            AppAction::UpdateEffectiveProfile(target.effective_profile()),
        );
        Self {
            provider,
            secrets,
            drafts: DraftStore::default(),
            queue: IdempotentActionQueue::default(),
            coalescer: ChunkCoalescer::default(),
            tokens: None,
            state,
        }
    }

    pub fn boot_offline_or_saved(&mut self) -> Result<OfflineStartupState, ClientError> {
        let profile = self.drafts.load_startup_profile()?;
        dispatch(
            &mut self.state,
            AppAction::UpdateEffectiveProfile(profile.target.effective_profile()),
        );
        if let Some(sid) = &profile.last_session_id
            && let Some(draft) = self.drafts.load_draft(sid)?
        {
            dispatch(&mut self.state, AppAction::UpdateComposer(draft.prompt));
        }
        Ok(profile)
    }

    pub async fn login_and_persist(
        &mut self,
        email: &str,
        password: &str,
    ) -> Result<(), ClientError> {
        dispatch(
            &mut self.state,
            AppAction::SetAuthState(AuthState::Authenticating, None),
        );
        let pair = match self.provider.login(email, password).await {
            Ok(p) => p,
            Err(err) => {
                let auth_err = match &err {
                    ClientError::HttpStatus { status: 429, .. } => AuthState::RateLimited,
                    ClientError::Offline(_) | ClientError::Transport(_) => AuthState::Offline,
                    _ => AuthState::InvalidCredentials,
                };
                dispatch(
                    &mut self.state,
                    AppAction::SetAuthState(auth_err, Some(err.to_string())),
                );
                return Err(err);
            }
        };
        self.secrets
            .store_secret("access_token", &pair.access_token)?;
        if !pair.refresh_token.is_empty() {
            self.secrets
                .store_secret("refresh_token", &pair.refresh_token)?;
        }
        self.tokens = Some(pair);
        dispatch(
            &mut self.state,
            AppAction::SetAuthState(AuthState::Authenticated, None),
        );
        Ok(())
    }

    pub async fn start_session(
        &mut self,
        title: &str,
        mode: InteractionMode,
    ) -> Result<SessionSummary, ClientError> {
        let token = self.access_token()?;
        let summary = self.provider.create_session(&token, title, mode).await?;
        dispatch(&mut self.state, AppAction::SetSession(summary.id.clone()));
        Ok(summary)
    }

    pub async fn submit_prompt(&mut self, prompt: &str) -> Result<TaskSubmission, ClientError> {
        let sid = self
            .state
            .session_id
            .clone()
            .ok_or_else(|| ClientError::Offline("no active session".to_owned()))?;
        self.drafts.save_draft(DraftRecord {
            session_id: sid.clone(),
            prompt: prompt.to_owned(),
            attachments: self
                .state
                .attachments
                .iter()
                .map(|a| a.name.clone())
                .collect(),
            updated_at_ms: 0,
        })?;
        let token = self.access_token()?;
        let submission = self
            .provider
            .submit_task(&token, &sid, prompt, self.state.interaction_mode)
            .await?;
        let _ = self.drafts.clear_draft(&sid);
        Ok(submission)
    }

    pub fn ingest_stream_envelope(&mut self, env: EventEnvelope) -> Vec<CoalesceOutcome> {
        let outcomes = self.coalescer.ingest(env.clone());
        for outcome in &outcomes {
            if let CoalesceOutcome::Appended {
                chunk: Some(text), ..
            } = outcome
            {
                dispatch(&mut self.state, AppAction::AppendStreamChunk(text.clone()));
            }
        }
        if let agents_protocol::StreamPayload::PreviewPublished(route) = env.payload {
            dispatch(
                &mut self.state,
                AppAction::SetPreview(PreviewEntry {
                    route_id: route.route_id,
                    title: route.slug,
                    url: route.public_url,
                    active: true,
                }),
            );
            dispatch(
                &mut self.state,
                AppAction::RingCompletion("preview_ready".to_owned()),
            );
        }
        outcomes
    }

    pub async fn send_idempotent_control(
        &mut self,
        task_id: &str,
        action: ControlAction,
    ) -> Result<ActionReceipt, ClientError> {
        let queued = self.queue.enqueue(task_id, action);
        let token = self.access_token()?;
        self.queue.mark_attempt(&queued.idempotency_key);
        let receipt = self
            .provider
            .send_control(&token, task_id, action, &queued.idempotency_key)
            .await?;
        self.queue.acknowledge(&queued.idempotency_key);
        dispatch(&mut self.state, AppAction::ControlTriggered(action));
        Ok(receipt)
    }

    #[must_use]
    pub fn coalescer(&self) -> &ChunkCoalescer {
        &self.coalescer
    }

    #[must_use]
    pub fn drafts(&self) -> &DraftStore {
        &self.drafts
    }

    #[must_use]
    pub fn render(&self, metrics: SurfaceMetrics) -> NshNode {
        render_app_shell(&self.state, metrics)
    }

    pub fn evaluate_release_manifest(
        &mut self,
        cfg: &crate::core::UpdaterConfig,
        manifest_bytes: &[u8],
        manifest_sig_b64: Option<&str>,
    ) -> Result<Option<crate::core::AvailableUpdate>, crate::core::UpdateError> {
        let found = crate::core::check_manifest(cfg, manifest_bytes, manifest_sig_b64)?;
        if let Some(upd) = &found {
            dispatch(
                &mut self.state,
                AppAction::SetUpdateInfo(crate::core::UpdateInfo {
                    current_version: cfg.current_version.clone(),
                    available_version: Some(upd.version.clone()),
                    notes: upd.notes.clone(),
                }),
            );
        }
        Ok(found)
    }

    fn access_token(&self) -> Result<String, ClientError> {
        if let Some(pair) = &self.tokens {
            return Ok(pair.access_token.clone());
        }
        self.secrets
            .load_secret("access_token")?
            .ok_or_else(|| ClientError::Auth("missing access token".to_owned()))
    }
}
