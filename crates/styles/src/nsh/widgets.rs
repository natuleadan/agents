use crate::components::{
    BreadcrumbSpec, ChartSeriesSpec, CommandPaletteSpec, Density, DialogSpec, EmptyStateSpec,
    InputSpec, InteractionState, TableSpec,
};
use crate::tokens::Theme;

use super::elements::{AccessibilityRole, FlexDirection, NshNode, escape_html};

impl NshNode {
    /// Constructs a labeled form input mapped to [`AccessibilityRole::Textbox`].
    #[must_use]
    pub fn input(id: impl Into<String>, spec: &InputSpec, theme: &Theme, density: Density) -> Self {
        let style = spec.resolve(theme, density);
        let mut node = Self::view(id);
        node.kind = "Input".to_owned();
        node.role = AccessibilityRole::Textbox;
        node.text = Some(spec.value.clone());
        node.label = Some(spec.placeholder.clone());
        node.placeholder = Some(spec.placeholder.clone());
        node.input_kind = Some(if spec.masked { "password" } else { "text" }.to_owned());
        node = node
            .bg(style.background)
            .fg(style.foreground)
            .border(style.border)
            .radius(style.radius_pt)
            .padding(style.padding_y_pt);
        if spec.state == InteractionState::Invalid {
            node.label = Some(format!("{} (invalid)", spec.placeholder));
        }
        node
    }

    /// Constructs a semantic table node from a [`TableSpec`].
    #[must_use]
    pub fn table(id: impl Into<String>, spec: &TableSpec, theme: &Theme) -> Self {
        let p = theme.palette();
        let mut node = Self::view(id);
        node.kind = "Table".to_owned();
        node.role = AccessibilityRole::Table;
        node.label = spec.caption.clone();
        node.columns = spec.columns.iter().map(|c| c.heading.clone()).collect();
        node.rows = spec.rows.clone();
        node.bg(p.card)
            .fg(p.card_foreground)
            .border(p.border)
            .radius(theme.radius.lg())
            .padding(theme.spacing.sm)
    }

    /// Constructs a multi-series vector chart node mapped to [`AccessibilityRole::Chart`].
    #[must_use]
    pub fn chart(
        id: impl Into<String>,
        title: impl Into<String>,
        series: Vec<ChartSeriesSpec>,
        theme: &Theme,
    ) -> Self {
        let p = theme.palette();
        let palette = [p.chart_1, p.chart_2, p.chart_3, p.chart_4, p.chart_5];
        let colors = series
            .iter()
            .map(|s| palette[s.palette_index % palette.len()])
            .collect();
        let mut node = Self::view(id);
        node.kind = "Chart".to_owned();
        node.role = AccessibilityRole::Chart;
        node.label = Some(title.into());
        node.series = series;
        node.series_colors = colors;
        node.bg(p.card)
            .fg(p.card_foreground)
            .border(p.border)
            .radius(theme.radius.lg())
            .padding(theme.spacing.md)
    }

    /// Constructs a modal dialog node from a [`DialogSpec`].
    #[must_use]
    pub fn dialog(id: impl Into<String>, spec: &DialogSpec, theme: &Theme) -> Self {
        let p = theme.palette();
        let id_str = id.into();
        let mut dlg = Self::view(id_str.clone())
            .role(AccessibilityRole::Dialog)
            .direction(FlexDirection::Column)
            .gap(theme.spacing.sm)
            .padding(theme.spacing.md)
            .bg(p.popover)
            .fg(p.popover_foreground)
            .border(if spec.destructive {
                p.destructive
            } else {
                p.border
            })
            .radius(theme.radius.xl());
        dlg.kind = "Dialog".to_owned();
        dlg.label = Some(spec.title.clone());
        dlg = dlg.child(Self::text(format!("{id_str}-title"), &spec.title));
        if let Some(desc) = &spec.description {
            dlg = dlg.child(Self::text(format!("{id_str}-desc"), desc).fg(p.muted_foreground));
        }
        dlg
    }

    /// Constructs a breadcrumb trail from a [`BreadcrumbSpec`].
    #[must_use]
    pub fn breadcrumb(id: impl Into<String>, spec: &BreadcrumbSpec, theme: &Theme) -> Self {
        let p = theme.palette();
        let id_str = id.into();
        let mut nav = Self::view(id_str.clone())
            .role(AccessibilityRole::Navigation)
            .direction(FlexDirection::Row)
            .gap(theme.spacing.xs);
        nav.kind = "Breadcrumb".to_owned();
        for (idx, item) in spec.items.iter().enumerate() {
            let fg = if item.current {
                p.foreground
            } else {
                p.muted_foreground
            };
            nav = nav.child(Self::text(format!("{id_str}-item-{idx}"), &item.label).fg(fg));
        }
        nav
    }

    /// Constructs an empty-state callout from an [`EmptyStateSpec`].
    #[must_use]
    pub fn empty_state(id: impl Into<String>, spec: &EmptyStateSpec, theme: &Theme) -> Self {
        let p = theme.palette();
        let id_str = id.into();
        let mut box_node = Self::view(id_str.clone())
            .direction(FlexDirection::Column)
            .gap(theme.spacing.xs)
            .padding(theme.spacing.md)
            .bg(p.muted)
            .fg(p.muted_foreground)
            .border(p.border)
            .radius(theme.radius.lg())
            .child(Self::text(format!("{id_str}-title"), &spec.title).fg(p.foreground))
            .child(Self::text(format!("{id_str}-desc"), &spec.description));
        if let Some(action) = &spec.action_label {
            box_node = box_node.child(Self::button(format!("{id_str}-cta"), action));
        }
        box_node
    }

    /// Constructs a searchable command palette node from a [`CommandPaletteSpec`].
    #[must_use]
    pub fn command_palette(
        id: impl Into<String>,
        spec: &CommandPaletteSpec,
        theme: &Theme,
    ) -> Self {
        let p = theme.palette();
        let id_str = id.into();
        let mut palette = Self::view(id_str.clone())
            .role(AccessibilityRole::Dialog)
            .direction(FlexDirection::Column)
            .gap(theme.spacing.xs)
            .padding(theme.spacing.sm)
            .bg(p.popover)
            .fg(p.popover_foreground)
            .border(p.border)
            .radius(theme.radius.lg());
        palette.kind = "CommandPalette".to_owned();
        let query_lower = spec.query.to_ascii_lowercase();
        let matches: Vec<_> = spec
            .items
            .iter()
            .filter(|i| {
                query_lower.is_empty()
                    || i.label.to_ascii_lowercase().contains(&query_lower)
                    || i.group.to_ascii_lowercase().contains(&query_lower)
            })
            .collect();
        if matches.is_empty() {
            palette = palette.child(Self::text(format!("{id_str}-empty"), &spec.empty_message));
        } else {
            for item in matches {
                let text = match &item.shortcut {
                    Some(sc) => format!("[{}] {} ({sc})", item.group, item.label),
                    None => format!("[{}] {}", item.group, item.label),
                };
                palette = palette.child(Self::button(format!("{id_str}-item-{}", item.id), text));
            }
        }
        palette
    }
}

pub(crate) fn project_node_html(node: &NshNode) -> String {
    let css = build_css_inline(node);
    let id = escape_html(&node.id);
    let tid = node
        .test_id
        .as_deref()
        .map(|v| format!(" data-testid=\"{}\"", escape_html(v)))
        .unwrap_or_default();
    let role = format!("{:?}", node.role).to_ascii_lowercase();
    let kind = escape_html(&node.kind);

    match node.kind.as_str() {
        "Input" => {
            let ty = node.input_kind.as_deref().unwrap_or("text");
            let val = escape_html(node.text.as_deref().unwrap_or(""));
            let ph = escape_html(node.placeholder.as_deref().unwrap_or(""));
            format!(
                "<input data-nsh-kind=\"{kind}\" id=\"{id}\"{tid} type=\"{ty}\" value=\"{val}\" placeholder=\"{ph}\" aria-label=\"{ph}\" style=\"{css}\" />"
            )
        }
        "Button" => {
            let txt = escape_html(node.text.as_deref().unwrap_or(""));
            format!(
                "<button data-nsh-kind=\"{kind}\" id=\"{id}\"{tid} type=\"button\" style=\"{css}\">{txt}</button>"
            )
        }
        "Table" => render_table_html(node, &id, &tid, &css),
        "Chart" => render_chart_html(node, &id, &tid, &css),
        "WebView" => {
            let url = escape_html(node.text.as_deref().unwrap_or("about:blank"));
            let title = escape_html(node.label.as_deref().unwrap_or("Preview"));
            format!(
                "<iframe data-nsh-kind=\"{kind}\" id=\"{id}\"{tid} title=\"{title}\" src=\"{url}\" sandbox=\"allow-scripts allow-forms\" style=\"{css}\"></iframe>"
            )
        }
        _ => {
            let mut inner = String::new();
            if let Some(txt) = &node.text {
                inner.push_str(&escape_html(txt));
            }
            for child in &node.children {
                inner.push_str(&project_node_html(child));
            }
            format!(
                "<div data-nsh-kind=\"{kind}\" id=\"{id}\"{tid} role=\"{role}\" style=\"{css}\">{inner}</div>"
            )
        }
    }
}

fn build_css_inline(node: &NshNode) -> String {
    let dir = match node.style.direction {
        FlexDirection::Column => "column",
        FlexDirection::Row => "row",
    };
    let mut css = format!(
        "display:flex;flex-direction:{dir};gap:{}px;padding:{}px",
        node.style.gap_pt, node.style.padding_pt
    );
    if let Some(bg) = node.style.background {
        css.push_str(&format!(";background:{}", bg.to_css()));
    }
    if let Some(fg) = node.style.foreground {
        css.push_str(&format!(";color:{}", fg.to_css()));
    }
    if let Some(border) = node.style.border {
        css.push_str(&format!(";border:1px solid {}", border.to_css()));
    }
    if let Some(radius) = node.style.radius_pt {
        css.push_str(&format!(";border-radius:{radius}px"));
    }
    css
}

fn render_table_html(node: &NshNode, id: &str, tid: &str, css: &str) -> String {
    let mut out = format!(
        "<div data-nsh-kind=\"Table\" id=\"{id}\"{tid} style=\"{css}\"><table style=\"width:100%;border-collapse:collapse\"><thead><tr>"
    );
    for col in &node.columns {
        out.push_str(&format!("<th>{}</th>", escape_html(col)));
    }
    out.push_str("</tr></thead><tbody>");
    for row in &node.rows {
        out.push_str("<tr>");
        for cell in row {
            out.push_str(&format!("<td>{}</td>", escape_html(cell)));
        }
        out.push_str("</tr>");
    }
    out.push_str("</tbody></table></div>");
    out
}

fn render_chart_html(node: &NshNode, id: &str, tid: &str, css: &str) -> String {
    let title = escape_html(node.label.as_deref().unwrap_or("Metrics"));
    let mut svg =
        String::from("<svg viewBox=\"0 0 320 96\" width=\"100%\" height=\"96\" role=\"img\">");
    for (idx, series) in node.series.iter().enumerate() {
        let color = node
            .series_colors
            .get(idx)
            .map(|c| c.to_css())
            .unwrap_or_else(|| "#ef4444".to_owned());
        let max_val = series
            .points
            .iter()
            .copied()
            .fold(1.0_f64, |acc, v| acc.max(v));
        let count = series.points.len().max(2);
        let pts: Vec<String> = series
            .points
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let x = (i as f64 / (count - 1) as f64) * 300.0 + 10.0;
                let y = 84.0 - ((v / max_val) * 68.0);
                format!("{x:.1},{y:.1}")
            })
            .collect();
        svg.push_str(&format!(
            "<polyline fill=\"none\" stroke=\"{color}\" stroke-width=\"2\" points=\"{}\" />",
            pts.join(" ")
        ));
    }
    svg.push_str("</svg>");
    format!(
        "<div data-nsh-kind=\"Chart\" id=\"{id}\"{tid} role=\"img\" aria-label=\"{title}\" style=\"{css}\"><span>{title}</span>{svg}</div>"
    )
}
