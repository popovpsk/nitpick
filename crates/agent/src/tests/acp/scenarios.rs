use std::time::Duration;

use rstest::rstest;
use serde_json::{Value, json};

use super::rig::*;
use crate::acp::Limits;
use crate::session::Command;
use crate::{AcpFlavor, Event, FailureKind, Launch};

const STOPPED: &str = "The assistant stopped before it finished the reply.";
const NOT_READY: &str = "The assistant stopped before the session was ready.";
const SILENT: &str = "The assistant is not responding.";

fn options() -> Value {
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

async fn ask(rig: &mut Rig, id: Value, call: Value, options: Value) {
    rig.write(json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "session/request_permission",
        "params": {"sessionId": SESSION, "toolCall": call, "options": options},
    }))
    .await;
}

async fn running_turn(flavor: AcpFlavor, capabilities: &str) -> (Rig, Value) {
    running_turn_within(flavor, capabilities, relaxed()).await
}

async fn running_tight(capabilities: &str) -> (Rig, Value) {
    running_turn_within(AcpFlavor::Generic, capabilities, tight()).await
}

async fn running_turn_within(
    flavor: AcpFlavor,
    capabilities: &str,
    limits: Limits,
) -> (Rig, Value) {
    let mut rig = Rig::start_with(flavor, launch(), limits);
    rig.prompt("hi");
    rig.ready(capabilities).await;
    let (id, _) = rig.read_prompt().await;
    (rig, id)
}

#[rstest]
#[case::edit_is_rejected(None, Some("edit"), options(), selected("reject"))]
#[case::delete_is_rejected(None, Some("delete"), options(), selected("reject"))]
#[case::move_is_rejected(None, Some("move"), options(), selected("reject"))]
#[case::read_is_allowed(None, Some("read"), options(), selected("once"))]
#[case::execute_is_allowed(None, Some("execute"), options(), selected("once"))]
#[case::other_is_allowed(None, Some("other"), options(), selected("once"))]
#[case::search_is_allowed(None, Some("search"), options(), selected("once"))]
#[case::unknown_is_allowed(None, Some("teleport"), options(), selected("once"))]
#[case::no_kind_at_all_is_allowed(None, None, options(), selected("once"))]
#[case::the_kind_comes_from_the_earlier_call(Some("edit"), None, options(), selected("reject"))]
#[case::a_read_from_the_earlier_call(Some("read"), None, options(), selected("once"))]
#[case::an_earlier_write_beats_a_later_read(
    Some("edit"),
    Some("read"),
    options(),
    selected("reject")
)]
#[case::no_reject_option_cancels(
    None,
    Some("edit"),
    json!([{"optionId": "once", "kind": "allow_once"}, {"optionId": "always", "kind": "allow_always"}]),
    cancelled()
)]
#[case::allow_always_alone_cancels(
    None,
    Some("read"),
    json!([{"optionId": "always", "kind": "allow_always"}, {"optionId": "never", "kind": "reject_always"}]),
    cancelled()
)]
#[case::no_options_cancels(None, Some("read"), json!([]), cancelled())]
#[tokio::test]
async fn a_permission_request_is_answered_by_the_tool_kind(
    #[case] earlier: Option<&str>,
    #[case] own: Option<&str>,
    #[case] options: Value,
    #[case] expected: Value,
) {
    let (mut rig, prompt) = running_turn(AcpFlavor::Generic, NOTHING).await;
    let mut call = json!({"toolCallId": "t1", "title": "do it"});
    if let Some(kind) = earlier {
        rig.tool("t1", Some(kind), "pending", "do it").await;
    }
    if let Some(kind) = own {
        call["kind"] = json!(kind);
    }
    ask(&mut rig, json!(77), call, options).await;
    let reply = rig.read().await;
    assert_eq!(
        reply,
        json!({"jsonrpc": "2.0", "id": 77, "result": expected})
    );
    rig.finish_prompt(prompt, "end_turn").await;
    let events = rig.until_finished().await;
    assert_eq!(events.last(), Some(&completed()));
    rig.stop().await;
}

#[tokio::test]
async fn permission_requests_are_answered_between_turns_and_during_setup_too() {
    let mut rig = Rig::start(AcpFlavor::Generic);
    rig.prompt("hi");
    rig.initialize(NOTHING).await;
    ask(
        &mut rig,
        json!(5),
        json!({"toolCallId": "t", "kind": "edit"}),
        options(),
    )
    .await;
    let (first, second) = (rig.read().await, rig.read().await);
    let (new, reply) = if first.get("method").is_some() {
        (first, second)
    } else {
        (second, first)
    };
    assert_eq!(new["method"], "session/new");
    assert_eq!(reply["id"], 5);
    assert_eq!(reply["result"], selected("reject"));
    rig.respond(new["id"].clone(), json!({"sessionId": SESSION}))
        .await;
    let (prompt, _) = rig.read_prompt().await;
    rig.finish_prompt(prompt, "end_turn").await;
    rig.until_finished().await;
    ask(
        &mut rig,
        json!(6),
        json!({"toolCallId": "t", "kind": "read"}),
        options(),
    )
    .await;
    assert_eq!(rig.read().await["result"], selected("once"));
    rig.stop().await;
}

#[rstest]
#[case::read_file("fs/read_text_file")]
#[case::write_file("fs/write_text_file")]
#[case::terminal("terminal/create")]
#[case::terminal_output("terminal/output")]
#[case::elicitation("elicitation/create")]
#[case::vendor("_vendor/ask")]
#[case::made_up("session/teleport")]
#[tokio::test]
async fn an_unknown_agent_request_is_rejected(#[case] method: &str) {
    let (mut rig, prompt) = running_turn(AcpFlavor::Generic, NOTHING).await;
    rig.write(json!({"jsonrpc": "2.0", "id": "abc", "method": method, "params": {"path": "/x"}}))
        .await;
    let reply = rig.read().await;
    assert_eq!(reply["id"], "abc");
    assert_eq!(reply["error"]["code"], -32601);
    assert!(reply.get("result").is_none());
    rig.finish_prompt(prompt, "end_turn").await;
    assert_eq!(
        rig.until_finished().await,
        vec![session(SESSION), completed()]
    );
    rig.stop().await;
}

#[tokio::test]
async fn a_new_session_declares_no_client_capabilities() {
    let mut rig = Rig::start(AcpFlavor::Generic);
    rig.prompt("hi");
    let init = rig.initialize(NOTHING).await;
    assert_eq!(init["protocolVersion"], 1);
    assert_eq!(
        init["clientCapabilities"],
        json!({"fs": {"readTextFile": false, "writeTextFile": false}, "terminal": false})
    );
    assert_eq!(init["clientInfo"]["name"], "nitpick");
    let params = rig.new_session(json!([])).await;
    assert_eq!(params, json!({"cwd": absolute_cwd(), "mcpServers": []}));
    assert_eq!(rig.next().await, session(SESSION));
    rig.stop().await;
}

fn resuming() -> Launch {
    Launch {
        resume: Some("ses_old".into()),
        ..launch()
    }
}

#[rstest]
#[case::resume_is_preferred(AcpFlavor::OpenCode, RESUME_AND_LOAD, Some("session/resume"))]
#[case::load_without_resume(AcpFlavor::Generic, LOAD_ONLY, Some("session/load"))]
#[case::resume_alone(
    AcpFlavor::Generic,
    r#"{"sessionCapabilities":{"resume":{}}}"#,
    Some("session/resume")
)]
#[case::deepseek_resumes(
    AcpFlavor::DeepSeek,
    r#"{"sessionCapabilities":{"resume":{}}}"#,
    Some("session/resume")
)]
#[case::neither_is_a_lost_session(AcpFlavor::Generic, NOTHING, None)]
#[case::deepseek_never_loads(AcpFlavor::DeepSeek, LOAD_ONLY, None)]
#[tokio::test]
async fn resuming_follows_what_the_agent_declared(
    #[case] flavor: AcpFlavor,
    #[case] capabilities: &str,
    #[case] method: Option<&str>,
) {
    let mut rig = Rig::start_with(flavor, resuming(), relaxed());
    rig.prompt("hi");
    rig.initialize(capabilities).await;
    match method {
        Some(method) => {
            let (id, params) = rig.request(method).await;
            assert_eq!(params["sessionId"], "ses_old");
            assert_eq!(params["cwd"], absolute_cwd());
            assert_eq!(params["mcpServers"], json!([]));
            rig.respond(id, json!({})).await;
            assert_eq!(rig.next().await, session("ses_old"));
            let (prompt, _) = rig.read_prompt().await;
            rig.finish_prompt(prompt, "end_turn").await;
            assert_eq!(rig.until_finished().await, vec![completed()]);
            rig.stop().await;
        }
        None => {
            assert_eq!(
                rig.until_finished().await,
                vec![failed(
                    FailureKind::SessionLost,
                    "The agent cannot resume a session."
                )]
            );
            assert!(rig.peer().reads_eof().await);
            rig.ends().await;
        }
    }
}

#[tokio::test]
async fn resume_answers_with_a_session_id_of_its_own_are_used() {
    let mut rig = Rig::start_with(AcpFlavor::Generic, resuming(), relaxed());
    rig.prompt("hi");
    rig.initialize(RESUME_AND_LOAD).await;
    let (id, _) = rig.request("session/resume").await;
    rig.respond(id, json!({"sessionId": "ses_forked"})).await;
    assert_eq!(rig.next().await, session("ses_forked"));
    let (_, params) = rig.read_prompt().await;
    assert_eq!(params["sessionId"], "ses_forked");
    rig.stop().await;
}

#[tokio::test]
async fn load_drops_what_arrives_before_its_answer() {
    let mut rig = Rig::start_with(AcpFlavor::Generic, resuming(), relaxed());
    rig.prompt("hi");
    rig.initialize(LOAD_ONLY).await;
    let (id, _) = rig.request("session/load").await;
    let chunk = |message: &str, text: &str| {
        json!({
            "sessionUpdate": "agent_message_chunk",
            "messageId": message,
            "content": {"type": "text", "text": text},
        })
    };
    rig.update_in("ses_old", chunk("old", "history")).await;
    rig.update_in("ses_old", json!({"sessionUpdate": "tool_call", "toolCallId": "old-tool", "kind": "execute", "status": "completed", "title": "git log"})).await;
    rig.respond(id, json!({})).await;
    assert_eq!(rig.next().await, session("ses_old"));
    let (prompt, _) = rig.read_prompt().await;
    rig.update_in("ses_old", chunk("new", "fresh")).await;
    rig.finish_prompt(prompt, "end_turn").await;
    assert_eq!(rig.until_finished().await, vec![text("fresh"), completed()]);
    rig.stop().await;
}

#[rstest]
#[case::spec_code(AcpFlavor::Generic, -32002, "Resource not found", Value::Null, FailureKind::SessionLost)]
#[case::text(AcpFlavor::Generic, -32603, "Session not found", Value::Null, FailureKind::SessionLost)]
#[case::opencode_shape(AcpFlavor::OpenCode, -32603, "Internal error: OpenCode service failure", json!({"service": "session"}), FailureKind::SessionLost)]
#[case::the_shape_is_opencode_only(AcpFlavor::Generic, -32603, "Internal error", json!({"service": "session"}), FailureKind::Other)]
#[case::another_error(AcpFlavor::Generic, -32603, "disk is full", Value::Null, FailureKind::Other)]
#[case::login(AcpFlavor::OpenCode, -32000, "Authentication required", Value::Null, FailureKind::NotLoggedIn)]
#[tokio::test]
async fn a_resume_error_is_classified(
    #[case] flavor: AcpFlavor,
    #[case] code: i64,
    #[case] message: &str,
    #[case] data: Value,
    #[case] kind: FailureKind,
) {
    let mut rig = Rig::start_with(flavor, resuming(), relaxed());
    rig.prompt("hi");
    rig.initialize(RESUME_AND_LOAD).await;
    let (id, _) = rig.request("session/resume").await;
    rig.respond_error(id, code, message, data).await;
    assert_eq!(rig.until_finished().await, vec![failed(kind, message)]);
    assert!(rig.no_events().await);
    rig.ends().await;
}

#[tokio::test]
async fn a_silent_resume_is_a_failure_not_a_lost_session() {
    let mut rig = Rig::start_with(AcpFlavor::Generic, resuming(), tight());
    rig.prompt("hi");
    rig.initialize(RESUME_AND_LOAD).await;
    rig.request("session/resume").await;
    let events = rig.until_finished().await;
    let [Event::TurnFinished(crate::Outcome::Failed(failure))] = events.as_slice() else {
        panic!("unexpected {events:?}");
    };
    assert_eq!(failure.kind, FailureKind::Other);
    assert!(
        failure.message.contains("did not answer session/resume"),
        "{}",
        failure.message
    );
    rig.ends().await;
}

#[tokio::test]
async fn a_broken_transport_while_resuming_is_not_a_lost_session() {
    let mut rig = Rig::start_with(AcpFlavor::Generic, resuming(), relaxed());
    rig.prompt("hi");
    rig.initialize(RESUME_AND_LOAD).await;
    rig.request("session/resume").await;
    rig.hang_up().await;
    assert_eq!(
        rig.until_finished().await,
        vec![failed(FailureKind::Other, NOT_READY)]
    );
    rig.ends().await;
}

#[tokio::test]
async fn a_session_that_is_not_ready_fails_each_queued_prompt() {
    let mut rig = Rig::start(AcpFlavor::Generic);
    rig.prompt("one");
    rig.prompt("two");
    rig.initialize(NOTHING).await;
    rig.request("session/new").await;
    rig.hang_up().await;
    assert_eq!(
        rig.until_finished().await,
        vec![failed(FailureKind::Other, NOT_READY)]
    );
    assert_eq!(
        rig.until_finished().await,
        vec![failed(FailureKind::Other, NOT_READY)]
    );
    assert!(rig.no_events().await);
    rig.ends().await;
}

#[tokio::test]
async fn a_session_that_cannot_start_reports_one_failure_even_without_a_prompt() {
    let mut rig = Rig::start(AcpFlavor::Generic);
    rig.initialize(NOTHING).await;
    rig.request("session/new").await;
    rig.hang_up().await;
    assert_eq!(
        rig.until_finished().await,
        vec![failed(FailureKind::Other, NOT_READY)]
    );
    rig.ends().await;
}

fn blocks(params: &Value) -> Vec<String> {
    params["prompt"]
        .as_array()
        .unwrap()
        .iter()
        .map(|block| {
            assert_eq!(block["type"], "text");
            block["text"].as_str().unwrap().to_owned()
        })
        .collect()
}

fn with_instructions(resume: Option<&str>) -> Launch {
    Launch {
        instructions: "Be brief.".into(),
        resume: resume.map(str::to_owned),
        ..launch()
    }
}

#[tokio::test]
async fn without_a_system_prompt_the_first_prompt_of_a_new_session_carries_the_instructions() {
    let mut rig = Rig::start_with(AcpFlavor::Generic, with_instructions(None), relaxed());
    rig.prompt("first");
    rig.initialize(NOTHING).await;
    let new = rig.new_session(json!([])).await;
    assert!(new.get("systemPrompt").is_none());
    let (id, params) = rig.read_prompt().await;
    assert_eq!(blocks(&params), ["Be brief.", "first"]);
    rig.finish_prompt(id, "end_turn").await;
    rig.until_finished().await;
    rig.prompt("second");
    let (id, params) = rig.read_prompt().await;
    assert_eq!(blocks(&params), ["second"]);
    rig.finish_prompt(id, "end_turn").await;
    rig.until_finished().await;
    rig.stop().await;
}

#[tokio::test]
async fn a_failed_first_prompt_sends_the_instructions_again() {
    let mut rig = Rig::start_with(AcpFlavor::Generic, with_instructions(None), relaxed());
    rig.prompt("first");
    rig.ready(NOTHING).await;
    let (id, params) = rig.read_prompt().await;
    assert_eq!(blocks(&params), ["Be brief.", "first"]);
    rig.respond_error(id, -32603, "model overloaded", Value::Null)
        .await;
    assert_eq!(
        rig.until_finished().await,
        vec![
            session(SESSION),
            failed(FailureKind::Other, "model overloaded")
        ]
    );
    rig.prompt("second");
    let (_, params) = rig.read_prompt().await;
    assert_eq!(blocks(&params), ["Be brief.", "second"]);
    rig.stop().await;
}

#[rstest]
#[case::resume(RESUME_AND_LOAD, "session/resume")]
#[case::load(LOAD_ONLY, "session/load")]
#[tokio::test]
async fn a_resumed_session_never_repeats_the_instructions(
    #[case] capabilities: &str,
    #[case] method: &str,
) {
    let mut rig = Rig::start_with(
        AcpFlavor::Generic,
        with_instructions(Some("ses_old")),
        relaxed(),
    );
    rig.prompt("first");
    rig.initialize(capabilities).await;
    let (id, params) = rig.request(method).await;
    assert!(params.get("systemPrompt").is_none());
    rig.respond(id, json!({})).await;
    let (_, params) = rig.read_prompt().await;
    assert_eq!(blocks(&params), ["first"]);
    rig.stop().await;
}

#[tokio::test]
async fn a_declared_system_prompt_takes_the_instructions_on_new_and_on_resume() {
    let mut rig = Rig::start_with(AcpFlavor::Generic, with_instructions(None), relaxed());
    rig.prompt("first");
    rig.initialize(SYSTEM_PROMPT).await;
    let new = rig.new_session(json!([])).await;
    assert_eq!(
        new["systemPrompt"],
        json!({"mode": "append", "text": "Be brief."})
    );
    let (_, params) = rig.read_prompt().await;
    assert_eq!(blocks(&params), ["first"]);
    rig.stop().await;

    let capabilities = r#"{"sessionCapabilities":{"systemPrompt":{},"resume":{}}}"#;
    let mut rig = Rig::start_with(
        AcpFlavor::Generic,
        with_instructions(Some("ses_old")),
        relaxed(),
    );
    rig.prompt("first");
    rig.initialize(capabilities).await;
    let (id, params) = rig.request("session/resume").await;
    assert_eq!(
        params["systemPrompt"],
        json!({"mode": "append", "text": "Be brief."})
    );
    rig.respond(id, json!({})).await;
    let (_, params) = rig.read_prompt().await;
    assert_eq!(blocks(&params), ["first"]);
    rig.stop().await;
}

#[tokio::test]
async fn empty_instructions_are_not_sent() {
    let mut rig = Rig::start(AcpFlavor::Generic);
    rig.prompt("first");
    rig.initialize(SYSTEM_PROMPT).await;
    assert!(
        rig.new_session(json!([]))
            .await
            .get("systemPrompt")
            .is_none()
    );
    let (_, params) = rig.read_prompt().await;
    assert_eq!(blocks(&params), ["first"]);
    rig.stop().await;
}

fn model_option(current: &str) -> Value {
    json!([{
        "id": "model", "name": "Model", "category": "model", "type": "select",
        "currentValue": current,
        "options": [{"value": "a"}, {"value": "b"}, {"value": "[\"x\",\"y\"]"}],
    }])
}

fn mode_option(current: &str, offered: &[&str]) -> Value {
    let values: Vec<Value> = offered
        .iter()
        .map(|value| json!({"value": value}))
        .collect();
    json!({"id": "mode", "name": "Mode", "category": "mode", "type": "select", "currentValue": current, "options": values})
}

fn with_model(model: &str) -> Launch {
    Launch {
        model: Some(model.into()),
        ..launch()
    }
}

#[rstest]
#[case::different("a", "b", true)]
#[case::opaque_json_text("a", "[\"x\",\"y\"]", true)]
#[case::already_current("b", "b", false)]
#[tokio::test]
async fn the_model_is_set_when_it_differs(
    #[case] current: &str,
    #[case] wanted: &str,
    #[case] sets: bool,
) {
    let mut rig = Rig::start_with(AcpFlavor::Generic, with_model(wanted), relaxed());
    rig.prompt("hi");
    rig.initialize(NOTHING).await;
    rig.new_session(model_option(current)).await;
    if sets {
        let (id, params) = rig.request("session/set_config_option").await;
        assert_eq!(
            params,
            json!({"sessionId": SESSION, "configId": "model", "value": wanted})
        );
        rig.respond(id, json!({"configOptions": []})).await;
    }
    assert_eq!(rig.next().await, session(SESSION));
    let (_, params) = rig.read_prompt().await;
    assert_eq!(params["sessionId"], SESSION);
    rig.stop().await;
}

#[tokio::test]
async fn no_model_option_or_no_model_means_no_request() {
    let mut rig = Rig::start_with(AcpFlavor::Generic, with_model("a"), relaxed());
    rig.prompt("hi");
    rig.initialize(NOTHING).await;
    rig.new_session(json!([])).await;
    rig.read_prompt().await;
    rig.stop().await;

    let mut rig = Rig::start(AcpFlavor::Generic);
    rig.prompt("hi");
    rig.initialize(NOTHING).await;
    rig.new_session(model_option("a")).await;
    rig.read_prompt().await;
    rig.stop().await;
}

#[tokio::test]
async fn a_refused_model_fails_the_session_as_model_not_found() {
    let mut rig = Rig::start_with(AcpFlavor::Generic, with_model("zzz"), relaxed());
    rig.prompt("hi");
    rig.initialize(NOTHING).await;
    rig.new_session(model_option("a")).await;
    let (id, _) = rig.request("session/set_config_option").await;
    rig.respond_error(id, -32602, "Invalid value for model", Value::Null)
        .await;
    assert_eq!(
        rig.until_finished().await,
        vec![failed(
            FailureKind::ModelNotFound,
            "Invalid value for model"
        )]
    );
    rig.ends().await;
}

fn modes(current: &str, offered: &[&str]) -> Value {
    json!([model_option("a")[0].clone(), mode_option(current, offered)])
}

#[tokio::test]
async fn opencode_selects_plan_mode_after_the_model_on_a_new_session() {
    let launch = Launch {
        model: Some("b".into()),
        ..launch()
    };
    let mut rig = Rig::start_with(AcpFlavor::OpenCode, launch, relaxed());
    rig.prompt("hi");
    rig.initialize(NOTHING).await;
    rig.new_session(modes("build", &["build", "plan"])).await;
    let (id, params) = rig.request("session/set_config_option").await;
    assert_eq!(params["configId"], "model");
    rig.respond(id, json!({})).await;
    let (id, params) = rig.request("session/set_config_option").await;
    assert_eq!(
        params,
        json!({"sessionId": SESSION, "configId": "mode", "value": "plan"})
    );
    rig.respond(id, json!({})).await;
    assert_eq!(rig.next().await, session(SESSION));
    rig.read_prompt().await;
    rig.stop().await;
}

#[tokio::test]
async fn opencode_selects_plan_mode_on_a_resumed_session_too() {
    let mut rig = Rig::start_with(AcpFlavor::OpenCode, resuming(), relaxed());
    rig.prompt("hi");
    rig.initialize(RESUME_AND_LOAD).await;
    let (id, _) = rig.request("session/resume").await;
    rig.respond(
        id,
        json!({"configOptions": [mode_option("build", &["build", "plan"])]}),
    )
    .await;
    let (_, params) = rig.request("session/set_config_option").await;
    assert_eq!(params["value"], "plan");
    rig.stop().await;
}

#[rstest]
#[case::already_plan(mode_option("plan", &["build", "plan"]))]
#[case::plan_not_offered(mode_option("build", &["build", "review"]))]
#[case::no_mode_option(json!(null))]
#[tokio::test]
async fn no_plan_mode_request_when_it_cannot_help(#[case] mode: Value) {
    let mut rig = Rig::start(AcpFlavor::OpenCode);
    rig.prompt("hi");
    rig.initialize(NOTHING).await;
    let options = if mode.is_null() {
        json!([])
    } else {
        json!([mode])
    };
    rig.new_session(options).await;
    assert_eq!(rig.next().await, session(SESSION));
    rig.read_prompt().await;
    rig.stop().await;
}

#[tokio::test]
async fn other_flavors_never_select_a_mode() {
    for flavor in [AcpFlavor::Generic, AcpFlavor::DeepSeek] {
        let mut rig = Rig::start(flavor);
        rig.prompt("hi");
        rig.initialize(NOTHING).await;
        rig.new_session(modes("build", &["build", "plan"])).await;
        rig.read_prompt().await;
        rig.stop().await;
    }
}

#[tokio::test]
async fn a_refused_plan_mode_does_not_stop_the_session() {
    let mut rig = Rig::start(AcpFlavor::OpenCode);
    rig.prompt("hi");
    rig.initialize(NOTHING).await;
    rig.new_session(json!([mode_option("build", &["build", "plan"])]))
        .await;
    let (id, _) = rig.request("session/set_config_option").await;
    rig.respond_error(id, -32602, "no such mode", Value::Null)
        .await;
    assert_eq!(rig.next().await, session(SESSION));
    rig.read_prompt().await;
    rig.stop().await;
}

#[rstest]
#[case::opencode_code(AcpFlavor::OpenCode, -32000, "Authentication required", FailureKind::NotLoggedIn)]
#[case::auth_required_text(AcpFlavor::Generic, -32603, "auth_required", FailureKind::NotLoggedIn)]
#[case::deepseek(AcpFlavor::DeepSeek, -32603, "MISSING_CREDENTIAL", FailureKind::NotLoggedIn)]
#[case::another_error(AcpFlavor::OpenCode, -32603, "boom", FailureKind::Other)]
#[tokio::test]
async fn a_login_failure_at_session_creation(
    #[case] flavor: AcpFlavor,
    #[case] code: i64,
    #[case] message: &str,
    #[case] kind: FailureKind,
) {
    let mut rig = Rig::start(flavor);
    rig.prompt("hi");
    rig.initialize(NOTHING).await;
    let (id, _) = rig.request("session/new").await;
    rig.respond_error(id, code, message, Value::Null).await;
    assert_eq!(rig.until_finished().await, vec![failed(kind, message)]);
    rig.ends().await;
}

#[tokio::test]
async fn a_session_without_an_id_is_a_failure() {
    let mut rig = Rig::start(AcpFlavor::Generic);
    rig.prompt("hi");
    rig.initialize(NOTHING).await;
    let (id, _) = rig.request("session/new").await;
    rig.respond(id, json!({})).await;
    assert_eq!(
        rig.until_finished().await,
        vec![failed(
            FailureKind::Other,
            "The agent did not return a session id."
        )]
    );
    rig.ends().await;
}

#[rstest]
#[case::end_turn("end_turn", completed())]
#[case::max_tokens("max_tokens", completed())]
#[case::max_turn_requests("max_turn_requests", completed())]
#[case::cancelled("cancelled", interrupted())]
#[case::refusal(
    "refusal",
    failed(FailureKind::Other, "The assistant declined to answer.")
)]
#[tokio::test]
async fn the_stop_reason_ends_the_turn(#[case] reason: &str, #[case] expected: Event) {
    let (mut rig, prompt) = running_turn(AcpFlavor::Generic, NOTHING).await;
    rig.finish_prompt(prompt, reason).await;
    assert_eq!(rig.until_finished().await, vec![session(SESSION), expected]);
    rig.stop().await;
}

#[tokio::test]
async fn a_failed_prompt_fails_the_turn_and_the_session_goes_on() {
    let (mut rig, prompt) = running_turn(AcpFlavor::OpenCode, NOTHING).await;
    rig.respond_error(prompt, -32603, "auth_required", Value::Null)
        .await;
    assert_eq!(
        rig.until_finished().await,
        vec![
            session(SESSION),
            failed(FailureKind::NotLoggedIn, "auth_required")
        ]
    );
    rig.prompt("again");
    let (prompt, _) = rig.read_prompt().await;
    rig.finish_prompt(prompt, "end_turn").await;
    assert_eq!(rig.until_finished().await, vec![completed()]);
    rig.stop().await;
}

#[tokio::test]
async fn a_prompt_during_a_turn_waits_for_it() {
    let (mut rig, first) = running_turn(AcpFlavor::Generic, NOTHING).await;
    rig.prompt("second");
    rig.prompt("third");
    assert!(rig.peer().reads_nothing().await);
    rig.finish_prompt(first, "end_turn").await;
    let (second, params) = rig.read_prompt().await;
    assert_eq!(blocks(&params), ["second"]);
    assert!(rig.peer().reads_nothing().await);
    rig.finish_prompt(second, "end_turn").await;
    let (third, params) = rig.read_prompt().await;
    assert_eq!(blocks(&params), ["third"]);
    rig.finish_prompt(third, "end_turn").await;
    assert_eq!(
        rig.until_finished().await,
        vec![session(SESSION), completed()]
    );
    assert_eq!(rig.until_finished().await, vec![completed()]);
    assert_eq!(rig.until_finished().await, vec![completed()]);
    rig.stop().await;
}

#[tokio::test]
async fn an_interrupt_without_a_turn_does_nothing() {
    let mut rig = Rig::start(AcpFlavor::Generic);
    rig.ready(NOTHING).await;
    assert_eq!(rig.next().await, session(SESSION));
    rig.send(Command::Interrupt);
    assert!(rig.peer().reads_nothing().await);
    assert!(rig.no_events().await);
    rig.prompt("hi");
    let (prompt, _) = rig.read_prompt().await;
    rig.finish_prompt(prompt, "end_turn").await;
    assert_eq!(rig.until_finished().await, vec![completed()]);
    rig.stop().await;
}

#[tokio::test]
async fn an_interrupt_cancels_the_turn_and_queued_prompts_survive() {
    let (mut rig, first) = running_turn(AcpFlavor::Generic, NOTHING).await;
    rig.prompt("second");
    rig.send(Command::Interrupt);
    rig.send(Command::Interrupt);
    let cancel = rig.notification("session/cancel").await;
    assert_eq!(cancel, json!({"sessionId": SESSION}));
    assert!(rig.peer().reads_nothing().await);
    rig.finish_prompt(first, "cancelled").await;
    assert_eq!(
        rig.until_finished().await,
        vec![session(SESSION), interrupted()]
    );
    let (second, params) = rig.read_prompt().await;
    assert_eq!(blocks(&params), ["second"]);
    rig.finish_prompt(second, "end_turn").await;
    assert_eq!(rig.until_finished().await, vec![completed()]);
    rig.stop().await;
}

#[tokio::test]
async fn the_agents_note_about_the_cancellation_is_not_shown() {
    let (mut rig, prompt) = running_turn(AcpFlavor::Generic, NOTHING).await;
    rig.say("m", "Working on it. ").await;
    rig.send(Command::Interrupt);
    rig.notification("session/cancel").await;
    rig.say("n", "The request was cancelled by the client.")
        .await;
    rig.finish_prompt(prompt, "cancelled").await;
    assert_eq!(
        rig.until_finished().await,
        vec![session(SESSION), text("Working on it. "), interrupted()]
    );
    rig.stop().await;
}

#[tokio::test]
async fn an_unanswered_cancel_closes_the_session_after_the_limit() {
    let (mut rig, _first) = running_tight(NOTHING).await;
    rig.prompt("second");
    rig.tool("t", None, "in_progress", "sleep 40").await;
    rig.send(Command::Interrupt);
    rig.notification("session/cancel").await;
    let events = rig.until_finished().await;
    assert_eq!(events[0], session(SESSION));
    assert_eq!(events[1], tool_started("t", "sleep 40"));
    assert!(matches!(&events[2], Event::Notice(_)), "{events:?}");
    assert_eq!(events[3], tool_finished("t", true));
    assert_eq!(events[4], interrupted());
    assert_eq!(
        rig.until_finished().await,
        vec![failed(FailureKind::Other, STOPPED)]
    );
    rig.ends().await;
}

#[tokio::test]
async fn a_silent_agent_is_given_up_on() {
    let mut rig = Rig::start_with(AcpFlavor::Generic, launch(), tight());
    rig.prompt("hi");
    rig.prompt("queued");
    rig.ready(NOTHING).await;
    rig.read_prompt().await;
    assert_eq!(
        rig.until_finished().await,
        vec![session(SESSION), failed(FailureKind::Other, SILENT)]
    );
    assert_eq!(
        rig.until_finished().await,
        vec![failed(FailureKind::Other, SILENT)]
    );
    rig.ends().await;
}

#[rstest]
#[case::running("in_progress")]
#[case::pending("pending")]
#[tokio::test]
async fn an_open_tool_call_keeps_the_watchdog_quiet(#[case] status: &str) {
    let (mut rig, prompt) = running_tight(NOTHING).await;
    rig.tool("t", Some("execute"), status, "sleep 1").await;
    tokio::time::sleep(Duration::from_millis(1000)).await;
    rig.tool_update("t", "completed", "sleep 1").await;
    rig.finish_prompt(prompt, "end_turn").await;
    let events = rig.until_finished().await;
    assert_eq!(events.last(), Some(&completed()));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, Event::TurnFinished(crate::Outcome::Failed(_))))
    );
    rig.stop().await;
}

#[tokio::test]
async fn the_watchdog_starts_again_once_the_tool_call_is_done() {
    let (mut rig, _prompt) = running_tight(NOTHING).await;
    rig.tool("t", None, "in_progress", "ls").await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    rig.tool_update("t", "completed", "ls").await;
    let events = rig.until_finished().await;
    assert_eq!(
        events,
        vec![
            session(SESSION),
            tool_started("t", "ls"),
            tool_finished("t", false),
            failed(FailureKind::Other, SILENT),
        ]
    );
    rig.ends().await;
}

#[tokio::test]
async fn any_message_resets_the_watchdog() {
    let limits = Limits {
        idle: Duration::from_millis(1000),
        ..tight()
    };
    let (mut rig, prompt) = running_turn_within(AcpFlavor::Generic, NOTHING, limits).await;
    for _ in 0..8 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        rig.update(json!({"sessionUpdate": "usage_update", "used": 1, "size": 2}))
            .await;
    }
    rig.finish_prompt(prompt, "end_turn").await;
    assert_eq!(
        rig.until_finished().await,
        vec![session(SESSION), completed()]
    );
    rig.stop().await;
}

#[tokio::test]
async fn the_watchdog_is_off_between_turns() {
    let mut rig = Rig::start_with(AcpFlavor::Generic, launch(), tight());
    rig.ready(NOTHING).await;
    assert_eq!(rig.next().await, session(SESSION));
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(rig.no_events().await);
    rig.stop().await;
}

#[tokio::test]
async fn an_update_without_a_call_is_a_started_tool() {
    let (mut rig, prompt) = running_turn(AcpFlavor::Generic, NOTHING).await;
    rig.tool_update("t", "in_progress", "cat lib.rs").await;
    rig.tool_update("t", "in_progress", "cat lib.rs").await;
    rig.tool_update("t", "completed", "cat lib.rs").await;
    rig.tool_update("u", "failed", "rm -rf x").await;
    rig.finish_prompt(prompt, "end_turn").await;
    assert_eq!(
        rig.until_finished().await,
        vec![
            session(SESSION),
            tool_started("t", "cat lib.rs"),
            tool_finished("t", false),
            tool_started("u", "rm -rf x"),
            tool_finished("u", true),
            completed(),
        ]
    );
    rig.stop().await;
}

#[tokio::test]
async fn repeated_updates_start_a_tool_once() {
    let (mut rig, prompt) = running_turn(AcpFlavor::OpenCode, NOTHING).await;
    rig.tool("t", Some("execute"), "pending", "bash").await;
    for _ in 0..4 {
        rig.tool_update("t", "in_progress", "git log --oneline")
            .await;
    }
    rig.tool_update("t", "completed", "git log --oneline").await;
    rig.finish_prompt(prompt, "end_turn").await;
    assert_eq!(
        rig.until_finished().await,
        vec![
            session(SESSION),
            tool_started("t", "git log --oneline"),
            tool_finished("t", false),
            completed(),
        ]
    );
    rig.stop().await;
}

#[tokio::test]
async fn everything_before_the_answer_arrives_before_the_turn_ends() {
    let (mut rig, mut prompt) = running_turn(AcpFlavor::Generic, NOTHING).await;
    for turn in 0..20 {
        let mut burst = String::new();
        for chunk in 0..25 {
            burst.push_str(
                &json!({
                    "jsonrpc": "2.0",
                    "method": "session/update",
                    "params": {"sessionId": SESSION, "update": {
                        "sessionUpdate": "agent_message_chunk",
                        "messageId": "m",
                        "content": {"type": "text", "text": format!("{chunk},")},
                    }},
                })
                .to_string(),
            );
            burst.push('\n');
        }
        burst.push_str(
            &json!({"jsonrpc": "2.0", "id": prompt, "result": {"stopReason": "end_turn"}})
                .to_string(),
        );
        rig.peer().write_raw(&burst).await;
        let events = rig.until_finished().await;
        let expected: String = (0..25).map(|chunk| format!("{chunk},")).collect();
        let said: String = events
            .iter()
            .filter_map(|event| match event {
                Event::Text(text) => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(said, expected, "turn {turn}");
        assert_eq!(events.last(), Some(&completed()));
        rig.prompt("next");
        prompt = rig.read_prompt().await.0;
    }
    rig.stop().await;
}

#[tokio::test]
async fn names_with_an_underscore_and_unknown_ones_are_ignored() {
    let (mut rig, prompt) = running_turn(AcpFlavor::Generic, NOTHING).await;
    rig.write(json!({"jsonrpc": "2.0", "method": "_vendor/progress", "params": {"n": 1}}))
        .await;
    rig.write(json!({"jsonrpc": "2.0", "method": "session/teleport", "params": {}}))
        .await;
    rig.update(json!({"sessionUpdate": "_vendor/update", "x": 1}))
        .await;
    rig.update(json!({"sessionUpdate": "plan", "entries": [{"content": "x"}]}))
        .await;
    rig.update(json!({"sessionUpdate": "session_info_update", "title": "t"}))
        .await;
    rig.update(json!({"sessionUpdate": "current_mode_update", "currentModeId": "plan"}))
        .await;
    rig.peer().write_raw("this is not json").await;
    rig.say("m", "ok").await;
    rig.finish_prompt(prompt, "end_turn").await;
    assert_eq!(
        rig.until_finished().await,
        vec![session(SESSION), text("ok"), completed()]
    );
    rig.stop().await;
}

#[tokio::test]
async fn the_end_of_output_fails_the_running_turn_and_every_queued_prompt() {
    let (mut rig, _prompt) = running_turn(AcpFlavor::Generic, NOTHING).await;
    rig.prompt("two");
    rig.prompt("three");
    rig.tool("t", None, "in_progress", "ls").await;
    rig.say("m", "partial").await;
    rig.hang_up().await;
    let mut events = Vec::new();
    for _ in 0..3 {
        events.extend(rig.until_finished().await);
    }
    assert_eq!(
        events,
        vec![
            session(SESSION),
            tool_started("t", "ls"),
            text("partial"),
            tool_finished("t", true),
            failed(FailureKind::Other, STOPPED),
            failed(FailureKind::Other, STOPPED),
            failed(FailureKind::Other, STOPPED),
        ]
    );
    assert!(rig.no_events().await);
    rig.ends().await;
}

#[tokio::test]
async fn a_prompt_sent_just_before_the_end_still_gets_its_answer() {
    for _ in 0..20 {
        let mut rig = Rig::start(AcpFlavor::Generic);
        rig.ready(NOTHING).await;
        assert_eq!(rig.next().await, session(SESSION));
        rig.hang_up().await;
        rig.prompt("late");
        rig.prompt("later");
        let mut finished = 0;
        while finished < 2 {
            if matches!(rig.next().await, Event::TurnFinished(_)) {
                finished += 1;
            }
        }
        rig.ends().await;
    }
}

#[tokio::test]
async fn the_end_of_output_between_turns_emits_nothing() {
    let mut rig = Rig::start(AcpFlavor::Generic);
    rig.ready(NOTHING).await;
    assert_eq!(rig.next().await, session(SESSION));
    rig.hang_up().await;
    assert!(rig.no_events().await);
    rig.ends().await;
}

#[tokio::test]
async fn a_shutdown_returns_without_further_events() {
    let (mut rig, _prompt) = running_turn(AcpFlavor::Generic, NOTHING).await;
    rig.prompt("queued");
    rig.tool("t", None, "in_progress", "ls").await;
    assert_eq!(rig.next().await, session(SESSION));
    assert_eq!(rig.next().await, tool_started("t", "ls"));
    rig.send(Command::Shutdown);
    let events = rig.events.clone();
    rig.ends().await;
    assert!(events.try_recv().is_err());
    assert!(!matches!(events.try_recv(), Ok(Event::Exited(_))));
}

#[tokio::test]
async fn a_shutdown_during_setup_returns_without_events() {
    let mut rig = Rig::start(AcpFlavor::Generic);
    rig.initialize(NOTHING).await;
    rig.request("session/new").await;
    rig.send(Command::Shutdown);
    let events = rig.events.clone();
    rig.ends().await;
    assert!(events.try_recv().is_err());
}

#[tokio::test]
async fn dropping_the_command_channel_ends_the_driver() {
    let mut rig = Rig::start(AcpFlavor::Generic);
    rig.ready(NOTHING).await;
    assert_eq!(rig.next().await, session(SESSION));
    let Rig {
        commands, driver, ..
    } = rig;
    drop(commands);
    tokio::time::timeout(WAIT, driver).await.unwrap().unwrap();
}

#[tokio::test]
async fn closing_the_driver_closes_the_agents_input() {
    let mut rig = Rig::start(AcpFlavor::Generic);
    rig.ready(NOTHING).await;
    rig.send(Command::Shutdown);
    assert!(rig.peer().reads_eof().await);
}

#[tokio::test]
async fn limits_default_to_the_documented_durations() {
    let limits = Limits::default();
    assert_eq!(limits.idle, Duration::from_secs(120));
    assert_eq!(limits.cancel, Duration::from_secs(12));
    assert_eq!(limits.load, Duration::from_secs(90));
}

#[tokio::test]
async fn an_update_for_another_session_is_ignored() {
    let (mut rig, prompt) = running_turn(AcpFlavor::Generic, NOTHING).await;
    rig.update_in(
        "child",
        json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "not ours"}}),
    )
    .await;
    rig.say("m", "ours").await;
    rig.finish_prompt(prompt, "end_turn").await;
    assert_eq!(
        rig.until_finished().await,
        vec![session(SESSION), text("ours"), completed()]
    );
    rig.stop().await;
}
