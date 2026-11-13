use agents_protocol::WorkerStatus;
use agents_styles::{ColorRgba, PaletteTokens, Theme, ThemeMode};
use ratatui::style::{Color, Modifier, Style};

/// Maps [`agents_styles::Theme`] design tokens cleanly to `ratatui` terminal styles.
#[derive(Debug, Clone)]
pub struct TuiTheme {
    pub bg: Color,
    pub fg: Color,
    pub card: Color,
    pub muted: Color,
    pub muted_fg: Color,
    pub primary: Color,
    pub primary_fg: Color,
    pub border: Color,
    pub focus_ring: Color,
    pub added_fg: Color,
    pub removed_fg: Color,
    pub status_working: Color,
    pub status_blocked: Color,
    pub status_done: Color,
    pub status_idle: Color,
}

impl TuiTheme {
    #[must_use]
    pub fn from_mode(mode: ThemeMode) -> Self {
        let theme = Theme::with_mode(mode, true);
        Self::from_palette(theme.palette())
    }

    #[must_use]
    pub fn from_palette(p: &PaletteTokens) -> Self {
        Self {
            bg: to_rgb(p.background),
            fg: to_rgb(p.foreground),
            card: to_rgb(p.card),
            muted: to_rgb(p.muted),
            muted_fg: to_rgb(p.muted_foreground),
            primary: to_rgb(p.primary),
            primary_fg: to_rgb(p.primary_foreground),
            border: to_rgb(p.border),
            focus_ring: to_rgb(p.ring),
            added_fg: to_rgb(p.status_done),
            removed_fg: to_rgb(p.destructive),
            status_working: to_rgb(p.status_working),
            status_blocked: to_rgb(p.status_blocked),
            status_done: to_rgb(p.status_done),
            status_idle: to_rgb(p.status_idle),
        }
    }

    #[must_use]
    pub fn block_border_style(&self, focused: bool) -> Style {
        if focused {
            Style::default()
                .fg(self.primary)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(self.border)
        }
    }

    #[must_use]
    pub fn worker_style(&self, status: &WorkerStatus) -> Style {
        let color = match status {
            WorkerStatus::Working => self.status_working,
            WorkerStatus::Blocked => self.status_blocked,
            WorkerStatus::Done => self.status_done,
            WorkerStatus::Idle | WorkerStatus::Unknown(_) => self.status_idle,
        };
        Style::default().fg(color).add_modifier(Modifier::BOLD)
    }
}

#[must_use]
const fn to_rgb(c: ColorRgba) -> Color {
    Color::Rgb(c.r, c.g, c.b)
}
