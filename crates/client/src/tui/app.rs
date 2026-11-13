use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::layout::{Constraint, Direction, Layout};

use super::theme::TuiTheme;
use super::widgets;
use crate::core::ClientError;
use crate::core::state::AppState;

/// Renders one deterministic TUI frame using any [`ratatui::backend::Backend`] (including `TestBackend`).
pub fn render_tui<B: Backend>(
    terminal: &mut Terminal<B>,
    state: &AppState,
) -> Result<(), ClientError> {
    let theme = TuiTheme::from_mode(state.theme_mode);
    terminal
        .draw(|frame| {
            let area = frame.area();
            let vertical = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(2),
                    Constraint::Length(6),
                    Constraint::Min(6),
                    Constraint::Length(2),
                ])
                .split(area);

            widgets::render_header(frame, vertical[0], state, &theme);
            widgets::render_onboarding_selectors(frame, vertical[1], state, &theme);
            widgets::render_workers_and_stream(frame, vertical[2], state, &theme);
            widgets::render_footer(frame, vertical[3], state, &theme);
            widgets::render_command_overlay(frame, area, state, &theme);
        })
        .map_err(|e| ClientError::Transport(format!("tui render failed: {e}")))?;
    Ok(())
}

/// Interactive desktop terminal event loop using `crossterm` when running `agents-client tui`.
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
pub fn run_terminal_tui(initial: AppState) -> Result<AppState, ClientError> {
    use crossterm::event::{self, Event, KeyCode, KeyModifiers};
    use crossterm::execute;
    use crossterm::terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    };
    use ratatui::backend::CrosstermBackend;
    use std::io;
    use std::time::Duration;

    use super::{TuiKey, map_key};
    use crate::core::state::dispatch;

    enable_raw_mode().map_err(|e| ClientError::Transport(e.to_string()))?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).map_err(|e| ClientError::Transport(e.to_string()))?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).map_err(|e| ClientError::Transport(e.to_string()))?;

    let mut state = initial;
    loop {
        render_tui(&mut terminal, &state)?;
        if event::poll(Duration::from_millis(100))
            .map_err(|e| ClientError::Transport(e.to_string()))?
            && let Event::Key(key) =
                event::read().map_err(|e| ClientError::Transport(e.to_string()))?
        {
            if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                break;
            }
            let mapped = match (key.code, key.modifiers) {
                (KeyCode::Tab, _) => Some(TuiKey::Tab),
                (KeyCode::BackTab, _) => Some(TuiKey::BackTab),
                (KeyCode::Up, _) => Some(TuiKey::Up),
                (KeyCode::Down, _) => Some(TuiKey::Down),
                (KeyCode::Left, _) => Some(TuiKey::Left),
                (KeyCode::Right, _) => Some(TuiKey::Right),
                (KeyCode::Enter, _) => Some(TuiKey::Enter),
                (KeyCode::Esc, _) => Some(TuiKey::Escape),
                (KeyCode::Char('k'), m) if m.contains(KeyModifiers::CONTROL) => Some(TuiKey::CtrlK),
                (KeyCode::Char(c), _) => Some(TuiKey::Char(c)),
                _ => None,
            };
            if let Some(tk) = mapped
                && let Some(action) = map_key(&state, tk)
            {
                dispatch(&mut state, action);
            }
        }
    }

    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let _ = terminal.show_cursor();
    Ok(state)
}
