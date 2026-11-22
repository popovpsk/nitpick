use std::collections::BTreeMap;

use agent::{AcpFlavor, HarnessKind};
use rstest::rstest;

use super::{LoginHint, Profile, format_custom_agents, parse_custom_agents, set_override};
use crate::settings_store::{AgentSettings, CustomAgent};

fn custom(name: &str, command: &str) -> CustomAgent {
    CustomAgent {
        name: name.to_string(),
        command: command.to_string(),
    }
}

fn with_custom(agents: &[(&str, &str)]) -> AgentSettings {
    AgentSettings {
        custom: agents.iter().map(|(n, c)| custom(n, c)).collect(),
        ..AgentSettings::default()
    }
}

#[test]
fn built_in_profiles_come_first_in_the_documented_order() {
    let profiles = AgentSettings::default().profiles();
    let rows: Vec<_> = profiles
        .iter()
        .map(|p| {
            (
                p.id.as_str(),
                p.name.as_str(),
                p.kind,
                p.default_command.as_str(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            ("claude", "Claude Code", HarnessKind::Claude, "claude"),
            ("codex", "Codex", HarnessKind::Codex, "codex"),
            (
                "opencode",
                "OpenCode",
                HarnessKind::Acp(AcpFlavor::OpenCode),
                "opencode acp"
            ),
            (
                "deepseek",
                "DeepSeek Harness",
                HarnessKind::Acp(AcpFlavor::DeepSeek),
                "dsh acp"
            ),
            ("pi", "Pi", HarnessKind::Pi, "pi"),
        ]
    );
    assert!(profiles.iter().all(|p| !p.custom));
}

#[test]
fn custom_profiles_follow_with_a_prefixed_id_and_the_generic_acp_kind() {
    let settings = with_custom(&[("my agent", " my-agent --acp ")]);
    let profiles = settings.profiles();
    let last = profiles.last().unwrap();
    assert_eq!(last.id, "custom:my agent");
    assert_eq!(last.name, "my agent");
    assert_eq!(last.kind, HarnessKind::Acp(AcpFlavor::Generic));
    assert_eq!(last.default_command, "my-agent --acp");
    assert!(last.custom);
    assert_eq!(settings.profile("custom:my agent").as_ref(), Some(last));
}

#[test]
fn unusable_custom_entries_in_the_file_are_not_profiles() {
    let settings = with_custom(&[
        ("", "x"),
        ("claude", "x"),
        ("Codex", "x"),
        ("good", "x"),
        ("GOOD", "y"),
        ("empty", "  "),
        (" padded", "x"),
    ]);
    let ids: Vec<String> = settings.profiles().into_iter().map(|p| p.id).collect();
    assert_eq!(ids[5..], ["custom:good".to_string()]);
}

#[rstest]
#[case::builtin("codex", "codex")]
#[case::custom("custom:mine", "custom:mine")]
#[case::empty("", "claude")]
#[case::unknown("gemini", "claude")]
#[case::removed_custom("custom:gone", "claude")]
fn the_active_profile_falls_back_to_claude(#[case] active: &str, #[case] expected: &str) {
    let settings = AgentSettings {
        active: active.to_string(),
        ..with_custom(&[("mine", "mine")])
    };
    assert_eq!(settings.active_profile().id, expected);
}

#[rstest]
#[case::default(None, "claude")]
#[case::override_wins(Some("my-claude-code"), "my-claude-code")]
#[case::trimmed(Some("  my-claude-code --x  "), "my-claude-code --x")]
#[case::blank(Some("   "), "claude")]
#[case::empty(Some(""), "claude")]
fn the_command_is_the_override_or_the_default(
    #[case] stored: Option<&str>,
    #[case] expected: &str,
) {
    let mut settings = AgentSettings::default();
    if let Some(stored) = stored {
        settings
            .commands
            .insert("claude".to_string(), stored.to_string());
    }
    let profile = settings.profile("claude").unwrap();
    assert_eq!(settings.command(&profile), expected);
}

#[test]
fn an_override_applies_to_a_custom_profile_too() {
    let mut settings = with_custom(&[("mine", "mine --acp")]);
    let profile = settings.profile("custom:mine").unwrap();
    assert_eq!(settings.command(&profile), "mine --acp");
    settings
        .commands
        .insert("custom:mine".to_string(), "wrapper mine".to_string());
    assert_eq!(settings.command(&profile), "wrapper mine");
}

#[rstest]
#[case::none(None, None)]
#[case::blank(Some("  "), None)]
#[case::set(Some(" opus "), Some("opus"))]
fn the_model_is_none_when_blank(#[case] stored: Option<&str>, #[case] expected: Option<&str>) {
    let mut settings = AgentSettings::default();
    if let Some(stored) = stored {
        settings
            .models
            .insert("claude".to_string(), stored.to_string());
    }
    let profile = settings.profile("claude").unwrap();
    assert_eq!(settings.model(&profile).as_deref(), expected);
}

#[rstest]
#[case::claude_unix("claude", false, true)]
#[case::claude_windows("claude", true, false)]
#[case::codex_unix("codex", false, true)]
#[case::codex_windows("codex", true, true)]
#[case::opencode("opencode", false, false)]
#[case::deepseek("deepseek", false, false)]
#[case::pi("pi", false, false)]
#[case::pi_windows("pi", true, false)]
#[case::custom("custom:mine", false, false)]
fn only_claude_and_codex_have_a_sandbox(
    #[case] id: &str,
    #[case] windows: bool,
    #[case] expected: bool,
) {
    let settings = with_custom(&[("mine", "mine")]);
    let profile: Profile = settings.profile(id).unwrap();
    assert_eq!(profile.has_sandbox_on(windows), expected);
}

#[rstest]
#[case::claude("claude", Some(LoginHint::Command("claude auth login")))]
#[case::codex("codex", Some(LoginHint::Command("codex login")))]
#[case::opencode("opencode", Some(LoginHint::Command("opencode auth login")))]
#[case::deepseek("deepseek", Some(LoginHint::EnvVar("DEEPSEEK_API_KEY")))]
#[case::pi("pi", Some(LoginHint::Command("pi auth login")))]
#[case::custom("custom:mine", None)]
fn every_built_in_profile_has_a_login_hint(#[case] id: &str, #[case] expected: Option<LoginHint>) {
    let settings = with_custom(&[("mine", "mine")]);
    assert_eq!(settings.profile(id).unwrap().login_hint(), expected);
}

#[test]
fn blank_overrides_are_removed_from_the_map() {
    let mut map = BTreeMap::new();
    set_override(&mut map, "claude", "  wrapper  ");
    assert_eq!(map.get("claude").map(String::as_str), Some("wrapper"));
    set_override(&mut map, "claude", "   ");
    assert!(map.is_empty());
}

#[test]
fn custom_agent_lines_become_agents() {
    let parsed = parse_custom_agents("my-agent = my-agent acp\n\n  other=run --flag=1  \n");
    assert_eq!(
        parsed.agents,
        vec![
            custom("my-agent", "my-agent acp"),
            custom("other", "run --flag=1")
        ]
    );
    assert!(parsed.ignored.is_empty());
}

#[rstest]
#[case::no_equals("just-a-line")]
#[case::empty_name("= command")]
#[case::empty_command("name =")]
#[case::blank_command("name =   ")]
#[case::builtin_id("claude = my-claude")]
#[case::builtin_id_any_case("Pi = my-pi")]
fn unusable_lines_are_reported_and_skipped(#[case] line: &str) {
    let parsed = parse_custom_agents(&format!("first = one\n{line}\nlast = two"));
    assert_eq!(
        parsed.agents,
        vec![custom("first", "one"), custom("last", "two")]
    );
    assert_eq!(parsed.ignored, vec![line.trim().to_string()]);
}

#[test]
fn a_repeated_name_keeps_the_first_line() {
    let parsed = parse_custom_agents("a = one\nA = two\nb = three\na = four");
    assert_eq!(
        parsed.agents,
        vec![custom("a", "one"), custom("b", "three")]
    );
    assert_eq!(parsed.ignored, vec!["A = two", "a = four"]);
}

#[test]
fn custom_agents_round_trip_through_the_text_field() {
    let agents = vec![
        custom("my agent", "my-agent --acp"),
        custom("second", "node server.js --mode=acp"),
    ];
    let text = format_custom_agents(&agents);
    assert_eq!(
        text,
        "my agent = my-agent --acp\nsecond = node server.js --mode=acp"
    );
    assert_eq!(parse_custom_agents(&text).agents, agents);
    assert_eq!(format_custom_agents(&[]), "");
    assert!(parse_custom_agents("").agents.is_empty());
}

#[test]
fn formatting_what_was_parsed_is_stable() {
    let parsed = parse_custom_agents("  b=  two \na = one\nbad\nb = again");
    let text = format_custom_agents(&parsed.agents);
    assert_eq!(text, "b = two\na = one");
    assert_eq!(
        format_custom_agents(&parse_custom_agents(&text).agents),
        text
    );
}
