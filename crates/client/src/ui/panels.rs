use agents_styles::prelude::*;

use crate::core::state::{ActiveView, AppState};

pub fn build_header_bar(state: &AppState, theme: &Theme, compact: bool) -> NshNode {
    let p = theme.palette();
    let mode_text = format!("{:?}", state.interaction_mode);
    let badge_style =
        BadgeSpec::new(&state.effective_profile.label, BadgeVariant::Outline).resolve(theme);

    let mut header = NshNode::view("header-bar")
        .direction(if compact {
            FlexDirection::Column
        } else {
            FlexDirection::Row
        })
        .gap(theme.spacing.sm)
        .padding(theme.spacing.md)
        .bg(p.card)
        .border(p.border)
        .child(NshNode::text("brand", "agents").fg(p.foreground))
        .child(
            NshNode::badge("effective-profile-badge", &state.effective_profile.label)
                .bg(badge_style.background)
                .fg(badge_style.foreground)
                .border(badge_style.border)
                .radius(badge_style.radius_pt),
        )
        .child(NshNode::badge("mode-badge", format!("mode: {mode_text}")));

    if state.notifications.attention_needed {
        header = header.child(
            NshNode::badge("attention-cue", "Attention Required")
                .bg(p.status_warning)
                .fg(p.background),
        );
    }
    if state.notifications.completion_chimes > 0 {
        header = header.child(
            NshNode::text(
                "completion-chime",
                format!("Chime x{}", state.notifications.completion_chimes),
            )
            .fg(p.status_done),
        );
    }
    header
}

pub fn build_navigation(state: &AppState, metrics: SurfaceMetrics, theme: &Theme) -> NshNode {
    let p = theme.palette();
    let items = [
        ("Workspace", ActiveView::Workspace),
        ("Swarm", ActiveView::Swarm),
        ("Preview", ActiveView::Preview),
        ("Updates", ActiveView::UpdateModal),
    ];
    let nav_role = if metrics.prefers_bottom_sheet() {
        AccessibilityRole::Sheet
    } else {
        AccessibilityRole::Navigation
    };
    let mut container = NshNode::view("nav-region")
        .role(nav_role)
        .direction(if metrics.prefers_bottom_sheet() {
            FlexDirection::Row
        } else {
            FlexDirection::Column
        })
        .gap(theme.spacing.xs)
        .padding(theme.spacing.sm)
        .bg(p.sidebar)
        .border(p.sidebar_border);

    for (label, view) in items {
        let selected = state.active_view == view;
        let btn_spec = ButtonSpec::new(label)
            .variant(if selected {
                ButtonVariant::Default
            } else {
                ButtonVariant::Ghost
            })
            .resolve(theme, metrics.density());
        container = container.child(
            NshNode::button(format!("nav-{}", label.to_ascii_lowercase()), label)
                .bg(btn_spec.background)
                .fg(btn_spec.foreground)
                .radius(btn_spec.radius_pt),
        );
    }
    container
}

pub fn build_workspace_panel(state: &AppState, theme: &Theme) -> NshNode {
    let p = theme.palette();
    let mut panel = NshNode::view("workspace-panel")
        .direction(FlexDirection::Column)
        .gap(theme.spacing.md)
        .padding(theme.spacing.md)
        .bg(p.background)
        .fg(p.foreground);

    let mut worker_strip = NshNode::view("workers-strip")
        .direction(FlexDirection::Row)
        .gap(theme.spacing.xs);
    for w in &state.workers {
        worker_strip = worker_strip.child(NshNode::badge(
            format!("worker-{}", w.id),
            format!("{} ({:?})", w.role, w.status),
        ));
    }
    panel = panel.child(worker_strip);

    for step in &state.thinking_steps {
        let label = if step.collapsed {
            format!("▸ {} (collapsed)", step.title)
        } else {
            format!("▾ {}: {}", step.title, step.detail)
        };
        panel = panel.child(
            NshNode::view(format!("thinking-{}", step.id))
                .padding(theme.spacing.sm)
                .bg(p.muted)
                .fg(p.muted_foreground)
                .radius(theme.radius.md())
                .child(NshNode::text(format!("thinking-text-{}", step.id), label)),
        );
    }

    if !state.stream_output.is_empty() {
        panel = panel.child(NshNode::text("stream-output", &state.stream_output));
    }

    if !state.attachments.is_empty() {
        let mut att_row = NshNode::view("attachments-row")
            .direction(FlexDirection::Row)
            .gap(theme.spacing.xs);
        for att in &state.attachments {
            att_row = att_row.child(NshNode::badge(
                format!("att-{}", att.name),
                format!("{} ({} B)", att.name, att.bytes),
            ));
        }
        panel = panel.child(att_row);
    }

    if !state.subagent_messages.is_empty() {
        let mut drawer = NshNode::view("subagent-drawer")
            .direction(FlexDirection::Column)
            .gap(theme.spacing.xs)
            .padding(theme.spacing.sm)
            .bg(p.card)
            .border(p.border);
        for msg in &state.subagent_messages {
            drawer = drawer.child(NshNode::text(
                format!("submsg-{}", msg.id),
                format!("[{}] {}", msg.from_role, msg.summary),
            ));
        }
        panel = panel.child(drawer);
    }

    if !state.swarm_forks.is_empty() {
        let mut swarm = NshNode::view("swarm-forks")
            .direction(FlexDirection::Column)
            .gap(theme.spacing.xs)
            .padding(theme.spacing.sm)
            .bg(p.card)
            .border(p.border);
        for fork in &state.swarm_forks {
            swarm = swarm.child(NshNode::text(
                format!("fork-{}", fork.id),
                format!(
                    "{} <- {} ({:?}, {} files)",
                    fork.branch, fork.parent_branch, fork.status, fork.changed_files
                ),
            ));
        }
        panel = panel.child(swarm);
    }

    panel
}

pub fn build_preview_panel(state: &AppState, theme: &Theme) -> NshNode {
    let p = theme.palette();
    let mut card = NshNode::view("preview-panel")
        .direction(FlexDirection::Column)
        .gap(theme.spacing.sm)
        .padding(theme.spacing.md)
        .bg(p.card)
        .border(p.border)
        .radius(theme.radius.lg());

    match &state.preview_route {
        Some(route) => {
            card = card
                .child(NshNode::text(
                    "preview-heading",
                    format!("Preview: {}", route.title),
                ))
                .child(NshNode::web_preview(
                    "traefik-webview",
                    &route.url,
                    &route.title,
                ));
        }
        None => {
            card = card.child(NshNode::text(
                "preview-empty",
                "No active Traefik preview route",
            ));
        }
    }

    if let Some(avail) = &state.update_info.available_version {
        card = card.child(
            NshNode::view("update-banner")
                .role(AccessibilityRole::Dialog)
                .padding(theme.spacing.sm)
                .bg(p.secondary)
                .child(NshNode::text(
                    "update-text",
                    format!(
                        "Update available: {} -> {} ({})",
                        state.update_info.current_version, avail, state.update_info.notes
                    ),
                )),
        );
    }

    card
}
