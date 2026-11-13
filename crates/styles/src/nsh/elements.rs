use crate::tokens::ColorRgba;
use serde::{Deserialize, Serialize};

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

/// Cross-platform UI element tree projected cleanly to native views, DOM HTML, or test trees.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NshNode {
    pub id: String,
    pub kind: String,
    pub role: AccessibilityRole,
    pub text: Option<String>,
    pub label: Option<String>,
    pub style: ViewStyle,
    pub children: Vec<NshNode>,
}

impl NshNode {
    #[must_use]
    pub fn view(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            kind: "View".to_owned(),
            role: AccessibilityRole::Region,
            text: None,
            label: None,
            style: ViewStyle::default(),
            children: Vec::new(),
        }
    }

    #[must_use]
    pub fn text(id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            kind: "Text".to_owned(),
            role: AccessibilityRole::Status,
            text: Some(content.into()),
            label: None,
            style: ViewStyle::default(),
            children: Vec::new(),
        }
    }

    #[must_use]
    pub fn button(id: impl Into<String>, label: impl Into<String>) -> Self {
        let text_val = label.into();
        Self {
            id: id.into(),
            kind: "Button".to_owned(),
            role: AccessibilityRole::Button,
            text: Some(text_val.clone()),
            label: Some(text_val),
            style: ViewStyle::default(),
            children: Vec::new(),
        }
    }

    #[must_use]
    pub fn badge(id: impl Into<String>, label: impl Into<String>) -> Self {
        let text_val = label.into();
        Self {
            id: id.into(),
            kind: "Badge".to_owned(),
            role: AccessibilityRole::Badge,
            text: Some(text_val.clone()),
            label: Some(text_val),
            style: ViewStyle::default(),
            children: Vec::new(),
        }
    }

    #[must_use]
    pub fn web_preview(
        id: impl Into<String>,
        url: impl Into<String>,
        title: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            kind: "WebView".to_owned(),
            role: AccessibilityRole::WebView,
            text: Some(url.into()),
            label: Some(title.into()),
            style: ViewStyle::default(),
            children: Vec::new(),
        }
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
        for child in &self.children {
            out.extend(child.flatten_text_lines());
        }
        out
    }

    /// Projects this NSH node tree into an escaped HTML fragment for the Web backend.
    #[must_use]
    pub fn render_html_fragment(&self) -> String {
        let dir = match self.style.direction {
            FlexDirection::Column => "column",
            FlexDirection::Row => "row",
        };
        let mut css = format!(
            "display:flex;flex-direction:{dir};gap:{}px;padding:{}px",
            self.style.gap_pt, self.style.padding_pt
        );
        if let Some(bg) = self.style.background {
            css.push_str(&format!(";background:{}", bg.to_css()));
        }
        if let Some(fg) = self.style.foreground {
            css.push_str(&format!(";color:{}", fg.to_css()));
        }
        if let Some(border) = self.style.border {
            css.push_str(&format!(";border:1px solid {}", border.to_css()));
        }
        if let Some(radius) = self.style.radius_pt {
            css.push_str(&format!(";border-radius:{radius}px"));
        }

        let id = escape_html(&self.id);
        let role = format!("{:?}", self.role).to_ascii_lowercase();
        let mut inner = String::new();
        if let Some(txt) = &self.text {
            inner.push_str(&escape_html(txt));
        }
        for child in &self.children {
            inner.push_str(&child.render_html_fragment());
        }
        format!(
            "<div data-nsh-kind=\"{}\" id=\"{id}\" role=\"{role}\" style=\"{css}\">{inner}</div>",
            escape_html(&self.kind)
        )
    }
}

fn escape_html(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
