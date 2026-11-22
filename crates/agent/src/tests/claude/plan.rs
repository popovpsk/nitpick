use std::path::{Path, PathBuf};
use std::sync::Arc;

use rstest::rstest;
use serde_json::Value;

use crate::claude::{
    ClaudeDriver, Os, SessionArg, arguments, config_dir, environment, session_file_exists,
    settings_json,
};
use crate::process::Environment;
use crate::registry::Registry;
use crate::session::{Driver, start_in};
use crate::{Event, FailureKind, HarnessKind, Launch, Outcome};

const ID: &str = "951f14f9-9400-4688-a95a-733f42893977";

fn launch() -> Launch {
    Launch {
        kind: HarnessKind::Claude,
        command: "claude".into(),
        cwd: PathBuf::from("/work/toy"),
        model: None,
        resume: None,
        instructions: "Be brief.".into(),
        scratch: PathBuf::from("/scratch"),
    }
}

fn scratch_path(name: &str) -> String {
    PathBuf::from("/scratch")
        .join(name)
        .to_string_lossy()
        .into_owned()
}

fn common() -> Vec<String> {
    let settings = scratch_path("settings.json");
    let instructions = scratch_path("instructions.md");
    [
        "--print",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--tools",
        "Bash,Read,Grep,Glob",
        "--allowedTools",
        "Bash,Read,Grep,Glob",
        "--disallowedTools",
        "Agent,Task,Workflow,AskUserQuestion,ExitPlanMode",
        "--permission-mode",
        "dontAsk",
        "--setting-sources",
        "user",
        "--strict-mcp-config",
        "--disable-slash-commands",
        "--settings",
        &settings,
        "--append-system-prompt-file",
        &instructions,
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

#[test]
fn a_new_session_gets_the_fixed_flags_and_its_own_id() {
    let args = arguments(&launch(), &SessionArg::New(ID.into()));
    let mut expected = common();
    expected.extend(["--session-id".into(), ID.into()]);
    assert_eq!(args, expected);
}

#[test]
fn a_resumed_session_repeats_every_flag() {
    let args = arguments(&launch(), &SessionArg::Resume(ID.into()));
    let mut expected = common();
    expected.push(format!("--resume={ID}"));
    assert_eq!(args, expected);
}

#[rstest]
#[case::set(Some("claude-haiku-5-5"), true)]
#[case::empty(Some(""), false)]
#[case::none(None, false)]
fn the_model_is_passed_only_when_set(#[case] model: Option<&str>, #[case] passed: bool) {
    let launch = Launch {
        model: model.map(str::to_owned),
        ..launch()
    };
    let args = arguments(&launch, &SessionArg::New(ID.into()));
    let position = args.iter().position(|arg| arg == "--model");
    assert_eq!(position.is_some(), passed);
    if let Some(position) = position {
        assert_eq!(args[position + 1], model.unwrap());
        assert_eq!(args[position + 2], "--session-id");
    }
}

#[test]
fn unix_settings_sandbox_the_cwd_read_only() {
    let settings: Value = serde_json::from_str(&settings_json(Path::new("/work/toy"), Os::Unix))
        .expect("settings are JSON");
    assert_eq!(settings["disableAllHooks"], true);
    assert_eq!(settings["sandbox"]["enabled"], true);
    assert_eq!(settings["sandbox"]["allowUnsandboxedCommands"], false);
    assert_eq!(
        settings["sandbox"]["filesystem"]["denyWrite"],
        serde_json::json!([Path::new("/work/toy").to_string_lossy()])
    );
}

#[test]
fn windows_settings_have_no_sandbox_block() {
    let settings: Value =
        serde_json::from_str(&settings_json(Path::new(r"C:\work\toy"), Os::Windows))
            .expect("settings are JSON");
    assert_eq!(settings, serde_json::json!({"disableAllHooks": true}));
}

#[test]
fn windows_arguments_carry_paths_only() {
    let launch = Launch {
        scratch: PathBuf::from(r"C:\Users\me\scratch"),
        instructions: "{\"long\": \"text\"}\nsecond line".into(),
        ..launch()
    };
    let args = arguments(&launch, &SessionArg::Resume(ID.into()));
    assert!(
        args.iter()
            .all(|arg| !arg.contains('{') && !arg.contains('\n'))
    );
    assert!(
        args.iter()
            .any(|arg| arg.starts_with(r"C:\Users\me\scratch"))
    );
}

#[test]
fn the_environment_turns_off_background_work() {
    let env = environment();
    assert!(env.iter().all(|(_, value)| value == "1"));
    for name in [
        "CLAUDE_CODE_DISABLE_BACKGROUND_TASKS",
        "CLAUDE_CODE_DISABLE_AUTO_MEMORY",
        "CLAUDE_CODE_DISABLE_TERMINAL_TITLE",
    ] {
        assert!(env.iter().any(|(key, _)| key == name), "{name}");
    }
}

fn driver(os: Os, known: &'static [&'static str]) -> ClaudeDriver {
    ClaudeDriver::with_lookup(Arc::new(move |id| known.contains(&id))).with_os(os)
}

fn arg_after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    let position = args.iter().position(|arg| arg == flag)?;
    args.get(position + 1).map(String::as_str)
}

#[test]
fn planning_a_new_session_writes_the_files_and_picks_an_id() {
    let scratch = tempfile::tempdir().unwrap();
    let launch = Launch {
        scratch: scratch.path().to_path_buf(),
        ..launch()
    };
    let plan = driver(Os::Unix, &[]).plan(&launch).unwrap();
    assert!(plan.refusal.is_none());
    let id = arg_after(&plan.args, "--session-id").expect("a new session id");
    assert!(uuid::Uuid::parse_str(id).is_ok(), "{id}");
    assert!(!plan.args.iter().any(|arg| arg.starts_with("--resume")));
    assert_eq!(
        std::fs::read_to_string(scratch.path().join("instructions.md")).unwrap(),
        "Be brief."
    );
    let settings: Value = serde_json::from_str(
        &std::fs::read_to_string(scratch.path().join("settings.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(settings["sandbox"]["enabled"], true);
    assert_eq!(settings["sandbox"]["allowUnsandboxedCommands"], false);
    assert_eq!(
        arg_after(&plan.args, "--settings").map(PathBuf::from),
        Some(scratch.path().join("settings.json"))
    );
}

#[test]
fn two_new_sessions_get_different_ids() {
    let scratch = tempfile::tempdir().unwrap();
    let launch = Launch {
        scratch: scratch.path().to_path_buf(),
        ..launch()
    };
    let driver = driver(Os::Unix, &[]);
    let first = driver.plan(&launch).unwrap();
    let second = driver.plan(&launch).unwrap();
    assert_ne!(
        arg_after(&first.args, "--session-id"),
        arg_after(&second.args, "--session-id")
    );
}

#[test]
fn planning_a_known_session_resumes_it() {
    let scratch = tempfile::tempdir().unwrap();
    let launch = Launch {
        scratch: scratch.path().to_path_buf(),
        resume: Some(ID.into()),
        ..launch()
    };
    let plan = driver(Os::Windows, &[ID]).plan(&launch).unwrap();
    assert!(plan.refusal.is_none());
    assert_eq!(plan.args.last().unwrap(), &format!("--resume={ID}"));
    assert!(!plan.args.iter().any(|arg| arg == "--session-id"));
    assert_eq!(
        std::fs::read_to_string(scratch.path().join("settings.json")).unwrap(),
        r#"{"disableAllHooks":true}"#
    );
}

#[test]
fn planning_a_missing_session_refuses_without_writing() {
    let scratch = tempfile::tempdir().unwrap();
    let launch = Launch {
        scratch: scratch.path().to_path_buf(),
        resume: Some(ID.into()),
        ..launch()
    };
    let plan = driver(Os::Unix, &[]).plan(&launch).unwrap();
    let failure = plan.refusal.expect("a refusal");
    assert_eq!(failure.kind, FailureKind::SessionLost);
    assert!(failure.message.contains(ID));
    assert!(plan.args.is_empty());
    assert_eq!(std::fs::read_dir(scratch.path()).unwrap().count(), 0);
}

#[test]
fn an_empty_scratch_folder_is_an_error() {
    let launch = Launch {
        scratch: PathBuf::new(),
        ..launch()
    };
    assert!(driver(Os::Unix, &[]).plan(&launch).is_err());
}

#[tokio::test]
async fn a_missing_session_is_reported_through_the_event_stream() {
    let scratch = tempfile::tempdir().unwrap();
    let registry = Arc::new(Registry::default());
    let launch = Launch {
        scratch: scratch.path().to_path_buf(),
        resume: Some(ID.into()),
        ..launch()
    };
    let (session, events) = start_in(
        Arc::clone(&registry),
        Environment::from_process(),
        Arc::new(driver(Os::Unix, &[])),
        launch,
    )
    .expect("a refused session still starts");

    match events.recv_async().await.unwrap() {
        Event::TurnFinished(Outcome::Failed(failure)) => {
            assert_eq!(failure.kind, FailureKind::SessionLost);
        }
        other => panic!("unexpected event {other:?}"),
    }
    assert!(matches!(
        events.recv_async().await.unwrap(),
        Event::Exited(Some(_))
    ));
    assert_eq!(registry.live(), 0);
    assert!(session.control().is_finished());
    session.prompt("ignored".into());
    session.interrupt();
    session.shutdown().await;
}

#[rstest]
#[case::found(
    "-work-toy",
    "951f14f9-9400-4688-a95a-733f42893977.jsonl",
    "951f14f9-9400-4688-a95a-733f42893977",
    true
)]
#[case::other_project(
    "-work-other",
    "951f14f9-9400-4688-a95a-733f42893977.jsonl",
    "951f14f9-9400-4688-a95a-733f42893977",
    true
)]
#[case::missing(
    "-work-toy",
    "unrelated.jsonl",
    "951f14f9-9400-4688-a95a-733f42893977",
    false
)]
#[case::traversal(
    "-work-toy",
    "951f14f9-9400-4688-a95a-733f42893977.jsonl",
    "../projects/-work-toy/951f14f9-9400-4688-a95a-733f42893977",
    false
)]
#[case::separator("-work-toy", "x.jsonl", "-work-toy/x", false)]
#[case::empty("-work-toy", ".jsonl", "", false)]
fn session_files_are_looked_up_under_every_project(
    #[case] project: &str,
    #[case] file: &str,
    #[case] id: &str,
    #[case] expected: bool,
) {
    let config = tempfile::tempdir().unwrap();
    let folder = config.path().join("projects").join(project);
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join(file), "{}\n").unwrap();
    assert_eq!(session_file_exists(config.path(), id), expected);
}

#[test]
fn no_projects_folder_means_no_session() {
    let config = tempfile::tempdir().unwrap();
    assert!(!session_file_exists(config.path(), ID));
}

#[rstest]
#[case::configured(Some("/custom"), Some("/home/me"), Some("/custom"))]
#[case::default(None, Some("/home/me"), Some("/home/me/.claude"))]
#[case::empty_value(Some(""), Some("/home/me"), Some("/home/me/.claude"))]
#[case::nowhere(None, None, None)]
fn the_config_dir_follows_the_variable_then_home(
    #[case] configured: Option<&str>,
    #[case] home: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let dir = config_dir(configured.map(std::ffi::OsStr::new), home.map(Path::new));
    assert_eq!(dir, expected.map(PathBuf::from));
}
