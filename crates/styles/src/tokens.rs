use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Errors returned while constructing or validating theme tokens.
#[derive(Debug, Error)]
pub enum ThemeError {
    #[error("invalid color literal: {0}")]
    InvalidColor(String),
    #[error("theme serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

/// User-selected theme preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    Light,
    Dark,
    #[default]
    System,
}

/// Concrete resolved appearance for rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Appearance {
    Light,
    #[default]
    Dark,
}

impl ThemeMode {
    /// Resolves `System` to an explicit [`Appearance`] given the host preference.
    #[must_use]
    pub fn resolve(self, system_dark: bool) -> Appearance {
        match self {
            Self::Light => Appearance::Light,
            Self::Dark => Appearance::Dark,
            Self::System => {
                if system_dark {
                    Appearance::Dark
                } else {
                    Appearance::Light
                }
            }
        }
    }
}

/// RGBA color value (`0..=255`) usable across TUI, desktop, mobile, and Web targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColorRgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl ColorRgba {
    #[must_use]
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    #[must_use]
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// Parses `#RGB`, `#RRGGBB`, or `#RRGGBBAA` hex color strings.
    pub fn from_hex(input: &str) -> Result<Self, ThemeError> {
        let hex = input.strip_prefix('#').unwrap_or(input);
        match hex.len() {
            3 => {
                let r = u8::from_str_radix(&hex[0..1], 16)
                    .map_err(|_| ThemeError::InvalidColor(input.to_owned()))?;
                let g = u8::from_str_radix(&hex[1..2], 16)
                    .map_err(|_| ThemeError::InvalidColor(input.to_owned()))?;
                let b = u8::from_str_radix(&hex[2..3], 16)
                    .map_err(|_| ThemeError::InvalidColor(input.to_owned()))?;
                Ok(Self::rgb(r * 17, g * 17, b * 17))
            }
            6 => {
                let r = u8::from_str_radix(&hex[0..2], 16)
                    .map_err(|_| ThemeError::InvalidColor(input.to_owned()))?;
                let g = u8::from_str_radix(&hex[2..4], 16)
                    .map_err(|_| ThemeError::InvalidColor(input.to_owned()))?;
                let b = u8::from_str_radix(&hex[4..6], 16)
                    .map_err(|_| ThemeError::InvalidColor(input.to_owned()))?;
                Ok(Self::rgb(r, g, b))
            }
            8 => {
                let r = u8::from_str_radix(&hex[0..2], 16)
                    .map_err(|_| ThemeError::InvalidColor(input.to_owned()))?;
                let g = u8::from_str_radix(&hex[2..4], 16)
                    .map_err(|_| ThemeError::InvalidColor(input.to_owned()))?;
                let b = u8::from_str_radix(&hex[4..6], 16)
                    .map_err(|_| ThemeError::InvalidColor(input.to_owned()))?;
                let a = u8::from_str_radix(&hex[6..8], 16)
                    .map_err(|_| ThemeError::InvalidColor(input.to_owned()))?;
                Ok(Self::rgba(r, g, b, a))
            }
            _ => Err(ThemeError::InvalidColor(input.to_owned())),
        }
    }

    #[must_use]
    pub fn with_alpha(self, alpha: f32) -> Self {
        let clamped = alpha.clamp(0.0, 1.0);
        Self {
            a: (clamped * 255.0).round() as u8,
            ..self
        }
    }

    /// Formats the color as CSS hex (`#rrggbb`) or `rgba(r, g, b, a)` when translucent.
    #[must_use]
    pub fn to_css(self) -> String {
        if self.a == 255 {
            format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
        } else {
            let alpha = f32::from(self.a) / 255.0;
            format!("rgba({}, {}, {}, {:.2})", self.r, self.g, self.b, alpha)
        }
    }

    /// Computes relative WCAG 2.1 luminance in `0.0..=1.0`.
    #[must_use]
    pub fn relative_luminance(self) -> f64 {
        fn channel(v: u8) -> f64 {
            let srgb = f64::from(v) / 255.0;
            if srgb <= 0.03928 {
                srgb / 12.92
            } else {
                ((srgb + 0.055) / 1.055).powf(2.4)
            }
        }
        0.2126 * channel(self.r) + 0.7152 * channel(self.g) + 0.0722 * channel(self.b)
    }

    /// Computes WCAG 2.1 contrast ratio against another opaque surface (`1.0..=21.0`).
    #[must_use]
    pub fn contrast_ratio(self, other: Self) -> f64 {
        let l1 = self.relative_luminance();
        let l2 = other.relative_luminance();
        let (hi, lo) = if l1 >= l2 { (l1, l2) } else { (l2, l1) };
        (hi + 0.05) / (lo + 0.05)
    }
}

/// Complete semantic color palette aligned with NSH `Theme` and shadcn `cssVars`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaletteTokens {
    pub background: ColorRgba,
    pub foreground: ColorRgba,
    pub card: ColorRgba,
    pub card_foreground: ColorRgba,
    pub popover: ColorRgba,
    pub popover_foreground: ColorRgba,
    pub primary: ColorRgba,
    pub primary_foreground: ColorRgba,
    pub secondary: ColorRgba,
    pub secondary_foreground: ColorRgba,
    pub muted: ColorRgba,
    pub muted_foreground: ColorRgba,
    pub accent: ColorRgba,
    pub accent_foreground: ColorRgba,
    pub destructive: ColorRgba,
    pub destructive_foreground: ColorRgba,
    pub border: ColorRgba,
    pub input: ColorRgba,
    pub ring: ColorRgba,
    pub sidebar: ColorRgba,
    pub sidebar_foreground: ColorRgba,
    pub sidebar_primary: ColorRgba,
    pub sidebar_primary_foreground: ColorRgba,
    pub sidebar_accent: ColorRgba,
    pub sidebar_accent_foreground: ColorRgba,
    pub sidebar_border: ColorRgba,
    pub sidebar_ring: ColorRgba,
    pub chart_1: ColorRgba,
    pub chart_2: ColorRgba,
    pub chart_3: ColorRgba,
    pub chart_4: ColorRgba,
    pub chart_5: ColorRgba,
    pub status_working: ColorRgba,
    pub status_blocked: ColorRgba,
    pub status_done: ColorRgba,
    pub status_idle: ColorRgba,
    pub status_warning: ColorRgba,
}

impl PaletteTokens {
    /// Warm neutral stone + signature crimson light palette for agent interfaces.
    #[must_use]
    pub const fn nla_light() -> Self {
        Self {
            background: ColorRgba::rgb(255, 255, 255),
            foreground: ColorRgba::rgb(12, 10, 9),
            card: ColorRgba::rgb(255, 255, 255),
            card_foreground: ColorRgba::rgb(12, 10, 9),
            popover: ColorRgba::rgb(255, 255, 255),
            popover_foreground: ColorRgba::rgb(12, 10, 9),
            primary: ColorRgba::rgb(193, 0, 7),
            primary_foreground: ColorRgba::rgb(254, 242, 242),
            secondary: ColorRgba::rgb(244, 244, 245),
            secondary_foreground: ColorRgba::rgb(24, 24, 27),
            muted: ColorRgba::rgb(245, 245, 244),
            muted_foreground: ColorRgba::rgb(120, 113, 108),
            accent: ColorRgba::rgb(245, 245, 244),
            accent_foreground: ColorRgba::rgb(28, 25, 23),
            destructive: ColorRgba::rgb(220, 38, 38),
            destructive_foreground: ColorRgba::rgb(254, 242, 242),
            border: ColorRgba::rgb(231, 229, 228),
            input: ColorRgba::rgb(231, 229, 228),
            ring: ColorRgba::rgb(168, 162, 158),
            sidebar: ColorRgba::rgb(250, 250, 249),
            sidebar_foreground: ColorRgba::rgb(12, 10, 9),
            sidebar_primary: ColorRgba::rgb(225, 29, 72),
            sidebar_primary_foreground: ColorRgba::rgb(254, 242, 242),
            sidebar_accent: ColorRgba::rgb(245, 245, 244),
            sidebar_accent_foreground: ColorRgba::rgb(28, 25, 23),
            sidebar_border: ColorRgba::rgb(231, 229, 228),
            sidebar_ring: ColorRgba::rgb(168, 162, 158),
            chart_1: ColorRgba::rgb(248, 113, 113),
            chart_2: ColorRgba::rgb(239, 68, 68),
            chart_3: ColorRgba::rgb(220, 38, 38),
            chart_4: ColorRgba::rgb(185, 28, 28),
            chart_5: ColorRgba::rgb(153, 27, 27),
            status_working: ColorRgba::rgb(37, 99, 235),
            status_blocked: ColorRgba::rgb(220, 38, 38),
            status_done: ColorRgba::rgb(22, 163, 74),
            status_idle: ColorRgba::rgb(120, 113, 108),
            status_warning: ColorRgba::rgb(217, 119, 6),
        }
    }

    /// Deep warm stone + crimson dark palette matching multi-device terminal & desktop clients.
    #[must_use]
    pub const fn nla_dark() -> Self {
        Self {
            background: ColorRgba::rgb(12, 10, 9),
            foreground: ColorRgba::rgb(250, 250, 249),
            card: ColorRgba::rgb(28, 25, 23),
            card_foreground: ColorRgba::rgb(250, 250, 249),
            popover: ColorRgba::rgb(28, 25, 23),
            popover_foreground: ColorRgba::rgb(250, 250, 249),
            primary: ColorRgba::rgb(185, 28, 28),
            primary_foreground: ColorRgba::rgb(254, 242, 242),
            secondary: ColorRgba::rgb(39, 39, 42),
            secondary_foreground: ColorRgba::rgb(250, 250, 250),
            muted: ColorRgba::rgb(41, 37, 36),
            muted_foreground: ColorRgba::rgb(168, 162, 158),
            accent: ColorRgba::rgb(68, 64, 60),
            accent_foreground: ColorRgba::rgb(250, 250, 249),
            destructive: ColorRgba::rgb(239, 68, 68),
            destructive_foreground: ColorRgba::rgb(254, 242, 242),
            border: ColorRgba::rgba(255, 255, 255, 26),
            input: ColorRgba::rgba(255, 255, 255, 38),
            ring: ColorRgba::rgb(120, 113, 108),
            sidebar: ColorRgba::rgb(28, 25, 23),
            sidebar_foreground: ColorRgba::rgb(250, 250, 249),
            sidebar_primary: ColorRgba::rgb(239, 68, 68),
            sidebar_primary_foreground: ColorRgba::rgb(254, 242, 242),
            sidebar_accent: ColorRgba::rgb(41, 37, 36),
            sidebar_accent_foreground: ColorRgba::rgb(250, 250, 249),
            sidebar_border: ColorRgba::rgba(255, 255, 255, 26),
            sidebar_ring: ColorRgba::rgb(120, 113, 108),
            chart_1: ColorRgba::rgb(252, 165, 165),
            chart_2: ColorRgba::rgb(248, 113, 113),
            chart_3: ColorRgba::rgb(239, 68, 68),
            chart_4: ColorRgba::rgb(220, 38, 38),
            chart_5: ColorRgba::rgb(185, 28, 28),
            status_working: ColorRgba::rgb(96, 165, 250),
            status_blocked: ColorRgba::rgb(248, 113, 113),
            status_done: ColorRgba::rgb(74, 222, 128),
            status_idle: ColorRgba::rgb(168, 162, 158),
            status_warning: ColorRgba::rgb(251, 191, 36),
        }
    }
}

/// Font families and modular type scale in points.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypographyTokens {
    pub font_sans: String,
    pub font_mono: String,
    pub xs_pt: f32,
    pub sm_pt: f32,
    pub base_pt: f32,
    pub lg_pt: f32,
    pub xl_pt: f32,
}

impl Default for TypographyTokens {
    fn default() -> Self {
        Self {
            font_sans: "Oxanium Variable, system-ui, sans-serif".to_owned(),
            font_mono: "Geist Mono Variable, ui-monospace, monospace".to_owned(),
            xs_pt: 11.0,
            sm_pt: 13.0,
            base_pt: 14.0,
            lg_pt: 16.0,
            xl_pt: 20.0,
        }
    }
}

/// Derived corner radii matching `--radius: 0.45rem` (7.2 pt at 16 pt root).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RadiusTokens {
    pub base_pt: f32,
}

impl Default for RadiusTokens {
    fn default() -> Self {
        Self { base_pt: 7.2 }
    }
}

impl RadiusTokens {
    #[must_use]
    pub fn sm(self) -> f32 {
        (self.base_pt - 4.0).max(2.0)
    }
    #[must_use]
    pub fn md(self) -> f32 {
        (self.base_pt - 2.0).max(4.0)
    }
    #[must_use]
    pub fn lg(self) -> f32 {
        self.base_pt
    }
    #[must_use]
    pub fn xl(self) -> f32 {
        self.base_pt + 4.0
    }
}

/// Consistent spacing scale in points.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SpacingScale {
    pub xxs: f32,
    pub xs: f32,
    pub sm: f32,
    pub md: f32,
    pub lg: f32,
    pub xl: f32,
}

impl Default for SpacingScale {
    fn default() -> Self {
        Self {
            xxs: 2.0,
            xs: 4.0,
            sm: 8.0,
            md: 12.0,
            lg: 16.0,
            xl: 24.0,
        }
    }
}

/// Top-level reusable theme bundle owning light/dark palettes, typography, radii, and spacing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Theme {
    pub name: String,
    pub appearance: Appearance,
    pub light: PaletteTokens,
    pub dark: PaletteTokens,
    pub typography: TypographyTokens,
    pub radius: RadiusTokens,
    pub spacing: SpacingScale,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            name: "NLA Crimson Stone".to_owned(),
            appearance: Appearance::Dark,
            light: PaletteTokens::nla_light(),
            dark: PaletteTokens::nla_dark(),
            typography: TypographyTokens::default(),
            radius: RadiusTokens::default(),
            spacing: SpacingScale::default(),
        }
    }
}

impl Theme {
    #[must_use]
    pub fn with_mode(mode: ThemeMode, system_dark: bool) -> Self {
        Self {
            appearance: mode.resolve(system_dark),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn palette(&self) -> &PaletteTokens {
        match self.appearance {
            Appearance::Light => &self.light,
            Appearance::Dark => &self.dark,
        }
    }

    /// Exports an NSH-compatible `cssVars` theme JSON document for `nsh.json` `"theme"`.
    pub fn to_nsh_css_vars_json(&self) -> Result<String, ThemeError> {
        let doc = serde_json::json!({
            "name": self.name,
            "cssVars": {
                "theme": {
                    "radius": format!("{}px", self.radius.base_pt),
                    "font-sans": self.typography.font_sans,
                    "font-mono": self.typography.font_mono,
                },
                "light": palette_css_map(&self.light),
                "dark": palette_css_map(&self.dark),
            }
        });
        Ok(serde_json::to_string_pretty(&doc)?)
    }

    /// Generates an atomic CSS custom-properties stylesheet supporting light & dark themes.
    #[must_use]
    pub fn to_css_stylesheet(&self) -> String {
        format!(
            ":root {{\n  --radius: {}px;\n  --font-sans: {};\n  --font-mono: {};\n{}\n}}\n:root[data-theme=\"dark\"], .dark {{\n{}\n}}\n",
            self.radius.base_pt,
            self.typography.font_sans,
            self.typography.font_mono,
            palette_css_vars_block(&self.light),
            palette_css_vars_block(&self.dark),
        )
    }
}

fn palette_css_vars_block(p: &PaletteTokens) -> String {
    let val = palette_css_map(p);
    let mut lines = Vec::new();
    if let Some(obj) = val.as_object() {
        for (k, v) in obj {
            if let Some(s) = v.as_str() {
                lines.push(format!("  --{k}: {s};"));
            }
        }
    }
    lines.extend([
        format!("  --status-working: {};", p.status_working.to_css()),
        format!("  --status-blocked: {};", p.status_blocked.to_css()),
        format!("  --status-done: {};", p.status_done.to_css()),
        format!("  --status-idle: {};", p.status_idle.to_css()),
        format!("  --status-warning: {};", p.status_warning.to_css()),
    ]);
    lines.join("\n")
}

fn palette_css_map(p: &PaletteTokens) -> serde_json::Value {
    serde_json::json!({
        "background": p.background.to_css(),
        "foreground": p.foreground.to_css(),
        "card": p.card.to_css(),
        "card-foreground": p.card_foreground.to_css(),
        "popover": p.popover.to_css(),
        "popover-foreground": p.popover_foreground.to_css(),
        "primary": p.primary.to_css(),
        "primary-foreground": p.primary_foreground.to_css(),
        "secondary": p.secondary.to_css(),
        "secondary-foreground": p.secondary_foreground.to_css(),
        "muted": p.muted.to_css(),
        "muted-foreground": p.muted_foreground.to_css(),
        "accent": p.accent.to_css(),
        "accent-foreground": p.accent_foreground.to_css(),
        "destructive": p.destructive.to_css(),
        "border": p.border.to_css(),
        "input": p.input.to_css(),
        "ring": p.ring.to_css(),
        "chart-1": p.chart_1.to_css(),
        "chart-2": p.chart_2.to_css(),
        "chart-3": p.chart_3.to_css(),
        "chart-4": p.chart_4.to_css(),
        "chart-5": p.chart_5.to_css(),
        "sidebar": p.sidebar.to_css(),
        "sidebar-foreground": p.sidebar_foreground.to_css(),
        "sidebar-primary": p.sidebar_primary.to_css(),
        "sidebar-primary-foreground": p.sidebar_primary_foreground.to_css(),
        "sidebar-accent": p.sidebar_accent.to_css(),
        "sidebar-accent-foreground": p.sidebar_accent_foreground.to_css(),
        "sidebar-border": p.sidebar_border.to_css(),
        "sidebar-ring": p.sidebar_ring.to_css(),
    })
}
