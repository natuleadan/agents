#![forbid(unsafe_code)]
//! Reusable design system, component specifications, and NSH-inspired reactive
//! cross-platform UI primitives for `natuleadan/agents` and companion Rust apps.
//!
//! Architecture follows the three-layer Native Shell (NSH) model:
//! 1. **Tokens** ([`tokens`]): Light/dark semantic color palettes, typography, spacing, and radii
//!    compatible with CSS custom properties and NSH `cssVars` theme JSON.
//! 2. **Components** ([`components`]): Reusable styled control and surface definitions with
//!    explicit variants, sizes, density modes, and visible interaction states.
//! 3. **Layout & Elements** ([`nsh`]): Platform-idiom-aware node tree (`Desktop`, `Web`, `Mobile`,
//!    `Terminal`), fine-grained reactive signals, and HTML/text projection renderers.

pub mod components;
pub mod nsh;
pub mod tokens;

pub use components::{BadgeVariant, ButtonSize, ButtonSpec, ButtonVariant, CardSpec, Density};
pub use nsh::{Idiom, NshNode, ResponsiveClass, Signal, SurfaceMetrics, TargetPlatform};
pub use tokens::{Appearance, ColorRgba, PaletteTokens, Theme, ThemeError, ThemeMode};

pub mod prelude {
    pub use crate::components::{
        AspectRatioSpec, BadgeSpec, BadgeVariant, BreadcrumbItem, BreadcrumbSpec, ButtonSize,
        ButtonSpec, ButtonVariant, CardSection, CardSpec, CardVariant, ChartSeriesSpec,
        CommandItemSpec, CommandPaletteSpec, ComponentStyle, Density, DialogSpec, DropdownItemSpec,
        DropdownMenuSpec, EmptyStateSpec, FieldSpec, InputGroupSpec, InputSpec, InteractionState,
        LabelSpec, SeparatorOrientation, SeparatorSpec, TableColumnSpec, TableSpec, TextareaSpec,
    };
    pub use crate::nsh::{
        AccessibilityRole, AlignItems, FlexDirection, Idiom, JustifyContent, NshNode,
        ResponsiveClass, Signal, SurfaceMetrics, TargetPlatform, ViewStyle,
    };
    pub use crate::tokens::{
        Appearance, ColorRgba, PaletteTokens, RadiusTokens, SpacingScale, Theme, ThemeError,
        ThemeMode, TypographyTokens,
    };
}
