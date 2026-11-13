use agents_protocol::{SetupMode, WorkerStatus};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Gauge, List, ListItem, Paragraph, Wrap};

use super::theme::TuiTheme;
use crate::core::state::{ActiveView, AppState, DiffLineKind};

pub fn render_header(frame: &mut Frame<'_>, area: Rect, state: &AppState, theme: &TuiTheme) {
    let mode_tag = format!("{:?}", state.interaction_mode).to_ascii_uppercase();
    let notif = if state.notifications.attention_needed {
        " [ATTENTION]"
    } else {
        ""
    };
    let line = Line::from(vec![
        Span::styled(
            " agents ",
            Style::default()
                .bg(theme.primary)
                .fg(theme.primary_fg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            format!("[{}]", state.effective_profile.label),
            Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!("  mode:{mode_tag}{notif}")),
    ]);
    let block = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(theme.block_border_style(false));
    frame.render_widget(Paragraph::new(line).block(block), area);
}

pub fn render_onboarding_selectors(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    theme: &TuiTheme,
) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(30),
            Constraint::Percentage(35),
            Constraint::Percentage(35),
        ])
        .split(area);

    let local_mark = if state.setup_mode == SetupMode::Local {
        "[●]"
    } else {
        "[ ]"
    };
    let remote_mark = if state.setup_mode == SetupMode::Remote {
        "[●]"
    } else {
        "[ ]"
    };
    let setup_items = vec![
        ListItem::new(format!("{local_mark} Local (Docker + Openshell)")),
        ListItem::new(format!("{remote_mark} Remote (Server / Provider)")),
    ];
    frame.render_widget(
        List::new(setup_items).block(
            Block::default()
                .title(" Setup Mode [1/2] ")
                .borders(Borders::ALL)
                .border_style(theme.block_border_style(state.focus_index == 0)),
        ),
        cols[0],
    );

    let provider_items: Vec<ListItem<'_>> = state
        .provider_choices
        .iter()
        .enumerate()
        .map(|(idx, name)| {
            let mark = if idx == state.selected_provider_idx {
                "▸ "
            } else {
                "  "
            };
            ListItem::new(format!("{mark}{name}"))
        })
        .collect();
    frame.render_widget(
        List::new(provider_items).block(
            Block::default()
                .title(" Provider [←/→] ")
                .borders(Borders::ALL)
                .border_style(theme.block_border_style(state.focus_index == 1)),
        ),
        cols[1],
    );

    let model_items: Vec<ListItem<'_>> = state
        .model_choices
        .iter()
        .enumerate()
        .map(|(idx, name)| {
            let mark = if idx == state.selected_model_idx {
                "▸ "
            } else {
                "  "
            };
            ListItem::new(format!("{mark}{name}"))
        })
        .collect();
    frame.render_widget(
        List::new(model_items).block(
            Block::default()
                .title(" Model [↑/↓] ")
                .borders(Borders::ALL)
                .border_style(theme.block_border_style(state.focus_index == 2)),
        ),
        cols[2],
    );
}

pub fn render_workers_and_stream(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    theme: &TuiTheme,
) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(34), Constraint::Percentage(66)])
        .split(area);

    let workers: Vec<ListItem<'_>> = state
        .workers
        .iter()
        .map(|w| {
            let badge = match w.status {
                WorkerStatus::Working => "WORKING",
                WorkerStatus::Blocked => "BLOCKED",
                WorkerStatus::Done => "DONE",
                WorkerStatus::Idle => "IDLE",
                WorkerStatus::Unknown(_) => "UNKNOWN",
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!("[{badge}] "), theme.worker_style(&w.status)),
                Span::raw(format!("{}: {}", w.role, w.current_task)),
            ]))
        })
        .collect();
    frame.render_widget(
        List::new(workers).block(
            Block::default()
                .title(" Workers ")
                .borders(Borders::ALL)
                .border_style(theme.block_border_style(state.focus_index == 3)),
        ),
        cols[0],
    );

    let mut lines: Vec<Line<'_>> = Vec::new();
    for step in &state.thinking_steps {
        let icon = if step.collapsed { "▸" } else { "▾" };
        lines.push(Line::from(Span::styled(
            format!("{icon} thinking: {}", step.title),
            Style::default().fg(theme.muted_fg),
        )));
        if !step.collapsed {
            lines.push(Line::from(format!("  {}", step.detail)));
        }
    }
    if !state.stream_output.is_empty() {
        for text_line in state.stream_output.lines() {
            lines.push(Line::from(text_line.to_owned()));
        }
    }
    for hunk in &state.diffs {
        lines.push(Line::from(Span::styled(
            format!("--- {} {}", hunk.file_path, hunk.header),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        for d in &hunk.lines {
            let (prefix, style) = match d.kind {
                DiffLineKind::Added => ("+", Style::default().fg(theme.added_fg)),
                DiffLineKind::Removed => ("-", Style::default().fg(theme.removed_fg)),
                DiffLineKind::Context => (" ", Style::default().fg(theme.fg)),
            };
            lines.push(Line::from(Span::styled(
                format!("{prefix}{}", d.content),
                style,
            )));
        }
    }

    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::default()
                .title(" Stream & Diff Viewer ")
                .borders(Borders::ALL)
                .border_style(theme.block_border_style(state.focus_index == 4)),
        ),
        cols[1],
    );
}

pub fn render_footer(frame: &mut Frame<'_>, area: Rect, state: &AppState, theme: &TuiTheme) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    let pct = (state.context_meter.ratio() * 100.0).round() as u16;
    let label = format!(
        "Context: {}/{} tok · ${:.4}",
        state.context_meter.used_tokens,
        state.context_meter.max_tokens,
        state.context_meter.estimated_cost_usd
    );
    frame.render_widget(
        Gauge::default()
            .gauge_style(Style::default().fg(theme.primary).bg(theme.muted))
            .percent(pct)
            .label(label),
        rows[0],
    );

    let commands = state.slash_commands.join("  ");
    frame.render_widget(
        Paragraph::new(format!(
            "Slash: {commands}  |  Ctrl+K Palette  Tab Focus  m Plan/Act  t Theme"
        ))
        .style(Style::default().fg(theme.muted_fg)),
        rows[1],
    );
}

pub fn render_command_overlay(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    theme: &TuiTheme,
) {
    if state.active_view != ActiveView::CommandPalette {
        return;
    }
    let popup = Rect {
        x: area.x + area.width / 6,
        y: area.y + area.height / 5,
        width: (area.width * 2) / 3,
        height: 8.min(area.height),
    };
    frame.render_widget(Clear, popup);
    let q = state.command_query.to_ascii_lowercase();
    let matches: Vec<ListItem<'_>> = state
        .slash_commands
        .iter()
        .filter(|cmd| q.is_empty() || cmd.to_ascii_lowercase().contains(&q))
        .map(|cmd| ListItem::new(format!("› {cmd}")))
        .collect();
    frame.render_widget(
        List::new(matches).block(
            Block::default()
                .title(format!(" Command Palette [{}] ", state.command_query))
                .borders(Borders::ALL)
                .border_style(theme.block_border_style(true)),
        ),
        popup,
    );
}
