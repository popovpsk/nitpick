use std::path::PathBuf;

use rstest::rstest;

use crate::pi::{PiDriver, arguments, valid_session_id};
use crate::session::Driver;
use crate::{FailureKind, HarnessKind, Launch};

const ID: &str = "nitpick-0b1e5b36-0f0f-4e0e-9c58-3a3d2b9a7a10";

fn launch() -> Launch {
    Launch {
        kind: HarnessKind::Pi,
        command: "pi".into(),
        cwd: PathBuf::from("/work/toy"),
        model: None,
        resume: None,
        history: false,
        instructions: "Be brief.".into(),
        scratch: PathBuf::from("/scratch"),
    }
}

fn instructions_path() -> String {
    PathBuf::from("/scratch")
        .join("instructions.md")
        .to_string_lossy()
        .into_owned()
}

fn fixed() -> Vec<String> {
    [
        "--mode",
        "rpc",
        "--no-themes",
        "--no-approve",
        "--no-skills",
        "--no-prompt-templates",
        "--tools",
        "read,grep,find,ls,bash",
        "--session-id",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

#[test]
fn a_new_session_gets_the_fixed_flags_and_the_given_id() {
    let mut expected = fixed();
    expected.extend([
        ID.to_owned(),
        "--append-system-prompt".to_owned(),
        instructions_path(),
    ]);
    assert_eq!(arguments(&launch(), ID), expected);
}

#[test]
fn a_resume_passes_the_stored_id_with_the_same_flags() {
    let launch = Launch {
        resume: Some("nitpick-stored".into()),
        history: false,
        ..launch()
    };
    let mut expected = fixed();
    expected.extend([
        "nitpick-stored".to_owned(),
        "--append-system-prompt".to_owned(),
        instructions_path(),
    ]);
    assert_eq!(arguments(&launch, "nitpick-stored"), expected);
}

#[test]
fn a_model_is_appended_last() {
    let launch = Launch {
        model: Some("anthropic/claude-sonnet-5-5".into()),
        ..launch()
    };
    let mut expected = fixed();
    expected.extend([
        ID.to_owned(),
        "--append-system-prompt".to_owned(),
        instructions_path(),
        "--model".to_owned(),
        "anthropic/claude-sonnet-5-5".to_owned(),
    ]);
    assert_eq!(arguments(&launch, ID), expected);
}

#[rstest]
#[case::empty(Some(""))]
#[case::none(None)]
fn no_model_means_no_flag(#[case] model: Option<&str>) {
    let launch = Launch {
        model: model.map(str::to_owned),
        ..launch()
    };
    assert!(!arguments(&launch, ID).contains(&"--model".to_owned()));
}

#[test]
fn no_edit_tool_and_no_long_text_in_the_arguments() {
    let launch = Launch {
        instructions: "{\"long\": \"text\"}\nsecond line".into(),
        scratch: PathBuf::from(r"C:\Users\me\scratch"),
        ..launch()
    };
    let args = arguments(&launch, ID);
    let tools = args.iter().position(|arg| arg == "--tools").unwrap();
    let list: Vec<&str> = args[tools + 1].split(',').collect();
    assert_eq!(list, ["read", "grep", "find", "ls", "bash"]);
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
fn plan_writes_the_instructions_file_and_picks_a_fresh_id() {
    let scratch = tempfile::tempdir().unwrap();
    let launch = Launch {
        scratch: scratch.path().to_path_buf(),
        instructions: "Be brief.\nRead only.".into(),
        ..launch()
    };
    let plan = PiDriver::new().plan(&launch).unwrap();
    assert!(plan.refusal.is_none());
    assert!(plan.env.is_empty());
    assert_eq!(
        std::fs::read_to_string(scratch.path().join("instructions.md")).unwrap(),
        "Be brief.\nRead only."
    );
    let at = plan
        .args
        .iter()
        .position(|arg| arg == "--session-id")
        .unwrap();
    let id = &plan.args[at + 1];
    let uuid = id.strip_prefix("nitpick-").expect("the id is ours");
    assert!(uuid::Uuid::parse_str(uuid).is_ok(), "{id}");
    assert_eq!(
        plan.args[at + 3],
        scratch
            .path()
            .join("instructions.md")
            .to_string_lossy()
            .into_owned()
    );
    let again = PiDriver::new().plan(&launch).unwrap();
    assert_ne!(plan.args[at + 1], again.args[at + 1]);
}

#[test]
fn plan_resumes_by_the_stored_id() {
    let scratch = tempfile::tempdir().unwrap();
    let launch = Launch {
        scratch: scratch.path().to_path_buf(),
        resume: Some("nitpick-stored".into()),
        history: false,
        ..launch()
    };
    let plan = PiDriver::new().plan(&launch).unwrap();
    let at = plan
        .args
        .iter()
        .position(|arg| arg == "--session-id")
        .unwrap();
    assert_eq!(plan.args[at + 1], "nitpick-stored");
}

#[test]
fn an_empty_resume_id_starts_a_new_session() {
    let scratch = tempfile::tempdir().unwrap();
    let launch = Launch {
        scratch: scratch.path().to_path_buf(),
        resume: Some(String::new()),
        history: false,
        ..launch()
    };
    let plan = PiDriver::new().plan(&launch).unwrap();
    let at = plan
        .args
        .iter()
        .position(|arg| arg == "--session-id")
        .unwrap();
    assert!(plan.args[at + 1].starts_with("nitpick-"));
}

#[test]
fn an_id_pi_cannot_accept_is_a_lost_session_without_a_process() {
    let scratch = tempfile::tempdir().unwrap();
    let launch = Launch {
        scratch: scratch.path().to_path_buf(),
        resume: Some("--mode".into()),
        history: false,
        ..launch()
    };
    let plan = PiDriver::new().plan(&launch).unwrap();
    assert!(plan.args.is_empty());
    assert_eq!(plan.refusal.unwrap().kind, FailureKind::SessionLost);
    assert!(!scratch.path().join("instructions.md").exists());
}

#[test]
fn a_missing_scratch_folder_is_an_error() {
    let launch = Launch {
        scratch: PathBuf::new(),
        ..launch()
    };
    assert!(PiDriver::new().plan(&launch).is_err());
}

#[rstest]
#[case::generated("nitpick-0b1e5b36-0f0f-4e0e-9c58-3a3d2b9a7a10", true)]
#[case::dots_and_underscores("a.b_c-d", true)]
#[case::single("a", true)]
#[case::empty("", false)]
#[case::leading_dash("-a", false)]
#[case::trailing_dot("a.", false)]
#[case::space("a b", false)]
#[case::slash("a/b", false)]
#[case::non_ascii("naïve", false)]
fn session_ids_follow_pis_rule(#[case] id: &str, #[case] valid: bool) {
    assert_eq!(valid_session_id(id), valid);
}
