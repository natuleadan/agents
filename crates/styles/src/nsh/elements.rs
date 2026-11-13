use crate::components::ChartSeriesSpec;
use crate::tokens::ColorRgba;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlexDirection {
    #[default]
    Column,
    Row,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlignItems {
    #[default]
    Stretch,
    Start,
    Center,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JustifyContent {
    #[default]
    Start,
    Center,
    End,
    SpaceBetween,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessibilityRole {
    #[default]
    Region,
    Button,
    Textbox,
    Navigation,
    Dialog,
    Sheet,
    Status,
    Badge,
    Heading,
    WebView,
    Table,
    Chart,
    Alert,
}

/// Typed layout and surface styles for declarative NSH view nodes.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ViewStyle {
    pub direction: FlexDirection,
    pub align_items: AlignItems,
    pub justify_content: JustifyContent,
    pub gap_pt: f32,
    pub padding_pt: f32,
    pub background: Option<ColorRgba>,
    pub foreground: Option<ColorRgba>,
    pub border: Option<ColorRgba>,
    pub radius_pt: Option<f32>,
    pub flex_grow: f32,
}

/// Deterministic mount summary inspired by NSH `--trace` verification output.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct NshMountTrace {
    pub total_nodes: usize,
    pub interactive_controls: usize,
    pub table_rows: usize,
    pub chart_series: usize,
    pub test_ids: Vec<String>,
    pub duplicate_test_ids: Vec<String>,
}

/// Cross-platform UI element tree projected cleanly to native views, DOM HTML, or test trees.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NshNode {
    pub id: String,
    pub test_id: Option<String>,
    pub kind: String,
    pub role: AccessibilityRole,
    pub text: Option<String>,
    pub label: Option<String>,
    pub placeholder: Option<String>,
    pub input_kind: Option<String>,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub series: Vec<ChartSeriesSpec>,
    pub series_colors: Vec<ColorRgba>,
    pub style: ViewStyle,
    pub children: Vec<NshNode>,
}

impl NshNode {
    #[must_use]
    pub fn view(id: impl Into<String>) -> Self {
        let id_str = id.into();
        Self {
            test_id: Some(id_str.clone()),
            id: id_str,
            kind: "View".to_owned(),
            role: AccessibilityRole::Region,
            text: None,
            label: None,
            placeholder: None,
            input_kind: None,
            columns: Vec::new(),
            rows: Vec::new(),
            series: Vec::new(),
            series_colors: Vec::new(),
            style: ViewStyle::default(),
            children: Vec::new(),
        }
    }

    #[must_use]
    pub fn text(id: impl Into<String>, content: impl Into<String>) -> Self {
        let mut node = Self::view(id);
        node.kind = "Text".to_owned();
        node.role = AccessibilityRole::Status;
        node.text = Some(content.into());
        node
    }

    #[must_use]
    pub fn button(id: impl Into<String>, label: impl Into<String>) -> Self {
        let text_val = label.into();
        let mut node = Self::view(id);
        node.kind = "Button".to_owned();
        node.role = AccessibilityRole::Button;
        node.text = Some(text_val.clone());
        node.label = Some(text_val);
        node
    }

    #[must_use]
    pub fn badge(id: impl Into<String>, label: impl Into<String>) -> Self {
        let text_val = label.into();
        let mut node = Self::view(id);
        node.kind = "Badge".to_owned();
        node.role = AccessibilityRole::Badge;
        node.text = Some(text_val.clone());
        node.label = Some(text_val);
        node
    }

    #[must_use]
    pub fn web_preview(
        id: impl Into<String>,
        url: impl Into<String>,
        title: impl Into<String>,
    ) -> Self {
        let mut node = Self::view(id);
        node.kind = "WebView".to_owned();
        node.role = AccessibilityRole::WebView;
        node.text = Some(url.into());
        node.label = Some(title.into());
        node
    }

    #[must_use]
    pub fn test_id(mut self, test_id: impl Into<String>) -> Self {
        self.test_id = Some(test_id.into());
        self
    }

    #[must_use]
    pub fn role(mut self, role: AccessibilityRole) -> Self {
        self.role = role;
        self
    }

    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    #[must_use]
    pub fn direction(mut self, direction: FlexDirection) -> Self {
        self.style.direction = direction;
        self
    }

    #[must_use]
    pub fn align_items(mut self, align: AlignItems) -> Self {
        self.style.align_items = align;
        self
    }

    #[must_use]
    pub fn justify_content(mut self, justify: JustifyContent) -> Self {
        self.style.justify_content = justify;
        self
    }

    #[must_use]
    pub fn gap(mut self, gap_pt: f32) -> Self {
        self.style.gap_pt = gap_pt;
        self
    }

    #[must_use]
    pub fn padding(mut self, padding_pt: f32) -> Self {
        self.style.padding_pt = padding_pt;
        self
    }

    #[must_use]
    pub fn bg(mut self, color: ColorRgba) -> Self {
        self.style.background = Some(color);
        self
    }

    #[must_use]
    pub fn fg(mut self, color: ColorRgba) -> Self {
        self.style.foreground = Some(color);
        self
    }

    #[must_use]
    pub fn border(mut self, color: ColorRgba) -> Self {
        self.style.border = Some(color);
        self
    }

    #[must_use]
    pub fn radius(mut self, radius_pt: f32) -> Self {
        self.style.radius_pt = Some(radius_pt);
        self
    }

    #[must_use]
    pub fn child(mut self, node: NshNode) -> Self {
        self.children.push(node);
        self
    }

    #[must_use]
    pub fn children(mut self, nodes: impl IntoIterator<Item = NshNode>) -> Self {
        self.children.extend(nodes);
        self
    }

    /// Collects all visible text lines in pre-order for headless NSH assertion testing.
    #[must_use]
    pub fn flatten_text_lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(t) = &self.text {
            out.push(t.clone());
        }
        for row in &self.rows {
            out.push(row.join(" | "));
        }
        for child in &self.children {
            out.extend(child.flatten_text_lines());
        }
        out
    }

    /// Collects a deterministic mount trace verifying `testID` uniqueness and mount size.
    #[must_use]
    pub fn collect_trace(&self) -> NshMountTrace {
        let mut trace = NshMountTrace::default();
        let mut seen = BTreeSet::new();
        self.walk_trace(&mut trace, &mut seen);
        trace
    }

    fn walk_trace(&self, trace: &mut NshMountTrace, seen: &mut BTreeSet<String>) {
        trace.total_nodes += 1;
        if matches!(
            self.role,
            AccessibilityRole::Button | AccessibilityRole::Textbox
        ) {
            trace.interactive_controls += 1;
        }
        trace.table_rows += self.rows.len();
        trace.chart_series += self.series.len();
        if let Some(tid) = &self.test_id {
            if !seen.insert(tid.clone()) {
                trace.duplicate_test_ids.push(tid.clone());
            }
            trace.test_ids.push(tid.clone());
        }
        for child in &self.children {
            child.walk_trace(trace, seen);
        }
    }

    /// Projects this NSH node tree into an escaped HTML fragment for the Web backend.
    #[must_use]
    pub fn render_html_fragment(&self) -> String {
        super::widgets::project_node_html(self)
    }
}

pub(crate) fn escape_html(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
