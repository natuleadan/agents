use agents_protocol::{InteractionMode, SetupMode, WorkerStatus};
use agents_styles::ThemeMode;
use serde::{Deserialize, Serialize};

use super::backend::{BackendTarget, ControlAction, EffectiveProfile};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActiveView {
    #[default]
    Onboarding,
    Workspace,
    Swarm,
    Preview,
    CommandPalette,
    Settings,
    UpdateModal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerEntry {
    pub id: String,
    pub role: String,
    pub status: WorkerStatus,
    pub current_task: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThinkingEntry {
    pub id: String,
    pub title: String,
    pub detail: String,
    pub collapsed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentEntry {
    pub name: String,
    pub mime_type: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffLineKind {
    Context,
    Added,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffHunk {
    pub file_path: String,
    pub header: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextMeter {
    pub used_tokens: u64,
    pub max_tokens: u64,
    pub estimated_cost_usd: f64,
}

impl Default for ContextMeter {
    fn default() -> Self {
        Self {
            used_tokens: 0,
            max_tokens: 200_000,
            estimated_cost_usd: 0.0,
        }
    }
}

impl ContextMeter {
    #[must_use]
    pub fn ratio(&self) -> f64 {
        if self.max_tokens == 0 {
            0.0
        } else {
            (self.used_tokens as f64 / self.max_tokens as f64).clamp(0.0, 1.0)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentDrawerMessage {
    pub id: String,
    pub from_role: String,
    pub summary: String,
    pub requires_approval: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwarmForkEntry {
    pub id: String,
    pub branch: String,
    pub parent_branch: String,
    pub status: WorkerStatus,
    pub changed_files: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewEntry {
    pub route_id: String,
    pub title: String,
    pub url: String,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct NotificationCue {
    pub attention_needed: bool,
    pub completion_chimes: u32,
    pub last_event_label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateInfo {
    pub current_version: String,
    pub available_version: Option<String>,
    pub notes: String,
}

impl Default for UpdateInfo {
    fn default() -> Self {
        Self {
            current_version: "0.1.0".to_owned(),
            available_version: None,
            notes: "Up to date".to_owned(),
        }
    }
}

/// Complete cross-surface state shared cleanly across `client-tui` and `client-ui`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppState {
    pub active_view: ActiveView,
    pub focus_index: usize,
    pub setup_mode: SetupMode,
    pub effective_profile: EffectiveProfile,
    pub theme_mode: ThemeMode,
    pub provider_choices: Vec<String>,
    pub selected_provider_idx: usize,
    pub model_choices: Vec<String>,
    pub selected_model_idx: usize,
    pub interaction_mode: InteractionMode,
    pub session_id: Option<String>,
    pub composer_text: String,
    pub attachments: Vec<AttachmentEntry>,
    pub stream_output: String,
    pub thinking_steps: Vec<ThinkingEntry>,
    pub workers: Vec<WorkerEntry>,
    pub diffs: Vec<DiffHunk>,
    pub subagent_messages: Vec<SubagentDrawerMessage>,
    pub swarm_forks: Vec<SwarmForkEntry>,
    pub preview_route: Option<PreviewEntry>,
    pub context_meter: ContextMeter,
    pub command_query: String,
    pub slash_commands: Vec<String>,
    pub notifications: NotificationCue,
    pub update_info: UpdateInfo,
    pub offline: bool,
    pub last_error: Option<String>,
}

impl Default for AppState {
    fn default() -> Self {
        let target = BackendTarget::local_server("http://127.0.0.1:8080");
        Self {
            active_view: ActiveView::Onboarding,
            focus_index: 0,
            setup_mode: SetupMode::Local,
            effective_profile: target.effective_profile(),
            theme_mode: ThemeMode::Dark,
            provider_choices: vec![
                "agents-server".to_owned(),
                "upstash_box".to_owned(),
                "daytona".to_owned(),
                "deepinfra".to_owned(),
                "instabox".to_owned(),
            ],
            selected_provider_idx: 0,
            model_choices: vec![
                "claude-sonnet-4-5".to_owned(),
                "claude-opus-4-5".to_owned(),
                "gpt-5-codex".to_owned(),
                "gemini-2.5-pro".to_owned(),
            ],
            selected_model_idx: 0,
            interaction_mode: InteractionMode::Plan,
            session_id: None,
            composer_text: String::new(),
            attachments: Vec::new(),
            stream_output: String::new(),
            thinking_steps: Vec::new(),
            workers: vec![WorkerEntry {
                id: "primary".to_owned(),
                role: "orchestrator".to_owned(),
                status: WorkerStatus::Idle,
                current_task: "Ready".to_owned(),
            }],
            diffs: Vec::new(),
            subagent_messages: Vec::new(),
            swarm_forks: Vec::new(),
            preview_route: None,
            context_meter: ContextMeter::default(),
            command_query: String::new(),
            slash_commands: vec![
                "/plan".to_owned(),
                "/act".to_owned(),
                "/pause".to_owned(),
                "/resume".to_owned(),
                "/swarm".to_owned(),
                "/preview".to_owned(),
            ],
            notifications: NotificationCue::default(),
            update_info: UpdateInfo::default(),
            offline: false,
            last_error: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AppAction {
    SetView(ActiveView),
    FocusNext,
    FocusPrev,
    SelectSetupMode(SetupMode),
    SelectProvider(usize),
    SelectModel(usize),
    ToggleInteractionMode,
    ToggleThemeMode,
    UpdateEffectiveProfile(EffectiveProfile),
    UpdateComposer(String),
    AddAttachment(AttachmentEntry),
    SetSession(String),
    AppendStreamChunk(String),
    AddThinkingStep(ThinkingEntry),
    ToggleThinkingCollapse(String),
    SetWorker(WorkerEntry),
    AddDiff(DiffHunk),
    PushSubagentMessage(SubagentDrawerMessage),
    UpsertSwarmFork(SwarmForkEntry),
    SetPreview(PreviewEntry),
    UpdateContextMeter(ContextMeter),
    SetCommandQuery(String),
    ControlTriggered(ControlAction),
    RingAttention(String),
    RingCompletion(String),
    SetUpdateInfo(UpdateInfo),
    SetOffline(bool),
    SetError(Option<String>),
}

/// Pure state transition reducer ensuring deterministic behavior across TUI, desktop, mobile, and Web.
pub fn dispatch(state: &mut AppState, action: AppAction) {
    match action {
        AppAction::SetView(view) => state.active_view = view,
        AppAction::FocusNext => state.focus_index = (state.focus_index + 1) % 6,
        AppAction::FocusPrev => {
            state.focus_index = if state.focus_index == 0 {
                5
            } else {
                state.focus_index - 1
            };
        }
        AppAction::SelectSetupMode(mode) => {
            state.setup_mode = mode;
            state.effective_profile = match mode {
                SetupMode::Local => {
                    BackendTarget::local_server("http://127.0.0.1:8080").effective_profile()
                }
                SetupMode::Remote => {
                    BackendTarget::remote_server("https://agents.internal").effective_profile()
                }
            };
        }
        AppAction::SelectProvider(idx) => {
            if idx < state.provider_choices.len() {
                state.selected_provider_idx = idx;
            }
        }
        AppAction::SelectModel(idx) => {
            if idx < state.model_choices.len() {
                state.selected_model_idx = idx;
            }
        }
        AppAction::ToggleInteractionMode => {
            state.interaction_mode = match state.interaction_mode {
                InteractionMode::Plan => InteractionMode::Act,
                InteractionMode::Act => InteractionMode::Plan,
            };
        }
        AppAction::ToggleThemeMode => {
            state.theme_mode = match state.theme_mode {
                ThemeMode::Dark => ThemeMode::Light,
                ThemeMode::Light => ThemeMode::System,
                ThemeMode::System => ThemeMode::Dark,
            };
        }
        AppAction::UpdateEffectiveProfile(profile) => {
            state.setup_mode = profile.setup_mode;
            state.effective_profile = profile;
        }
        AppAction::UpdateComposer(text) => state.composer_text = text,
        AppAction::AddAttachment(item) => state.attachments.push(item),
        AppAction::SetSession(id) => {
            state.session_id = Some(id);
            state.active_view = ActiveView::Workspace;
        }
        AppAction::AppendStreamChunk(chunk) => state.stream_output.push_str(&chunk),
        AppAction::AddThinkingStep(step) => state.thinking_steps.push(step),
        AppAction::ToggleThinkingCollapse(id) => {
            if let Some(step) = state.thinking_steps.iter_mut().find(|s| s.id == id) {
                step.collapsed = !step.collapsed;
            }
        }
        AppAction::SetWorker(worker) => {
            if let Some(existing) = state.workers.iter_mut().find(|w| w.id == worker.id) {
                *existing = worker;
            } else {
                state.workers.push(worker);
            }
        }
        AppAction::AddDiff(hunk) => state.diffs.push(hunk),
        AppAction::PushSubagentMessage(msg) => state.subagent_messages.push(msg),
        AppAction::UpsertSwarmFork(fork) => {
            if let Some(existing) = state.swarm_forks.iter_mut().find(|f| f.id == fork.id) {
                *existing = fork;
            } else {
                state.swarm_forks.push(fork);
            }
        }
        AppAction::SetPreview(preview) => state.preview_route = Some(preview),
        AppAction::UpdateContextMeter(meter) => state.context_meter = meter,
        AppAction::SetCommandQuery(query) => state.command_query = query,
        AppAction::ControlTriggered(ctrl) => {
            state.notifications.last_event_label = Some(format!("control:{}", ctrl.as_str()));
        }
        AppAction::RingAttention(reason) => {
            state.notifications.attention_needed = true;
            state.notifications.last_event_label = Some(reason);
        }
        AppAction::RingCompletion(label) => {
            state.notifications.attention_needed = false;
            state.notifications.completion_chimes += 1;
            state.notifications.last_event_label = Some(label);
        }
        AppAction::SetUpdateInfo(info) => state.update_info = info,
        AppAction::SetOffline(offline) => state.offline = offline,
        AppAction::SetError(err) => state.last_error = err,
    }
}
