#![forbid(unsafe_code)]
//! Multi-platform client workspace (`core`, `tui`, `ui`) built on [`agents_styles`] and [`agents_protocol`].

pub mod core;
pub mod tui;
pub mod ui;

pub use agents_styles as styles;
