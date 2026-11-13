use agents_styles::prelude::*;

use crate::core::state::AppState;

/// Builds the private-VLAN Boxer Cluster table panel and SVG telemetry charts.
#[must_use]
pub fn build_cluster_and_metrics_panel(state: &AppState, theme: &Theme) -> NshNode {
    let p = theme.palette();
    let mut container = NshNode::view("cluster-metrics-panel")
        .direction(FlexDirection::Column)
        .gap(theme.spacing.md)
        .padding(theme.spacing.md)
        .bg(p.background)
        .fg(p.foreground);

    container = container.child(NshNode::breadcrumb(
        "workspace-breadcrumb",
        &BreadcrumbSpec {
            items: vec![
                BreadcrumbItem {
                    label: "Cluster".to_owned(),
                    current: false,
                },
                BreadcrumbItem {
                    label: format!("Mode: {:?}", state.login.mode),
                    current: true,
                },
            ],
        },
        theme,
    ));

    if state.cluster_nodes.is_empty() {
        container = container.child(NshNode::empty_state(
            "cluster-nodes-empty",
            &EmptyStateSpec {
                title: "No private-VLAN Boxer workers registered".to_owned(),
                description:
                    "Start agents-boxer on any 10.x.x.x node to auto-register via heartbeat."
                        .to_owned(),
                action_label: Some("Refresh Nodes".to_owned()),
            },
            theme,
        ));
    } else {
        let rows = state
            .cluster_nodes
            .iter()
            .map(|n| {
                vec![
                    n.node_id.clone(),
                    n.vlan_ip.clone(),
                    n.runtime_label.clone(),
                    format!("{}/{}", n.active_boxes, n.max_boxes),
                    n.status_label.clone(),
                ]
            })
            .collect();
        let table = TableSpec {
            caption: Some("Auto-Registered Private-VLAN Workers".to_owned()),
            columns: vec![
                TableColumnSpec {
                    key: "node_id".to_owned(),
                    heading: "Node".to_owned(),
                },
                TableColumnSpec {
                    key: "vlan_ip".to_owned(),
                    heading: "VLAN IP".to_owned(),
                },
                TableColumnSpec {
                    key: "runtime".to_owned(),
                    heading: "Runtime".to_owned(),
                },
                TableColumnSpec {
                    key: "load".to_owned(),
                    heading: "Boxes".to_owned(),
                },
                TableColumnSpec {
                    key: "status".to_owned(),
                    heading: "Status".to_owned(),
                },
            ],
            rows,
        };
        container = container.child(NshNode::table("cluster-nodes-table", &table, theme));
    }

    let chart = NshNode::chart(
        "telemetry-latency-chart",
        "Stream Latency (ms) & Context Tokens",
        vec![
            ChartSeriesSpec {
                id: "latency".to_owned(),
                label: "Latency (ms)".to_owned(),
                points: state.latency_samples_ms.clone(),
                palette_index: 0,
            },
            ChartSeriesSpec {
                id: "tokens".to_owned(),
                label: "Tokens (k)".to_owned(),
                points: state.token_samples.iter().map(|t| t / 100.0).collect(),
                palette_index: 2,
            },
        ],
        theme,
    );
    container = container.child(chart);
    container
}

/// Builds the searchable Command Palette and Update Modal dialog views.
#[must_use]
pub fn build_command_and_update_dialogs(state: &AppState, theme: &Theme) -> NshNode {
    let items = state
        .slash_commands
        .iter()
        .enumerate()
        .map(|(idx, cmd)| CommandItemSpec {
            id: format!("cmd-{idx}"),
            group: "Actions".to_owned(),
            label: cmd.clone(),
            shortcut: Some(format!("Ctrl+{}", idx + 1)),
            selected: idx == 0,
        })
        .collect();

    let palette = NshNode::command_palette(
        "slash-command-palette",
        &CommandPaletteSpec {
            placeholder: "Type a command or /slash action…".to_owned(),
            query: state.command_query.clone(),
            empty_message: "No matching commands".to_owned(),
            items,
        },
        theme,
    );

    let update_dlg = NshNode::dialog(
        "app-update-dialog",
        &DialogSpec {
            title: format!("App Version v{}", state.update_info.current_version),
            description: Some(match &state.update_info.available_version {
                Some(next) => format!(
                    "Signed update v{next} available: {}",
                    state.update_info.notes
                ),
                None => state.update_info.notes.clone(),
            }),
            show_close_button: true,
            destructive: false,
        },
        theme,
    );

    NshNode::view("modal-stack")
        .direction(FlexDirection::Column)
        .gap(theme.spacing.sm)
        .child(palette)
        .child(update_dlg)
}
