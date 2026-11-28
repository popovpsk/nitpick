use std::path::PathBuf;

use crate::codex::{CodexDriver, arguments};
use crate::session::{Driver, driver_for};
use crate::{HarnessKind, Launch};

fn launch() -> Launch {
    Launch {
        kind: HarnessKind::Codex,
        command: "codex".into(),
        cwd: PathBuf::from("/work/toy"),
        model: Some("gpt-5.6-sol".into()),
        resume: Some("thread-old".into()),
        history: false,
        instructions: "Be brief.".into(),
        scratch: PathBuf::new(),
    }
}

#[test]
fn the_command_is_the_app_server_and_nothing_else() {
    assert_eq!(arguments(), vec!["app-server".to_owned()]);
    let plan = CodexDriver::new().plan(&launch()).unwrap();
    assert_eq!(plan.args, vec!["app-server".to_owned()]);
    assert!(plan.env.is_empty());
    assert!(plan.refusal.is_none());
}

#[test]
fn the_plan_needs_no_scratch_folder_and_ignores_resume() {
    let mut launch = launch();
    launch.resume = None;
    launch.scratch = PathBuf::new();
    assert!(CodexDriver::new().plan(&launch).unwrap().refusal.is_none());
}

#[test]
fn codex_has_a_driver() {
    assert!(driver_for(HarnessKind::Codex).is_ok());
}
