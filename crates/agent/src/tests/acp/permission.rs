use rstest::rstest;
use serde_json::{Value, json};

use crate::acp::permission::{Verdict, answer, outcome, tool_call_id, verdict};

fn standard() -> Value {
    json!([
        {"optionId": "once", "kind": "allow_once", "name": "Allow once"},
        {"optionId": "always", "kind": "allow_always", "name": "Always allow"},
        {"optionId": "reject", "kind": "reject_once", "name": "Reject"},
    ])
}

fn selected(id: &str) -> Value {
    json!({"outcome": {"outcome": "selected", "optionId": id}})
}

fn cancelled() -> Value {
    json!({"outcome": {"outcome": "cancelled"}})
}

#[rstest]
#[case::edit(Some("edit"), Verdict::Reject)]
#[case::delete(Some("delete"), Verdict::Reject)]
#[case::move_(Some("move"), Verdict::Reject)]
#[case::capitalized(Some("Edit"), Verdict::Reject)]
#[case::padded(Some(" edit "), Verdict::Reject)]
#[case::read(Some("read"), Verdict::Allow)]
#[case::search(Some("search"), Verdict::Allow)]
#[case::execute(Some("execute"), Verdict::Allow)]
#[case::think(Some("think"), Verdict::Allow)]
#[case::fetch(Some("fetch"), Verdict::Allow)]
#[case::switch_mode(Some("switch_mode"), Verdict::Allow)]
#[case::other(Some("other"), Verdict::Allow)]
#[case::unknown(Some("teleport"), Verdict::Allow)]
#[case::empty(Some(""), Verdict::Allow)]
#[case::missing(None, Verdict::Allow)]
fn a_tool_kind_decides_the_verdict(#[case] kind: Option<&str>, #[case] expected: Verdict) {
    assert_eq!(verdict(&[kind]), expected);
}

#[rstest]
#[case::either_is_a_write(&[Some("read"), Some("edit")], Verdict::Reject)]
#[case::the_other_is_a_write(&[Some("edit"), Some("read")], Verdict::Reject)]
#[case::both_reads(&[Some("read"), Some("search")], Verdict::Allow)]
#[case::one_unknown(&[None, Some("execute")], Verdict::Allow)]
#[case::none(&[None, None], Verdict::Allow)]
fn a_write_kind_anywhere_rejects(#[case] kinds: &[Option<&str>], #[case] expected: Verdict) {
    assert_eq!(verdict(kinds), expected);
}

#[rstest]
#[case::allow_by_kind(Verdict::Allow, standard(), selected("once"))]
#[case::reject_by_kind(Verdict::Reject, standard(), selected("reject"))]
#[case::ids_are_not_read(
    Verdict::Allow,
    json!([
        {"optionId": "reject", "kind": "allow_once"},
        {"optionId": "allow", "kind": "reject_once"},
    ]),
    selected("reject")
)]
#[case::position_is_not_read(
    Verdict::Reject,
    json!([
        {"optionId": "a", "kind": "reject_once"},
        {"optionId": "b", "kind": "allow_once"},
    ]),
    selected("a")
)]
#[case::hyphenated_kind(
    Verdict::Allow,
    json!([{"optionId": "x", "kind": "allow-once"}, {"optionId": "y", "kind": "reject-once"}]),
    selected("x")
)]
#[case::deepseek_ids(
    Verdict::Reject,
    json!([
        {"optionId": "allow-once", "kind": "allow_once"},
        {"optionId": "reject-once", "kind": "reject_once"},
    ]),
    selected("reject-once")
)]
#[case::first_of_several(
    Verdict::Allow,
    json!([
        {"optionId": "one", "kind": "allow_once"},
        {"optionId": "two", "kind": "allow_once"},
    ]),
    selected("one")
)]
#[case::allow_always_is_never_chosen(
    Verdict::Allow,
    json!([
        {"optionId": "always", "kind": "allow_always"},
        {"optionId": "no", "kind": "reject_once"},
    ]),
    cancelled()
)]
#[case::reject_always_is_not_a_reject_once(
    Verdict::Reject,
    json!([
        {"optionId": "yes", "kind": "allow_once"},
        {"optionId": "never", "kind": "reject_always"},
    ]),
    cancelled()
)]
#[case::a_nameless_kind(Verdict::Allow, json!([{"optionId": "once"}]), cancelled())]
#[case::no_options(Verdict::Allow, json!([]), cancelled())]
#[case::options_are_not_a_list(Verdict::Allow, json!({"optionId": "once"}), cancelled())]
#[case::options_are_missing(Verdict::Reject, Value::Null, cancelled())]
fn an_option_is_chosen_by_its_kind(
    #[case] verdict: Verdict,
    #[case] options: Value,
    #[case] expected: Value,
) {
    assert_eq!(outcome(verdict, &options), expected);
}

fn request(kind: Option<&str>) -> Value {
    let mut call = json!({"toolCallId": "call-1", "title": "do it"});
    if let Some(kind) = kind {
        call["kind"] = json!(kind);
    }
    json!({"sessionId": "s", "toolCall": call, "options": standard()})
}

#[rstest]
#[case::own_kind_edit(Some("edit"), None, selected("reject"))]
#[case::own_kind_read(Some("read"), None, selected("once"))]
#[case::remembered_edit(None, Some("edit"), selected("reject"))]
#[case::remembered_execute(None, Some("execute"), selected("once"))]
#[case::nothing_known(None, None, selected("once"))]
#[case::a_write_on_either_side(Some("read"), Some("move"), selected("reject"))]
fn a_request_without_a_kind_uses_the_remembered_one(
    #[case] own: Option<&str>,
    #[case] remembered: Option<&str>,
    #[case] expected: Value,
) {
    assert_eq!(answer(&request(own), remembered), expected);
}

#[test]
fn the_tool_call_id_is_read_from_the_request() {
    assert_eq!(tool_call_id(&request(None)), Some("call-1"));
    assert_eq!(tool_call_id(&json!({"toolCall": {}})), None);
    assert_eq!(tool_call_id(&json!({})), None);
}
