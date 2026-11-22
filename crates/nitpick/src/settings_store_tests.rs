use forge::ForgeKind;
use rstest::rstest;

use super::{
    AgentSettings, CustomAgent, Settings, SyntaxPalettes, WrapLines, clamp_agent_panel_width,
    fit_split_ratio,
};

#[test]
fn settings_without_syntax_colors_still_load() {
    let settings: Settings =
        serde_json::from_str(r#"{ "theme": "system", "diff_layout": "unified" }"#).unwrap();
    assert_eq!(settings.syntax_colors, SyntaxPalettes::default());
    assert!(!settings.show_blame);
    assert!(!settings.show_whole_file);
    assert_eq!(settings.wrap_lines, WrapLines::SplitOnly);
    assert!(settings.editor_command.is_empty());
    assert!(settings.auto_update);
}

#[test]
fn gitlab_only_settings_still_load_and_github_defaults_to_github_com() {
    let settings: Settings = serde_json::from_str(
        r#"{ "gitlab_url": "https://gitlab.example.com", "gitlab_token": "glpat-plain" }"#,
    )
    .unwrap();
    assert_eq!(settings.gitlab_url, "https://gitlab.example.com");
    assert_eq!(settings.gitlab_token, "glpat-plain");
    assert!(settings.github_url.is_empty());
    assert!(settings.github_token.is_empty());
    assert!(settings.forge_configured(ForgeKind::GitLab));
    assert!(!settings.forge_configured(ForgeKind::GitHub));
    assert!(settings.forge_client(ForgeKind::GitHub).is_none());
    assert_eq!(settings.forge_url(ForgeKind::GitHub), "https://github.com");
    assert_eq!(
        settings.forge_hosts(),
        vec![
            (ForgeKind::GitLab, "https://gitlab.example.com".to_string()),
            (ForgeKind::GitHub, "https://github.com".to_string()),
        ]
    );
}

#[test]
fn github_settings_pick_the_enterprise_host() {
    let settings: Settings = serde_json::from_str(
        r#"{ "github_url": " https://ghe.example.com ", "github_token": "ghp-plain" }"#,
    )
    .unwrap();
    assert_eq!(
        settings.forge_url(ForgeKind::GitHub),
        "https://ghe.example.com"
    );
    assert_eq!(
        settings.forge_hosts(),
        vec![(ForgeKind::GitHub, "https://ghe.example.com".to_string())]
    );
    let client = settings.forge_client(ForgeKind::GitHub).unwrap();
    assert_eq!(client.kind(), ForgeKind::GitHub);
    assert_eq!(client.base_url(), "https://ghe.example.com");
}

#[test]
fn syntax_palettes_are_kept_per_appearance() {
    let mut palettes = SyntaxPalettes::default();
    palettes.set(true, Some("Gruvbox Dark".into()));
    assert_eq!(palettes.get(true).map(String::as_str), Some("Gruvbox Dark"));
    assert_eq!(palettes.get(false), None);
    assert_eq!(
        serde_json::to_value(&palettes).unwrap(),
        serde_json::json!({ "dark": "Gruvbox Dark" })
    );
    let settings: Settings =
        serde_json::from_str(r#"{ "syntax_colors": { "dark": "Gruvbox Dark" } }"#).unwrap();
    assert_eq!(settings.syntax_colors, palettes);
    palettes.set(true, None);
    assert_eq!(
        serde_json::to_value(&palettes).unwrap(),
        serde_json::json!({})
    );
}

#[test]
fn settings_without_split_ratio_split_in_the_middle() {
    let settings: Settings = serde_json::from_str(r#"{ "tree_width": 300.0 }"#).unwrap();
    assert_eq!(settings.split_ratio, 0.5);
}

#[rstest]
#[case::kept(0.3, 1000., 0.3)]
#[case::left_column_minimum(0.05, 1000., 0.16)]
#[case::right_column_minimum(0.95, 1000., 0.84)]
#[case::component_range_on_a_wide_area(0.02, 4000., 0.1)]
#[case::too_narrow_for_two_minimums(0.3, 300., 0.5)]
#[case::unmeasured(0.3, 0., 0.5)]
#[case::not_a_number(f32::NAN, 1000., 0.5)]
fn split_ratio_keeps_both_columns_usable(
    #[case] ratio: f32,
    #[case] width: f32,
    #[case] expected: f32,
) {
    assert!((fit_split_ratio(ratio, width) - expected).abs() < 1e-6);
}

#[test]
fn settings_without_the_agent_field_get_its_defaults() {
    let settings: Settings = serde_json::from_str(r#"{ "theme": "system" }"#).unwrap();
    assert_eq!(settings.agent, AgentSettings::default());
    assert!(settings.agent.enabled);
    assert!(!settings.agent.auto_overview);
    assert_eq!(settings.agent.active, "claude");
    assert!(settings.agent.commands.is_empty());
    assert!(settings.agent.models.is_empty());
    assert!(settings.agent.custom.is_empty());
    assert!(settings.agent.instructions.is_empty());
    assert_eq!(settings.agent.panel_width, 480.);
}

#[test]
fn a_partial_agent_section_fills_in_the_rest() {
    let settings: Settings =
        serde_json::from_str(r#"{ "agent": { "auto_overview": true } }"#).unwrap();
    assert!(settings.agent.auto_overview);
    assert!(settings.agent.enabled);
    assert_eq!(settings.agent.active, "claude");
    assert_eq!(settings.agent.panel_width, 480.);
}

#[test]
fn the_agent_section_round_trips() {
    let settings = Settings {
        agent: AgentSettings {
            enabled: false,
            auto_overview: true,
            active: "custom:mine".to_string(),
            commands: [("claude".to_string(), "my-claude-code".to_string())].into(),
            models: [("claude".to_string(), "opus".to_string())].into(),
            custom: vec![CustomAgent {
                name: "mine".to_string(),
                command: "mine --acp".to_string(),
            }],
            instructions: "Be terse.\nAnswer in two sentences.".to_string(),
            panel_width: 620.,
        },
        ..Settings::default()
    };
    let text = serde_json::to_string(&settings).unwrap();
    let back: Settings = serde_json::from_str(&text).unwrap();
    assert_eq!(back.agent, settings.agent);
}

#[rstest]
#[case::kept(480., 480.)]
#[case::lower_bound(320., 320.)]
#[case::too_narrow(10., 320.)]
#[case::upper_bound(900., 900.)]
#[case::too_wide(5000., 900.)]
#[case::not_a_number(f32::NAN, 480.)]
#[case::infinite(f32::INFINITY, 480.)]
fn the_agent_panel_width_stays_in_range(#[case] width: f32, #[case] expected: f32) {
    assert_eq!(clamp_agent_panel_width(width), expected);
}

#[rstest]
#[case::too_narrow("10", 320.)]
#[case::too_wide("5000.5", 900.)]
#[case::fine("600", 600.)]
fn a_stored_agent_panel_width_is_clamped_on_load(#[case] stored: &str, #[case] expected: f32) {
    let settings: Settings =
        serde_json::from_str(&format!(r#"{{ "agent": {{ "panel_width": {stored} }} }}"#)).unwrap();
    assert_eq!(settings.agent.panel_width, expected);
}

#[test]
fn a_binding_for_a_removed_command_still_loads_and_is_kept() {
    let settings: Settings = serde_json::from_str(
        r#"{ "keybindings": { "review.agent_next_stop": ["alt-shift-down"] } }"#,
    )
    .unwrap();
    assert!(crate::keymap::command("review.agent_next_stop").is_none());
    let text = serde_json::to_string(&settings).unwrap();
    assert!(text.contains("review.agent_next_stop"));
}
