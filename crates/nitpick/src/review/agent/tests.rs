use std::ffi::OsString;

use ::agent::process::Environment;
use gpui::SharedString;
use rstest::rstest;

use super::{Lookup, Phase, locate_in, unread_after};

fn environment_with_path(path: &std::path::Path) -> Environment {
    Environment::merge(
        vec![(OsString::from("PATH"), path.as_os_str().to_owned())],
        &[],
        None,
    )
}

#[cfg(unix)]
fn install(dir: &std::path::Path, name: &str) {
    use std::os::unix::fs::PermissionsExt;
    let file = dir.join(name);
    std::fs::write(&file, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(unix)]
#[rstest]
#[case::program("nitpick-fake-agent", Lookup::Found)]
#[case::program_with_arguments("nitpick-fake-agent acp --flag", Lookup::Found)]
#[case::quoted_program("\"nitpick-fake-agent\" acp", Lookup::Found)]
#[case::other_program("nitpick-other-agent", Lookup::Missing)]
#[case::empty("", Lookup::Missing)]
#[case::blank("   ", Lookup::Missing)]
fn locate_searches_the_path(#[case] command: &str, #[case] expected: Lookup) {
    let dir = tempfile::tempdir().unwrap();
    install(dir.path(), "nitpick-fake-agent");
    assert_eq!(
        locate_in(command, environment_with_path(dir.path())),
        expected
    );
}

#[cfg(unix)]
#[test]
fn locate_refuses_a_file_without_the_execute_bit() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("nitpick-fake-agent"), "").unwrap();
    assert_eq!(
        locate_in("nitpick-fake-agent", environment_with_path(dir.path())),
        Lookup::Missing
    );
}

#[cfg(unix)]
#[test]
fn locate_accepts_an_existing_path_and_refuses_a_missing_one() {
    let dir = tempfile::tempdir().unwrap();
    install(dir.path(), "wrapper");
    let existing = dir.path().join("wrapper");
    let missing = dir.path().join("gone");
    let env = || environment_with_path(dir.path());
    assert_eq!(
        locate_in(&existing.display().to_string(), env()),
        Lookup::Found
    );
    assert_eq!(
        locate_in(&missing.display().to_string(), env()),
        Lookup::Missing
    );
}

#[test]
fn locate_misses_a_program_that_is_nowhere() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        locate_in(
            "nitpick-no-such-agent-anywhere",
            environment_with_path(dir.path())
        ),
        Lookup::Missing
    );
}

#[rstest]
#[case::idle(Phase::Idle, false)]
#[case::starting(Phase::Starting, true)]
#[case::running(Phase::Running, true)]
#[case::ready(Phase::Ready, false)]
#[case::failed(Phase::Failed(SharedString::from("boom")), false)]
fn only_starting_and_running_are_busy(#[case] phase: Phase, #[case] busy: bool) {
    assert_eq!(phase.is_busy(), busy);
}

#[rstest]
#[case::finished_while_closed(false, false, Phase::Ready, true)]
#[case::finished_while_open(false, true, Phase::Ready, false)]
#[case::stays_unread_until_opened(true, true, Phase::Ready, true)]
#[case::new_turn_clears_it(true, false, Phase::Running, false)]
#[case::starting_clears_it(true, false, Phase::Starting, false)]
#[case::idle_keeps_it(true, false, Phase::Idle, true)]
#[case::failure_keeps_it(true, false, Phase::Failed(SharedString::from("x")), true)]
fn unread_follows_the_phase(
    #[case] unread: bool,
    #[case] open: bool,
    #[case] phase: Phase,
    #[case] expected: bool,
) {
    assert_eq!(unread_after(unread, open, &phase), expected);
}
