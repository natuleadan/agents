use agents_styles::prelude::*;

use crate::core::backend::BackendMode;
use crate::core::state::{AppState, AuthState};

/// Builds the dedicated Login & Provider Mode Connection screen with accessible state feedback.
#[must_use]
pub fn build_login_screen(state: &AppState, metrics: SurfaceMetrics, theme: &Theme) -> NshNode {
    let p = theme.palette();
    let density = metrics.density();
    let is_server = matches!(state.login.mode, BackendMode::Server { .. });

    let card_spec = CardSpec {
        variant: CardVariant::Default,
        section: CardSection {
            title: "Sign In to Agents Orchestrator".to_owned(),
            description: Some(
                "Choose Server Mode or Third-Party Sandbox Adapter and authenticate cleanly."
                    .to_owned(),
            ),
            footer: None,
        },
    }
    .resolve(theme);

    let mode_row = NshNode::view("login-mode-tabs")
        .direction(FlexDirection::Row)
        .gap(theme.spacing.xs)
        .child(mode_tab_button(
            "mode-server-btn",
            "Server Mode",
            is_server,
            theme,
            density,
        ))
        .child(mode_tab_button(
            "mode-thirdparty-btn",
            "Third-Party Adapter",
            !is_server,
            theme,
            density,
        ));

    let invalid_state = matches!(
        state.login.auth_state,
        AuthState::InvalidCredentials | AuthState::SessionExpired
    );
    let url_spec = InputSpec {
        placeholder: "https://orchestrator.example.com".to_owned(),
        value: state.login.server_url.clone(),
        disabled: state.login.auth_state == AuthState::Authenticating,
        masked: false,
        state: InteractionState::Default,
    };
    let email_spec = InputSpec {
        placeholder: "operator@domain.com".to_owned(),
        value: state.login.email.clone(),
        disabled: state.login.auth_state == AuthState::Authenticating,
        masked: false,
        state: if invalid_state {
            InteractionState::Invalid
        } else {
            InteractionState::Default
        },
    };
    let pass_spec = InputSpec {
        placeholder: "Password".to_owned(),
        value: if state.login.password.is_empty() {
            String::new()
        } else {
            "••••••••".to_owned()
        },
        disabled: state.login.auth_state == AuthState::Authenticating,
        masked: true,
        state: if invalid_state {
            InteractionState::Invalid
        } else {
            InteractionState::Default
        },
    };

    let submit_label = match state.login.auth_state {
        AuthState::Authenticating => "Authenticating…",
        AuthState::Offline => "Retry When Online",
        _ => "Sign In & Connect",
    };
    let submit_style = ButtonSpec::new(submit_label)
        .variant(ButtonVariant::Default)
        .state(if state.login.auth_state == AuthState::Authenticating {
            InteractionState::Loading
        } else {
            InteractionState::Default
        })
        .resolve(theme, density);

    let mut card = NshNode::view("login-screen")
        .test_id("login-screen")
        .direction(FlexDirection::Column)
        .gap(theme.spacing.sm)
        .padding(card_spec.padding_x_pt)
        .bg(card_spec.background)
        .fg(card_spec.foreground)
        .border(card_spec.border)
        .radius(card_spec.radius_pt)
        .child(
            NshNode::text("login-title", card_spec_title(card_spec_heading(is_server)))
                .role(AccessibilityRole::Heading)
                .fg(p.foreground),
        )
        .child(mode_row)
        .child(NshNode::text("login-url-label", "Control Plane URL").fg(p.muted_foreground))
        .child(NshNode::input("login-url-input", &url_spec, theme, density))
        .child(NshNode::text("login-email-label", "Operator Email").fg(p.muted_foreground))
        .child(NshNode::input(
            "login-email-input",
            &email_spec,
            theme,
            density,
        ))
        .child(NshNode::text("login-pass-label", "Password").fg(p.muted_foreground))
        .child(NshNode::input(
            "login-pass-input",
            &pass_spec,
            theme,
            density,
        ))
        .child(
            NshNode::button("login-submit-btn", submit_label)
                .bg(submit_style.background)
                .fg(submit_style.foreground)
                .radius(submit_style.radius_pt),
        );

    if let Some(status_banner) = auth_status_message(state) {
        card = card.child(
            NshNode::view("login-error-banner")
                .role(AccessibilityRole::Alert)
                .padding(theme.spacing.sm)
                .bg(p.destructive)
                .fg(p.destructive_foreground)
                .radius(theme.radius.md())
                .child(NshNode::text("login-error-text", status_banner)),
        );
    }

    card
}

fn mode_tab_button(
    id: &str,
    label: &str,
    active: bool,
    theme: &Theme,
    density: Density,
) -> NshNode {
    let style = ButtonSpec::new(label)
        .variant(if active {
            ButtonVariant::Secondary
        } else {
            ButtonVariant::Outline
        })
        .resolve(theme, density);
    NshNode::button(id, label)
        .bg(style.background)
        .fg(style.foreground)
        .border(style.border)
        .radius(style.radius_pt)
}

fn card_spec_heading(is_server: bool) -> &'static str {
    if is_server {
        "Connect to Self-Hosted Server"
    } else {
        "Connect via Third-Party Sandbox Provider"
    }
}

fn card_spec_title(s: &str) -> String {
    s.to_owned()
}

fn auth_status_message(state: &AppState) -> Option<String> {
    if let Some(msg) = &state.login.error_message {
        return Some(msg.clone());
    }
    match state.login.auth_state {
        AuthState::RateLimited => Some("Rate limit exceeded; retry shortly.".to_owned()),
        AuthState::SessionExpired => Some("Session expired; please sign in again.".to_owned()),
        AuthState::InvalidCredentials => Some("Invalid email or password.".to_owned()),
        AuthState::Offline => Some("Network unavailable; working in offline mode.".to_owned()),
        _ => None,
    }
}
