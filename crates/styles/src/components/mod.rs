use crate::tokens::ColorRgba;
use serde::{Deserialize, Serialize};

pub mod controls;
pub mod surfaces;

pub use controls::{
    BadgeSpec, BadgeVariant, ButtonSize, ButtonSpec, ButtonVariant, FieldSpec, InputGroupSpec,
    InputSpec, LabelSpec, TextareaSpec,
};
pub use surfaces::{
    AspectRatioSpec, BreadcrumbItem, BreadcrumbSpec, CardSection, CardSpec, CardVariant,
    ChartSeriesSpec, CommandItemSpec, CommandPaletteSpec, DialogSpec, DropdownItemSpec,
    DropdownMenuSpec, EmptyStateSpec, SeparatorOrientation, SeparatorSpec, TableColumnSpec,
    TableSpec,
};

/// Visible interaction state passed to every styled component resolver.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractionState {
    #[default]
    Default,
    Hover,
    Pressed,
    Focused,
    Disabled,
    Selected,
    Invalid,
    Loading,
}

/// Platform-appropriate sizing density (`Desktop` compact, `Mobile` touch targets).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Density {
    Compact,
    #[default]
    Comfortable,
    Touch,
}

impl Density {
    /// Minimum interactive hit target height in points (`44.0` for mobile touch).
    #[must_use]
    pub const fn min_target_height_pt(self) -> f32 {
        match self {
            Self::Compact => 28.0,
            Self::Comfortable => 32.0,
            Self::Touch => 44.0,
        }
    }
}

/// Resolved visual styling properties computed from [`crate::tokens::Theme`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComponentStyle {
    pub background: ColorRgba,
    pub foreground: ColorRgba,
    pub border: ColorRgba,
    pub focus_ring: Option<ColorRgba>,
    pub height_pt: f32,
    pub padding_x_pt: f32,
    pub padding_y_pt: f32,
    pub radius_pt: f32,
    pub border_width_pt: f32,
    pub opacity: f32,
}
