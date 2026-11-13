use crate::components::Density;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

/// Supported compilation & presentation targets matching the workspace CI matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetPlatform {
    Web,
    Ios,
    Android,
    MacOs,
    Windows,
    #[default]
    Linux,
    Tui,
}

impl TargetPlatform {
    /// Parses CLI/manifest target names (`web`, `ios`, `android`, `macos`, `windows`, `linux`, `tui`).
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "web" | "wasm" | "web-wasm" => Some(Self::Web),
            "ios" => Some(Self::Ios),
            "android" => Some(Self::Android),
            "macos" | "darwin" => Some(Self::MacOs),
            "windows" | "win32" => Some(Self::Windows),
            "linux" | "gtk4" => Some(Self::Linux),
            "tui" | "terminal" => Some(Self::Tui),
            _ => None,
        }
    }

    #[must_use]
    pub const fn idiom(self) -> Idiom {
        match self {
            Self::Web => Idiom::Web,
            Self::Ios | Self::Android => Idiom::Mobile,
            Self::MacOs | Self::Windows | Self::Linux => Idiom::Desktop,
            Self::Tui => Idiom::Terminal,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Web => "web",
            Self::Ios => "ios",
            Self::Android => "android",
            Self::MacOs => "macos",
            Self::Windows => "windows",
            Self::Linux => "linux",
            Self::Tui => "tui",
        }
    }
}

/// Platform interaction family per NSH design guides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Idiom {
    #[default]
    Desktop,
    Mobile,
    Web,
    Terminal,
}

/// Breakpoint class used to reshape sidebars, sheets, and split preview panes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponsiveClass {
    Compact,
    Medium,
    #[default]
    Expanded,
}

/// Viewport and safe-area geometry supplied by the native or terminal shell.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SurfaceMetrics {
    pub target: TargetPlatform,
    pub width_pt: f32,
    pub height_pt: f32,
    pub safe_top_pt: f32,
    pub safe_bottom_pt: f32,
}

impl SurfaceMetrics {
    #[must_use]
    pub fn desktop(target: TargetPlatform) -> Self {
        Self {
            target,
            width_pt: 1280.0,
            height_pt: 800.0,
            safe_top_pt: 0.0,
            safe_bottom_pt: 0.0,
        }
    }

    #[must_use]
    pub fn mobile(target: TargetPlatform) -> Self {
        Self {
            target,
            width_pt: 390.0,
            height_pt: 844.0,
            safe_top_pt: 47.0,
            safe_bottom_pt: 34.0,
        }
    }

    #[must_use]
    pub fn web(width_pt: f32, height_pt: f32) -> Self {
        Self {
            target: TargetPlatform::Web,
            width_pt,
            height_pt,
            safe_top_pt: 0.0,
            safe_bottom_pt: 0.0,
        }
    }

    #[must_use]
    pub fn responsive_class(self) -> ResponsiveClass {
        if self.target.idiom() == Idiom::Mobile || self.width_pt < 640.0 {
            ResponsiveClass::Compact
        } else if self.width_pt < 1024.0 {
            ResponsiveClass::Medium
        } else {
            ResponsiveClass::Expanded
        }
    }

    #[must_use]
    pub fn density(self) -> Density {
        match self.target.idiom() {
            Idiom::Mobile => Density::Touch,
            Idiom::Terminal => Density::Compact,
            Idiom::Desktop | Idiom::Web => Density::Comfortable,
        }
    }

    /// True when secondary pickers/menus open as bottom sheets instead of anchored popovers.
    #[must_use]
    pub fn prefers_bottom_sheet(self) -> bool {
        self.responsive_class() == ResponsiveClass::Compact
    }
}

/// Fine-grained reactive signal inspired by NSH's ownership-safe signal graph.
#[derive(Clone, Debug)]
pub struct Signal<T> {
    inner: Arc<Mutex<T>>,
}

impl<T: Clone> Signal<T> {
    #[must_use]
    pub fn new(initial: T) -> Self {
        Self {
            inner: Arc::new(Mutex::new(initial)),
        }
    }

    #[must_use]
    pub fn get(&self) -> T {
        match self.inner.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    pub fn set(&self, value: T) {
        let mut guard = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        *guard = value;
    }

    pub fn update(&self, f: impl FnOnce(&mut T)) {
        let mut guard = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        f(&mut guard);
    }
}
