use std::ffi::OsString;

use serde_json::{Value, json};
use tokio::io::{AsyncWriteExt, duplex, split};

use crate::catalog::{
    Model, acp_models, claude_models, codex_models, opencode_models, parse_acp, parse_claude,
    parse_codex_page, parse_pi, pi_models,
};
use crate::lines::Lines;
use crate::{HarnessKind, Launch};

const CLAUDE_MODELS: &str = include_str!("../fixtures/claude/models.jsonl");

fn model(id: &str, name: &str, description: Option<&str>, is_default: bool) -> Model {
    Model {
        id: id.to_owned(),
        name: name.to_owned(),
        description: description.map(str::to_owned),
        is_default,
    }
}

fn launch() -> Launch {
    Launch {
        kind: HarnessKind::Acp(crate::AcpFlavor::Generic),
        command: "agent".into(),
        cwd: std::env::temp_dir(),
        model: None,
        resume: None,
        history: false,
        instructions: String::new(),
        scratch: Default::default(),
    }
}

fn recorded_reply() -> Value {
    let last = CLAUDE_MODELS.lines().last().unwrap();
    serde_json::from_str(last).unwrap()
}

#[test]
fn the_recorded_claude_reply_gives_ids_names_and_the_default() {
    let reply = recorded_reply();
    let models = parse_claude(&reply["response"]["response"]);
    let ids: Vec<&str> = models.iter().map(|model| model.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "opus",
            "fable",
            "sonnet",
            "haiku",
            "claude-fable-5-1",
            "claude-haiku-4-5-20251001",
            "claude-sonnet-5",
            "claude-opus-5",
            "claude-fable-5",
            "claude-opus-4-8",
            "claude-opus-4-7",
            "claude-opus-4-6",
            "claude-sonnet-4-6",
        ]
    );
    assert!(!ids.contains(&"default"));
    let opus = models.iter().find(|model| model.id == "opus").unwrap();
    assert_eq!(
        opus,
        &model(
            "opus",
            "Opus 5.5",
            Some("For complex work and everyday tasks"),
            true
        )
    );
    assert_eq!(models.iter().filter(|model| model.is_default).count(), 1);
}

#[test]
fn a_claude_reply_without_models_is_empty() {
    assert!(parse_claude(&json!({"commands": []})).is_empty());
    assert!(parse_claude(&json!({"models": "none"})).is_empty());
}

#[test]
fn claude_entries_without_a_value_are_skipped_and_a_name_falls_back_to_the_value() {
    let reply = json!({"models": [
        {"displayName": "Nameless"},
        {"value": "x", "description": "  "},
    ]});
    assert_eq!(parse_claude(&reply), [model("x", "x", None, false)]);
}

async fn peer_answers_claude(reply: String) -> Result<Vec<Model>, String> {
    let (ours, theirs) = duplex(1 << 20);
    let (reader, writer) = split(ours);
    let (peer_reader, mut peer_writer) = split(theirs);
    let peer = tokio::spawn(async move {
        let mut lines = Lines::new(peer_reader);
        let request: Value =
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(request["type"], "control_request");
        assert_eq!(request["request"]["subtype"], "initialize");
        assert_eq!(request["request_id"], "init-1");
        peer_writer.write_all(reply.as_bytes()).await.unwrap();
        peer_writer.shutdown().await.unwrap();
    });
    let result = claude_models(reader, writer).await;
    peer.await.unwrap();
    result
}

#[tokio::test]
async fn claude_is_asked_with_an_initialize_request_and_hook_frames_are_skipped() {
    let models = peer_answers_claude(CLAUDE_MODELS.to_owned()).await.unwrap();
    assert!(models.iter().any(|model| model.id == "sonnet"));
}

#[tokio::test]
async fn claude_answers_to_other_requests_are_ignored() {
    let other = json!({"type": "control_response", "response": {"subtype": "success", "request_id": "other", "response": {"models": [{"value": "wrong"}]}}});
    let reply = format!("{other}\n{CLAUDE_MODELS}");
    let models = peer_answers_claude(reply).await.unwrap();
    assert!(models.iter().all(|model| model.id != "wrong"));
}

#[tokio::test]
async fn a_claude_refusal_and_an_early_exit_are_errors() {
    let refusal = json!({"type": "control_response", "response": {"subtype": "error", "request_id": "init-1", "error": "not ready"}});
    assert_eq!(
        peer_answers_claude(format!("{refusal}\n"))
            .await
            .unwrap_err(),
        "not ready"
    );
    assert!(peer_answers_claude(String::new()).await.is_err());
}

#[test]
fn opencode_verbose_output_gives_provider_slash_id_and_the_name() {
    let output = "\
opencode/big-pickle
{
  \"id\": \"big-pickle\",
  \"providerID\": \"opencode\",
  \"name\": \"Big Pickle\",
  \"limit\": {
    \"context\": 1
  }
}
deepseek/deepseek-flash
{
  \"id\": \"deepseek-flash\",
  \"providerID\": \"deepseek\",
  \"name\": \"DeepSeek V4.1 Flash\"
}
";
    assert_eq!(
        opencode_models(output),
        [
            model("opencode/big-pickle", "Big Pickle", Some("opencode"), false),
            model(
                "deepseek/deepseek-flash",
                "DeepSeek V4.1 Flash",
                Some("deepseek"),
                false
            ),
        ]
    );
}

#[test]
fn opencode_plain_output_is_the_fallback() {
    let output = "opencode/a\nnot a model line\nllama-cpp/qwen\n";
    assert_eq!(
        opencode_models(output),
        [
            model("opencode/a", "opencode/a", None, false),
            model("llama-cpp/qwen", "llama-cpp/qwen", None, false),
        ]
    );
    assert!(opencode_models("").is_empty());
}

fn config_options_reply() -> Value {
    json!({
        "sessionId": "s",
        "configOptions": [
            {"id": "mode", "category": "mode", "options": [{"value": "build"}]},
            {"id": "model", "category": "model", "currentValue": "b/two", "options": [
                {"value": "a/one", "name": "One"},
                {"group": "g", "options": [
                    {"value": "b/two", "name": "Two", "description": "second"}
                ]}
            ]}
        ],
        "models": {
            "currentModelId": "unused",
            "availableModels": [{"modelId": "unused", "name": "Unused"}]
        }
    })
}

#[test]
fn acp_config_options_win_and_groups_are_flattened() {
    assert_eq!(
        parse_acp(&config_options_reply()),
        [
            model("a/one", "One", None, false),
            model("b/two", "Two", Some("second"), true),
        ]
    );
}

#[test]
fn acp_falls_back_to_the_unstable_model_list() {
    let reply = json!({
        "sessionId": "s",
        "models": {
            "currentModelId": "m2",
            "availableModels": [
                {"modelId": "m1", "name": "M1"},
                {"modelId": "m2", "name": "M2", "description": "d"},
                {"name": "no id"}
            ]
        }
    });
    assert_eq!(
        parse_acp(&reply),
        [
            model("m1", "M1", None, false),
            model("m2", "M2", Some("d"), true)
        ]
    );
}

#[test]
fn acp_without_either_list_is_empty() {
    assert!(parse_acp(&json!({"sessionId": "s"})).is_empty());
    assert!(parse_acp(&json!({"configOptions": [{"id": "mode", "category": "mode"}]})).is_empty());
}

async fn answer(
    lines: &mut Lines<tokio::io::ReadHalf<tokio::io::DuplexStream>>,
    writer: &mut tokio::io::WriteHalf<tokio::io::DuplexStream>,
    result: Value,
) -> Value {
    let request: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
    let reply = json!({"jsonrpc": "2.0", "id": request["id"], "result": result});
    writer
        .write_all(format!("{reply}\n").as_bytes())
        .await
        .unwrap();
    request
}

#[tokio::test]
async fn an_acp_agent_is_asked_for_a_new_session_and_nothing_else() {
    let (ours, theirs) = duplex(1 << 20);
    let (reader, writer) = split(ours);
    let (peer_reader, mut peer_writer) = split(theirs);
    let peer = tokio::spawn(async move {
        let mut lines = Lines::new(peer_reader);
        let init = answer(&mut lines, &mut peer_writer, json!({"protocolVersion": 1})).await;
        assert_eq!(init["method"], "initialize");
        let new = answer(&mut lines, &mut peer_writer, config_options_reply()).await;
        assert_eq!(new["method"], "session/new");
        assert_eq!(new["params"]["mcpServers"], json!([]));
        assert!(new["params"].get("systemPrompt").is_none());
        assert!(lines.next_line().await.unwrap().is_none());
    });
    let models = acp_models(reader, writer, &launch()).await.unwrap();
    assert_eq!(models.len(), 2);
    peer.await.unwrap();
}

#[test]
fn codex_pages_skip_hidden_models_and_name_the_value_to_pass_back() {
    let page = json!({
        "data": [
            {"id": "gpt-x", "model": "gpt-x", "displayName": "GPT X", "description": "d", "hidden": false, "isDefault": true},
            {"id": "old", "model": "old", "hidden": true},
            {"id": "id-only"}
        ],
        "nextCursor": "c2"
    });
    let (models, next) = parse_codex_page(&page);
    assert_eq!(
        models,
        [
            model("gpt-x", "GPT X", Some("d"), true),
            model("id-only", "id-only", None, false)
        ]
    );
    assert_eq!(next.as_deref(), Some("c2"));
    assert_eq!(parse_codex_page(&json!({"data": []})).1, None);
}

#[tokio::test]
async fn codex_is_paged_until_the_cursor_ends() {
    let (ours, theirs) = duplex(1 << 20);
    let (reader, writer) = split(ours);
    let (peer_reader, mut peer_writer) = split(theirs);
    let peer = tokio::spawn(async move {
        let mut lines = Lines::new(peer_reader);
        let init = answer(
            &mut lines,
            &mut peer_writer,
            json!({"userAgent": "codex/1"}),
        )
        .await;
        assert_eq!(init["method"], "initialize");
        let notice: Value =
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(notice["method"], "initialized");
        let first = answer(
            &mut lines,
            &mut peer_writer,
            json!({"data": [{"model": "a"}], "nextCursor": "next"}),
        )
        .await;
        assert_eq!(first["method"], "model/list");
        assert_eq!(first["params"], json!({}));
        let second = answer(
            &mut lines,
            &mut peer_writer,
            json!({"data": [{"model": "b"}]}),
        )
        .await;
        assert_eq!(second["params"], json!({"cursor": "next"}));
    });
    let models = codex_models(reader, writer).await.unwrap();
    let ids: Vec<&str> = models.iter().map(|model| model.id.as_str()).collect();
    assert_eq!(ids, ["a", "b"]);
    peer.await.unwrap();
}

#[test]
fn pi_models_are_stored_as_provider_slash_id() {
    let data = json!({"models": [
        {"id": "sonnet", "name": "Sonnet", "provider": "anthropic"},
        {"id": "local"},
        {"name": "no id"}
    ]});
    assert_eq!(
        parse_pi(&data),
        [
            model("anthropic/sonnet", "Sonnet", Some("anthropic"), false),
            model("local", "local", None, false),
        ]
    );
}

#[tokio::test]
async fn pi_is_asked_for_its_available_models() {
    let (ours, theirs) = duplex(1 << 20);
    let (reader, writer) = split(ours);
    let (peer_reader, mut peer_writer) = split(theirs);
    let peer = tokio::spawn(async move {
        let mut lines = Lines::new(peer_reader);
        let request: Value =
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(request["type"], "get_available_models");
        let noise = json!({"type": "response", "command": "get_state", "success": true});
        let reply = json!({"type": "response", "command": "get_available_models", "id": request["id"], "success": true, "data": {"models": [{"id": "m", "provider": "p"}]}});
        peer_writer
            .write_all(format!("not json\n{noise}\n{reply}\n").as_bytes())
            .await
            .unwrap();
    });
    let models = pi_models(reader, writer).await.unwrap();
    assert_eq!(models, [model("p/m", "m", Some("p"), false)]);
    peer.await.unwrap();
}

#[cfg(unix)]
mod process {
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    use super::*;
    use crate::process::Environment;
    use crate::{AcpFlavor, Error};

    fn environment() -> Environment {
        Environment::merge(
            vec![(OsString::from("PATH"), OsString::from("/usr/bin:/bin"))],
            &[],
            None,
        )
    }

    fn script(dir: &Path, name: &str, body: &str) -> String {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[tokio::test]
    async fn a_claude_process_is_asked_in_an_empty_folder_that_is_removed_afterwards() {
        let dir = tempfile::tempdir().unwrap();
        let fixture = dir.path().join("models.jsonl");
        std::fs::write(&fixture, CLAUDE_MODELS).unwrap();
        let seen = dir.path().join("cwd.txt");
        let modes = dir.path().join("modes.txt");
        let command = script(
            dir.path(),
            "claude",
            &format!(
                "pwd > {}\nls -ld .. > {}\nread request\ncase \"$request\" in *initialize*) ;; *) exit 3;; esac\n[ -z \"$(ls -A .)\" ] || exit 4\n/bin/cat {}\nsleep 5",
                seen.display(),
                modes.display(),
                fixture.display()
            ),
        );
        let models = crate::catalog::models_in(HarnessKind::Claude, &command, Some(environment()))
            .await
            .unwrap();
        assert!(models.iter().any(|model| model.id == "haiku"));
        let cwd = std::fs::read_to_string(&seen).unwrap();
        assert!(cwd.contains("nitpick-models-"));
        assert!(!std::path::Path::new(cwd.trim()).exists());
        let modes = std::fs::read_to_string(&modes).unwrap();
        assert!(modes.starts_with("drwx------"), "{modes}");
        let name = std::path::Path::new(cwd.trim())
            .parent()
            .and_then(|parent| parent.file_name())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(name.starts_with("nitpick-models-") && name.len() > "nitpick-models-".len() + 4);
    }

    #[tokio::test]
    async fn opencode_is_asked_through_its_models_command_without_the_acp_argument() {
        let dir = tempfile::tempdir().unwrap();
        let command = script(
            dir.path(),
            "opencode",
            "[ \"$1 $2\" = \"models --verbose\" ] || exit 3\nprintf 'p/m\\n{\\n  \"id\": \"m\",\\n  \"providerID\": \"p\",\\n  \"name\": \"M\"\\n}\\n'",
        );
        let models = crate::catalog::models_in(
            HarnessKind::Acp(AcpFlavor::OpenCode),
            &format!("{command} acp"),
            Some(environment()),
        )
        .await
        .unwrap();
        assert_eq!(models, [model("p/m", "M", Some("p"), false)]);
    }

    #[tokio::test]
    async fn a_failing_command_is_reported_with_its_error_line() {
        let dir = tempfile::tempdir().unwrap();
        let command = script(dir.path(), "opencode", "echo 'boom' >&2\nexit 1");
        let error = crate::catalog::models_in(
            HarnessKind::Acp(AcpFlavor::OpenCode),
            &command,
            Some(environment()),
        )
        .await
        .unwrap_err();
        assert!(matches!(error, Error::Probe { ref detail, .. } if detail == "boom"));
    }

    #[tokio::test]
    async fn a_missing_program_is_not_found() {
        let error = crate::catalog::models_in(
            HarnessKind::Claude,
            "no-such-agent-program",
            Some(environment()),
        )
        .await
        .unwrap_err();
        assert!(matches!(error, Error::NotFound(_)));
    }
}
