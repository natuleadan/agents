pub mod app;
pub mod theme;
pub mod widgets;

use crate::core::state::{ActiveView, AppAction, AppState};
use agents_protocol::SetupMode;

pub use app::render_tui;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
pub use app::run_terminal_tui;
pub use theme::TuiTheme;

/// Platform-independent key events for deterministic TUI navigation and snapshot testing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TuiKey {
    Tab,
    BackTab,
    Up,
    Down,
    Left,
    Right,
    Enter,
    Escape,
    CtrlK,
    Char(char),
}

/// Maps a keyboard event against [`AppState`] into a pure [`AppAction`].
#[must_use]
pub fn map_key(state: &AppState, key: TuiKey) -> Option<AppAction> {
    match key {
        TuiKey::Tab => Some(AppAction::FocusNext),
        TuiKey::BackTab => Some(AppAction::FocusPrev),
        TuiKey::CtrlK => Some(AppAction::SetView(ActiveView::CommandPalette)),
        TuiKey::Escape => Some(AppAction::SetView(ActiveView::Workspace)),
        TuiKey::Char('1') => Some(AppAction::SelectSetupMode(SetupMode::Local)),
        TuiKey::Char('2') => Some(AppAction::SelectSetupMode(SetupMode::Remote)),
        TuiKey::Char('m') => Some(AppAction::ToggleInteractionMode),
        TuiKey::Char('t') => Some(AppAction::ToggleThemeMode),
        TuiKey::Char('s') => Some(AppAction::SetView(ActiveView::Swarm)),
        TuiKey::Char('p') => Some(AppAction::SetView(ActiveView::Preview)),
        TuiKey::Up if state.selected_model_idx > 0 => {
            Some(AppAction::SelectModel(state.selected_model_idx - 1))
        }
        TuiKey::Down if state.selected_model_idx + 1 < state.model_choices.len() => {
            Some(AppAction::SelectModel(state.selected_model_idx + 1))
        }
        TuiKey::Left if state.selected_provider_idx > 0 => {
            Some(AppAction::SelectProvider(state.selected_provider_idx - 1))
        }
        TuiKey::Right if state.selected_provider_idx + 1 < state.provider_choices.len() => {
            Some(AppAction::SelectProvider(state.selected_provider_idx + 1))
        }
        TuiKey::Enter if state.active_view == ActiveView::Onboarding => {
            Some(AppAction::SetView(ActiveView::Workspace))
        }
        _ => None,
    }
}
