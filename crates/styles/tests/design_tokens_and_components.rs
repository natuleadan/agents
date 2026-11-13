use agents_styles::prelude::*;

#[test]
fn light_and_dark_themes_meet_wcag_contrast_and_export_nsh_json() {
    let dark = Theme::with_mode(ThemeMode::Dark, false);
    let light = Theme::with_mode(ThemeMode::Light, true);

    assert!(
        dark.palette()
            .foreground
            .contrast_ratio(dark.palette().background)
            >= 4.5
    );
    assert!(
        light
            .palette()
            .foreground
            .contrast_ratio(light.palette().background)
            >= 4.5
    );
    assert!(
        light
            .palette()
            .primary_foreground
            .contrast_ratio(light.palette().primary)
            >= 4.5
    );

    let json = dark.to_nsh_css_vars_json().expect("serialize theme json");
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid json");
    assert_eq!(parsed["cssVars"]["theme"]["radius"], "7.2px");
    assert!(parsed["cssVars"]["light"]["primary"].as_str().is_some());
    assert!(parsed["cssVars"]["dark"]["sidebar"].as_str().is_some());
}

#[test]
fn button_badge_and_card_resolve_styles_across_densities() {
    let theme = Theme::default();
    let compact_btn = ButtonSpec::new("Run")
        .variant(ButtonVariant::Default)
        .size(ButtonSize::Xs)
        .resolve(&theme, Density::Compact);
    let touch_btn = ButtonSpec::new("Run")
        .variant(ButtonVariant::Outline)
        .size(ButtonSize::Xs)
        .state(InteractionState::Focused)
        .resolve(&theme, Density::Touch);

    assert_eq!(compact_btn.height_pt, 28.0);
    assert_eq!(touch_btn.height_pt, 44.0);
    assert!(touch_btn.focus_ring.is_some());
    assert_eq!(touch_btn.border_width_pt, 1.0);

    let badge = BadgeSpec::new("local · docker + openshell", BadgeVariant::Outline).resolve(&theme);
    assert_eq!(badge.radius_pt, 999.0);

    let card = CardSpec {
        variant: CardVariant::Sm,
        section: CardSection {
            title: "Workers".to_owned(),
            description: None,
            footer: None,
        },
    }
    .resolve(&theme);
    assert_eq!(card.padding_x_pt, 12.0);
}

#[test]
fn responsive_metrics_and_signal_html_projection_work() {
    let mobile = SurfaceMetrics::mobile(TargetPlatform::Ios);
    let desktop = SurfaceMetrics::desktop(TargetPlatform::MacOs);
    let web_compact = SurfaceMetrics::web(375.0, 812.0);

    assert_eq!(mobile.responsive_class(), ResponsiveClass::Compact);
    assert!(mobile.prefers_bottom_sheet());
    assert_eq!(desktop.responsive_class(), ResponsiveClass::Expanded);
    assert!(!desktop.prefers_bottom_sheet());
    assert_eq!(web_compact.responsive_class(), ResponsiveClass::Compact);

    let count = Signal::new(1_u32);
    count.update(|v| *v += 2);
    assert_eq!(count.get(), 3);

    let node = NshNode::view("root")
        .direction(FlexDirection::Column)
        .child(NshNode::text(
            "status",
            format!("Count: {} <safe>", count.get()),
        ));

    let html = node.render_html_fragment();
    assert!(html.contains("&lt;safe&gt;"));
    assert_eq!(node.flatten_text_lines(), vec!["Count: 3 <safe>"]);
}
