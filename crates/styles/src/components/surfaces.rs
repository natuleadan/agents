use crate::components::ComponentStyle;
use crate::tokens::Theme;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardVariant {
    #[default]
    Default,
    Sm,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardSection {
    pub title: String,
    pub description: Option<String>,
    pub footer: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardSpec {
    pub variant: CardVariant,
    pub section: CardSection,
}

impl CardSpec {
    #[must_use]
    pub fn resolve(&self, theme: &Theme) -> ComponentStyle {
        let p = theme.palette();
        let pad = match self.variant {
            CardVariant::Default => 16.0,
            CardVariant::Sm => 12.0,
        };
        ComponentStyle {
            background: p.card,
            foreground: p.card_foreground,
            border: p.border,
            focus_ring: None,
            height_pt: 0.0,
            padding_x_pt: pad,
            padding_y_pt: pad,
            radius_pt: theme.radius.xl(),
            border_width_pt: 1.0,
            opacity: 1.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandItemSpec {
    pub id: String,
    pub group: String,
    pub label: String,
    pub shortcut: Option<String>,
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandPaletteSpec {
    pub placeholder: String,
    pub query: String,
    pub empty_message: String,
    pub items: Vec<CommandItemSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DialogSpec {
    pub title: String,
    pub description: Option<String>,
    pub show_close_button: bool,
    pub destructive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DropdownItemSpec {
    pub id: String,
    pub label: String,
    pub shortcut: Option<String>,
    pub destructive: bool,
    pub checked: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DropdownMenuSpec {
    pub label: String,
    pub items: Vec<DropdownItemSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BreadcrumbItem {
    pub label: String,
    pub current: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BreadcrumbSpec {
    pub items: Vec<BreadcrumbItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableColumnSpec {
    pub key: String,
    pub heading: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableSpec {
    pub caption: Option<String>,
    pub columns: Vec<TableColumnSpec>,
    pub rows: Vec<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmptyStateSpec {
    pub title: String,
    pub description: String,
    pub action_label: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeparatorOrientation {
    #[default]
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeparatorSpec {
    pub orientation: SeparatorOrientation,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AspectRatioSpec {
    pub ratio: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChartSeriesSpec {
    pub id: String,
    pub label: String,
    pub points: Vec<f64>,
    pub palette_index: usize,
}
