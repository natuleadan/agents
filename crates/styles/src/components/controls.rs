use crate::components::{ComponentStyle, Density, InteractionState};
use crate::tokens::{ColorRgba, Theme};
use serde::{Deserialize, Serialize};

/// Semantic button variants aligned with the NLA component system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ButtonVariant {
    #[default]
    Default,
    Outline,
    Secondary,
    Ghost,
    Destructive,
    Link,
}

/// Explicit button sizing tiers including square icon buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ButtonSize {
    #[default]
    Default,
    Xs,
    Sm,
    Lg,
    Icon,
    IconXs,
    IconSm,
    IconLg,
}

impl ButtonSize {
    #[must_use]
    pub const fn height_pt(self, density: Density) -> f32 {
        let base = match self {
            Self::Xs | Self::IconXs => 24.0,
            Self::Sm | Self::IconSm => 28.0,
            Self::Default | Self::Icon => 32.0,
            Self::Lg | Self::IconLg => 36.0,
        };
        let floor = density.min_target_height_pt();
        if base < floor { floor } else { base }
    }

    #[must_use]
    pub const fn padding_x_pt(self) -> f32 {
        match self {
            Self::Xs => 8.0,
            Self::Sm => 10.0,
            Self::Default => 12.0,
            Self::Lg => 16.0,
            Self::Icon | Self::IconXs | Self::IconSm | Self::IconLg => 0.0,
        }
    }
}

/// Reusable button specification resolved deterministically against a [`Theme`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ButtonSpec {
    pub label: String,
    pub variant: ButtonVariant,
    pub size: ButtonSize,
    pub state: InteractionState,
}

impl ButtonSpec {
    #[must_use]
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            variant: ButtonVariant::Default,
            size: ButtonSize::Default,
            state: InteractionState::Default,
        }
    }

    #[must_use]
    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }

    #[must_use]
    pub fn size(mut self, size: ButtonSize) -> Self {
        self.size = size;
        self
    }

    #[must_use]
    pub fn state(mut self, state: InteractionState) -> Self {
        self.state = state;
        self
    }

    #[must_use]
    pub fn resolve(&self, theme: &Theme, density: Density) -> ComponentStyle {
        let p = theme.palette();
        let (bg, fg, border, border_w) = match self.variant {
            ButtonVariant::Default => (p.primary, p.primary_foreground, p.primary, 0.0),
            ButtonVariant::Outline => (p.background, p.foreground, p.border, 1.0),
            ButtonVariant::Secondary => (p.secondary, p.secondary_foreground, p.secondary, 0.0),
            ButtonVariant::Ghost => (ColorRgba::rgba(0, 0, 0, 0), p.foreground, p.border, 0.0),
            ButtonVariant::Destructive => {
                (p.destructive, p.destructive_foreground, p.destructive, 0.0)
            }
            ButtonVariant::Link => (ColorRgba::rgba(0, 0, 0, 0), p.primary, p.border, 0.0),
        };

        let opacity = match self.state {
            InteractionState::Disabled => 0.5,
            InteractionState::Pressed | InteractionState::Hover => 0.9,
            InteractionState::Loading => 0.75,
            _ => 1.0,
        };

        let focus_ring = matches!(
            self.state,
            InteractionState::Focused | InteractionState::Invalid
        )
        .then_some(if self.state == InteractionState::Invalid {
            p.destructive
        } else {
            p.ring
        });

        ComponentStyle {
            background: bg,
            foreground: fg,
            border: if self.state == InteractionState::Invalid {
                p.destructive
            } else {
                border
            },
            focus_ring,
            height_pt: self.size.height_pt(density),
            padding_x_pt: self.size.padding_x_pt(),
            padding_y_pt: 4.0,
            radius_pt: theme.radius.lg(),
            border_width_pt: border_w,
            opacity,
        }
    }
}

/// Badge visual variants for statuses, modes, and effective profile pills.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BadgeVariant {
    #[default]
    Default,
    Secondary,
    Destructive,
    Outline,
    Ghost,
    Link,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BadgeSpec {
    pub text: String,
    pub variant: BadgeVariant,
}

impl BadgeSpec {
    #[must_use]
    pub fn new(text: impl Into<String>, variant: BadgeVariant) -> Self {
        Self {
            text: text.into(),
            variant,
        }
    }

    #[must_use]
    pub fn resolve(&self, theme: &Theme) -> ComponentStyle {
        let p = theme.palette();
        let (bg, fg, border, border_w) = match self.variant {
            BadgeVariant::Default => (p.primary, p.primary_foreground, p.primary, 0.0),
            BadgeVariant::Secondary => (p.secondary, p.secondary_foreground, p.secondary, 0.0),
            BadgeVariant::Destructive => {
                (p.destructive, p.destructive_foreground, p.destructive, 0.0)
            }
            BadgeVariant::Outline => (p.card, p.foreground, p.border, 1.0),
            BadgeVariant::Ghost => (
                ColorRgba::rgba(0, 0, 0, 0),
                p.muted_foreground,
                p.border,
                0.0,
            ),
            BadgeVariant::Link => (ColorRgba::rgba(0, 0, 0, 0), p.primary, p.border, 0.0),
        };
        ComponentStyle {
            background: bg,
            foreground: fg,
            border,
            focus_ring: None,
            height_pt: 20.0,
            padding_x_pt: 8.0,
            padding_y_pt: 2.0,
            radius_pt: 999.0,
            border_width_pt: border_w,
            opacity: 1.0,
        }
    }
}

/// Single-line text input descriptor with validation and focus awareness.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct InputSpec {
    pub placeholder: String,
    pub value: String,
    pub masked: bool,
    pub disabled: bool,
    pub state: InteractionState,
}

impl InputSpec {
    #[must_use]
    pub fn resolve(&self, theme: &Theme, density: Density) -> ComponentStyle {
        let p = theme.palette();
        let border = if self.state == InteractionState::Invalid {
            p.destructive
        } else if self.state == InteractionState::Focused {
            p.ring
        } else {
            p.input
        };
        ComponentStyle {
            background: p.background,
            foreground: p.foreground,
            border,
            focus_ring: (self.state == InteractionState::Focused).then_some(p.ring),
            height_pt: density.min_target_height_pt(),
            padding_x_pt: 10.0,
            padding_y_pt: 4.0,
            radius_pt: theme.radius.lg(),
            border_width_pt: 1.0,
            opacity: if self.state == InteractionState::Disabled {
                0.5
            } else {
                1.0
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextareaSpec {
    pub placeholder: String,
    pub value: String,
    pub min_rows: u16,
    pub state: InteractionState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputGroupSpec {
    pub prefix: Option<String>,
    pub input: InputSpec,
    pub suffix_action: Option<ButtonSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelSpec {
    pub text: String,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldSpec {
    pub label: LabelSpec,
    pub description: Option<String>,
    pub error: Option<String>,
    pub orientation_horizontal: bool,
}
