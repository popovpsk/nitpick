use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rstest::rstest;
use serde_json::Value;

use crate::acp::{AcpDriver, VersionProbe, probe_program_version};
use crate::session::{Driver, driver_for};
use crate::{AcpFlavor, Error, HarnessKind, Launch};

fn launch(flavor: AcpFlavor, command: &str) -> Launch {
    Launch {
        kind: HarnessKind::Acp(flavor),
        command: command.into(),
        cwd: PathBuf::from("/work/toy"),
        model: None,
        resume: None,
        instructions: "Be brief.".into(),
        scratch: PathBuf::new(),
    }
}

fn probing(answer: Option<&str>) -> (VersionProbe, Arc<Mutex<Vec<String>>>) {
    let asked = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&asked);
    let answer = answer.map(str::to_owned);
    let probe: VersionProbe = Arc::new(move |program| {
        seen.lock().unwrap().push(program.to_owned());
        answer.clone()
    });
    (probe, asked)
}

fn config_of(env: &[(String, String)]) -> Value {
    let (_, content) = env
        .iter()
        .find(|(key, _)| key == "OPENCODE_CONFIG_CONTENT")
        .expect("the config is set");
    serde_json::from_str(content).expect("the config is JSON")
}

fn value_of<'a>(env: &'a [(String, String)], key: &str) -> Option<&'a str> {
    env.iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.as_str())
}

#[rstest]
#[case::recorded_version(Some("1.19.4"), r#"{"permission":{"edit":"deny"}}"#)]
#[case::next_major(
    Some("2.0.0"),
    r#"{"permissions":[{"action":"edit","resource":"*","effect":"deny"}]}"#
)]
#[case::the_probe_failed(None, r#"{"permission":{"edit":"deny"}}"#)]
#[case::the_probe_printed_nonsense(Some("not a version"), r#"{"permission":{"edit":"deny"}}"#)]
fn opencode_gets_the_dialect_of_its_version(#[case] version: Option<&str>, #[case] expected: &str) {
    let (probe, _) = probing(version);
    let driver = AcpDriver::new(AcpFlavor::OpenCode).with_probe(probe);
    let plan = driver
        .plan(&launch(AcpFlavor::OpenCode, "opencode acp"))
        .unwrap();
    assert_eq!(
        config_of(&plan.env),
        serde_json::from_str::<Value>(expected).unwrap()
    );
    assert_eq!(
        value_of(&plan.env, "OPENCODE_DISABLE_PROJECT_CONFIG"),
        Some("1")
    );
    assert!(plan.args.is_empty());
    assert!(plan.refusal.is_none());
}

#[rstest]
#[case::plain("opencode acp", "opencode")]
#[case::wrapper("my-opencode --profile work acp", "my-opencode")]
#[case::quoted(r#""/opt/my tools/opencode" acp"#, "/opt/my tools/opencode")]
#[case::alone("opencode", "opencode")]
fn only_the_program_is_asked_for_its_version(#[case] command: &str, #[case] program: &str) {
    let (probe, asked) = probing(Some("1.19.4"));
    let driver = AcpDriver::new(AcpFlavor::OpenCode).with_probe(probe);
    driver.plan(&launch(AcpFlavor::OpenCode, command)).unwrap();
    assert_eq!(*asked.lock().unwrap(), vec![program.to_owned()]);
}

#[test]
fn an_empty_command_is_not_probed() {
    let (probe, asked) = probing(Some("2.0.0"));
    let driver = AcpDriver::new(AcpFlavor::OpenCode).with_probe(probe);
    let plan = driver.plan(&launch(AcpFlavor::OpenCode, "  ")).unwrap();
    assert!(asked.lock().unwrap().is_empty());
    assert_eq!(
        config_of(&plan.env),
        serde_json::json!({"permission": {"edit": "deny"}})
    );
}

#[rstest]
#[case::deepseek(AcpFlavor::DeepSeek)]
#[case::generic(AcpFlavor::Generic)]
fn other_flavors_need_no_probe_and_no_environment(#[case] flavor: AcpFlavor) {
    let (probe, asked) = probing(Some("2.0.0"));
    let driver = AcpDriver::new(flavor).with_probe(probe);
    let plan = driver.plan(&launch(flavor, "dsh acp")).unwrap();
    assert!(asked.lock().unwrap().is_empty());
    assert!(plan.env.is_empty());
    assert!(plan.args.is_empty());
    assert!(plan.refusal.is_none());
}

#[test]
fn a_program_that_does_not_exist_has_no_version() {
    assert_eq!(probe_program_version("no-such-program-for-nitpick"), None);
}

#[rstest]
#[case::opencode(AcpFlavor::OpenCode)]
#[case::deepseek(AcpFlavor::DeepSeek)]
#[case::generic(AcpFlavor::Generic)]
fn every_acp_flavor_has_a_driver(#[case] flavor: AcpFlavor) {
    assert!(driver_for(HarnessKind::Acp(flavor)).is_ok());
}

#[test]
fn pi_still_has_none() {
    assert!(matches!(
        driver_for(HarnessKind::Pi),
        Err(Error::Unsupported)
    ));
}
