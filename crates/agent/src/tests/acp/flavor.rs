use rstest::rstest;
use serde_json::{Value, json};

use crate::acp::flavor::{
    Dialect, OPENCODE_CONFIG, OPENCODE_NO_PROJECT_CONFIG, ResumeVia, environment, failure,
    needs_version, opencode_config, resume_via, session_lost, wants_plan_mode,
};
use crate::acp::wire::{Capabilities, config_option};
use crate::jsonrpc::RpcError;
use crate::{AcpFlavor, FailureKind};

const OPENCODE_INIT: &str = include_str!("../../fixtures/opencode/basic.jsonl");
const OPENCODE_PLAN: &str = include_str!("../../fixtures/opencode/plan_mode.jsonl");

fn recorded(text: &str, id: i64) -> Value {
    text.lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|line| line["id"] == id)
        .map(|line| line["result"].clone())
        .expect("the recording answers the request")
}

fn error(code: i64, message: &str, data: Option<Value>) -> RpcError {
    RpcError {
        code,
        message: message.into(),
        data,
    }
}

#[rstest]
#[case::recorded("1.19.4", Dialect::V1)]
#[case::two("2.0.0", Dialect::V2)]
#[case::prefixed("v2.1.3", Dialect::V2)]
#[case::named("opencode 2.0.1", Dialect::V2)]
#[case::pre_release("2.0.0-beta.3", Dialect::V2)]
#[case::padded("\n  2.4.0  \n", Dialect::V2)]
#[case::another_tool("10.8.2", Dialect::V1)]
#[case::no_minor("2", Dialect::V1)]
#[case::garbage("it works", Dialect::V1)]
#[case::empty("", Dialect::V1)]
fn the_dialect_follows_the_major_version(#[case] output: &str, #[case] expected: Dialect) {
    assert_eq!(Dialect::from_version(Some(output)), expected);
}

#[test]
fn an_unknown_version_gets_the_verified_dialect() {
    assert_eq!(Dialect::from_version(None), Dialect::V1);
}

#[test]
fn the_two_dialects_are_valid_json_with_the_edit_ban() {
    let v1: Value = serde_json::from_str(&opencode_config(Dialect::V1)).unwrap();
    assert_eq!(v1, json!({"permission": {"edit": "deny"}}));
    let v2: Value = serde_json::from_str(&opencode_config(Dialect::V2)).unwrap();
    assert_eq!(
        v2,
        json!({"permissions": [{"action": "edit", "resource": "*", "effect": "deny"}]})
    );
}

#[test]
fn opencode_gets_the_config_and_no_project_config() {
    let env = environment(AcpFlavor::OpenCode, Dialect::V1);
    assert_eq!(
        env,
        vec![
            (
                OPENCODE_CONFIG.to_owned(),
                r#"{"permission":{"edit":"deny"}}"#.to_owned()
            ),
            (OPENCODE_NO_PROJECT_CONFIG.to_owned(), "1".to_owned()),
        ]
    );
}

#[rstest]
#[case::deepseek(AcpFlavor::DeepSeek)]
#[case::generic(AcpFlavor::Generic)]
fn other_flavors_add_no_environment(#[case] flavor: AcpFlavor) {
    assert!(environment(flavor, Dialect::V2).is_empty());
    assert!(!needs_version(flavor));
    assert!(!wants_plan_mode(flavor));
}

#[test]
fn only_opencode_needs_the_version_and_the_plan_mode() {
    assert!(needs_version(AcpFlavor::OpenCode));
    assert!(wants_plan_mode(AcpFlavor::OpenCode));
}

#[rstest]
#[case::recorded_opencode(recorded(OPENCODE_INIT, 1), Capabilities { load_session: true, resume: true, system_prompt: false })]
#[case::empty(json!({}), Capabilities::default())]
#[case::resume_flag(
    json!({"agentCapabilities": {"sessionCapabilities": {"resume": true}}}),
    Capabilities { load_session: false, resume: true, system_prompt: false }
)]
#[case::resume_off(
    json!({"agentCapabilities": {"sessionCapabilities": {"resume": false, "systemPrompt": null}}}),
    Capabilities::default()
)]
#[case::system_prompt(
    json!({"agentCapabilities": {"sessionCapabilities": {"systemPrompt": {}}}}),
    Capabilities { load_session: false, resume: false, system_prompt: true }
)]
#[case::mistyped(
    json!({"agentCapabilities": {"loadSession": "yes", "sessionCapabilities": []}}),
    Capabilities::default()
)]
fn capabilities_are_read_from_initialize(#[case] result: Value, #[case] expected: Capabilities) {
    assert_eq!(Capabilities::from_initialize(&result), expected);
}

fn caps(load_session: bool, resume: bool) -> Capabilities {
    Capabilities {
        load_session,
        resume,
        system_prompt: false,
    }
}

#[rstest]
#[case::resume_wins(AcpFlavor::OpenCode, caps(true, true), Some(ResumeVia::Resume))]
#[case::load_only(AcpFlavor::Generic, caps(true, false), Some(ResumeVia::Load))]
#[case::resume_only(AcpFlavor::Generic, caps(false, true), Some(ResumeVia::Resume))]
#[case::nothing(AcpFlavor::Generic, caps(false, false), None)]
#[case::deepseek_resume(AcpFlavor::DeepSeek, caps(false, true), Some(ResumeVia::Resume))]
#[case::deepseek_never_loads(AcpFlavor::DeepSeek, caps(true, false), None)]
fn resuming_follows_the_declared_capabilities(
    #[case] flavor: AcpFlavor,
    #[case] capabilities: Capabilities,
    #[case] expected: Option<ResumeVia>,
) {
    assert_eq!(resume_via(flavor, &capabilities), expected);
}

#[test]
fn the_resume_methods_have_their_wire_names() {
    assert_eq!(ResumeVia::Resume.method(), "session/resume");
    assert_eq!(ResumeVia::Load.method(), "session/load");
}

#[rstest]
#[case::opencode_shape(AcpFlavor::OpenCode, error(-32603, "Internal error: OpenCode service failure", Some(json!({"service": "session"}))), true)]
#[case::opencode_another_service(AcpFlavor::OpenCode, error(-32603, "Internal error: OpenCode service failure", Some(json!({"service": "provider"}))), false)]
#[case::opencode_without_data(AcpFlavor::OpenCode, error(-32603, "Internal error", None), false)]
#[case::the_shape_is_opencode_only(AcpFlavor::Generic, error(-32603, "Internal error", Some(json!({"service": "session"}))), false)]
#[case::spec_code(AcpFlavor::Generic, error(-32002, "Resource not found", None), true)]
#[case::spec_code_for_deepseek(AcpFlavor::DeepSeek, error(-32002, "x", None), true)]
#[case::text(AcpFlavor::Generic, error(-32602, "Session not found: abc", None), true)]
#[case::text_any_case(AcpFlavor::Generic, error(-32603, "SESSION NOT FOUND", None), true)]
#[case::unrelated(AcpFlavor::Generic, error(-32603, "disk is full", None), false)]
fn a_missing_session_is_recognised(
    #[case] flavor: AcpFlavor,
    #[case] error: RpcError,
    #[case] expected: bool,
) {
    assert_eq!(session_lost(flavor, &error), expected);
}

#[rstest]
#[case::lost_on_resume(AcpFlavor::OpenCode, error(-32603, "m", Some(json!({"service": "session"}))), true, FailureKind::SessionLost)]
#[case::the_same_error_elsewhere(AcpFlavor::OpenCode, error(-32603, "m", Some(json!({"service": "session"}))), false, FailureKind::Other)]
#[case::opencode_login(AcpFlavor::OpenCode, error(-32000, "Authentication required", None), false, FailureKind::NotLoggedIn)]
#[case::opencode_login_on_resume(AcpFlavor::OpenCode, error(-32000, "Authentication required", None), true, FailureKind::NotLoggedIn)]
#[case::generic_code_is_not_a_login(AcpFlavor::Generic, error(-32000, "Server error", None), false, FailureKind::Other)]
#[case::auth_required_text(AcpFlavor::Generic, error(-32603, "auth_required", None), false, FailureKind::NotLoggedIn)]
#[case::auth_required_in_data(AcpFlavor::Generic, error(-32603, "failed", Some(json!({"reason": "AUTH_REQUIRED"}))), false, FailureKind::NotLoggedIn)]
#[case::deepseek_credential_in_message(AcpFlavor::DeepSeek, error(-32603, "MISSING_CREDENTIAL: DEEPSEEK_API_KEY", None), false, FailureKind::NotLoggedIn)]
#[case::deepseek_credential_in_data(AcpFlavor::DeepSeek, error(-32603, "failed", Some(json!({"code": "MISSING_CREDENTIAL"}))), false, FailureKind::NotLoggedIn)]
#[case::the_credential_text_is_deepseek_only(AcpFlavor::Generic, error(-32603, "MISSING_CREDENTIAL", None), false, FailureKind::Other)]
#[case::plain(AcpFlavor::OpenCode, error(-32603, "boom", None), false, FailureKind::Other)]
fn an_error_is_classified_by_flavor(
    #[case] flavor: AcpFlavor,
    #[case] error: RpcError,
    #[case] resuming: bool,
    #[case] expected: FailureKind,
) {
    assert_eq!(failure(flavor, &error, resuming).kind, expected);
}

#[test]
fn an_error_without_a_message_still_says_something() {
    let failure = failure(AcpFlavor::Generic, &error(-32603, "  ", None), false);
    assert!(failure.message.contains("-32603"), "{}", failure.message);
}

#[test]
fn the_recorded_options_name_the_model_and_the_modes() {
    let session = recorded(OPENCODE_PLAN, 2);
    let model = config_option(&session, "model").expect("a model option");
    assert_eq!(model.id, "model");
    assert!(model.is_current("opencode/big-pickle"));
    assert!(model.offers("deepseek/deepseek-v4-pro"));
    assert!(!model.offers("nothing/here"));
    let mode = config_option(&session, "mode").expect("a mode option");
    assert!(mode.is_current("build"));
    assert!(mode.offers("plan"));
    assert!(mode.offers("build"));
    let after = config_option(&recorded(OPENCODE_PLAN, 3), "mode").unwrap();
    assert!(after.is_current("plan"));
    assert!(config_option(&session, "thought_level").is_none());
}

#[test]
fn grouped_option_values_are_found_too() {
    let result = json!({"configOptions": [{
        "id": "m", "category": "model", "currentValue": "a",
        "options": [{"group": "g", "options": [{"value": "a"}, {"value": "b"}]}, {"value": "c"}],
    }]});
    let option = config_option(&result, "model").unwrap();
    assert!(option.offers("a") && option.offers("b") && option.offers("c"));
}
