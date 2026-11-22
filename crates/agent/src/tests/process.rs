use std::ffi::OsString;

use crate::Error;
use rstest::rstest;

use crate::process::{Environment, Grace, Invocation, REAPER_SCRIPT, reaper_args};

fn env_with(pairs: &[(&str, &str)]) -> Environment {
    let vars = pairs
        .iter()
        .map(|(key, value)| (OsString::from(key), OsString::from(value)))
        .collect();
    Environment::merge(vars, &[], None)
}

#[rstest]
#[case::default_grace(3000, 2000, "3", "2")]
#[case::rounds_up(1500, 1, "2", "1")]
#[case::zero(0, 0, "0", "0")]
fn the_reaper_waits_whole_seconds(
    #[case] term_ms: u64,
    #[case] kill_ms: u64,
    #[case] term: &str,
    #[case] kill: &str,
) {
    let grace = Grace {
        stdin: std::time::Duration::from_millis(term_ms),
        term: std::time::Duration::from_millis(kill_ms),
    };
    let args = reaper_args(&[4242], grace).unwrap();
    assert_eq!(&args[2..], ["sh", term, kill, "4242"]);
}

#[test]
fn the_reaper_gets_only_plain_group_numbers() {
    let args = reaper_args(
        &[0, 1, 2, 31337, i32::MAX as u32 + 1, u32::MAX],
        Grace::default(),
    )
    .unwrap();
    assert_eq!(args[0], "-c");
    assert_eq!(args[1], REAPER_SCRIPT);
    assert_eq!(&args[2..], ["sh", "3", "2", "2", "31337"]);
    assert!(
        args[3..]
            .iter()
            .all(|arg| arg.chars().all(|c| c.is_ascii_digit()))
    );
}

#[rstest]
#[case::none(&[])]
#[case::reserved(&[0, 1])]
#[case::out_of_range(&[u32::MAX])]
fn the_reaper_is_not_started_without_a_real_group(#[case] groups: &[u32]) {
    assert!(reaper_args(groups, Grace::default()).is_none());
}

#[test]
fn an_empty_command_is_rejected() {
    for command in ["", "   "] {
        let result = Invocation::resolve_in(
            command,
            Vec::new(),
            &[],
            env_with(&[("PATH", "/bin")]),
            &std::env::temp_dir(),
        );
        assert!(matches!(result, Err(Error::EmptyCommand)));
    }
}

#[test]
fn a_missing_program_is_reported_by_name() {
    let result = Invocation::resolve_in(
        "no-such-program-for-nitpick",
        Vec::new(),
        &[],
        env_with(&[("PATH", "/bin")]),
        &std::env::temp_dir(),
    );
    assert!(matches!(result, Err(Error::NotFound(name)) if name == "no-such-program-for-nitpick"));
}

#[cfg(unix)]
mod unix {
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::process::ExitStatusExt;

    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use rstest::rstest;

    use super::super::group::{assert_dies, ps_state};
    use super::*;
    use crate::lines::Lines;
    use crate::process::{Grace, Process, resolve_program};

    fn invocation(program: &str, args: &[&str], env: Environment) -> Invocation {
        Invocation::bare(
            PathBuf::from(program),
            args.iter().map(OsString::from).collect(),
            std::env::temp_dir(),
            env,
        )
    }

    fn sh(script: &str) -> Invocation {
        invocation(
            "/bin/sh",
            &["-c", script],
            env_with(&[("PATH", "/usr/bin:/bin")]),
        )
    }

    async fn read_line(lines: &mut Lines<tokio::process::ChildStdout>) -> String {
        tokio::time::timeout(Duration::from_secs(5), lines.next_line())
            .await
            .expect("the child printed in time")
            .unwrap()
            .expect("the child printed a line")
    }

    #[tokio::test]
    async fn runs_a_command_and_keeps_the_stderr_tail() {
        let (mut process, pipes) = Process::spawn(&sh("echo hi; echo oops >&2; exit 3")).unwrap();
        drop(pipes.stdin);
        let mut lines = Lines::new(pipes.stdout);
        assert_eq!(read_line(&mut lines).await, "hi");
        let exit = process.wait().await;
        assert_eq!(exit.status.and_then(|status| status.code()), Some(3));
        assert_eq!(exit.stderr, "oops");
    }

    #[tokio::test]
    async fn talks_to_the_child_over_its_pipes() {
        use tokio::io::AsyncWriteExt;
        let (mut process, mut pipes) = Process::spawn(&sh("cat")).unwrap();
        pipes.stdin.write_all(b"ping\n").await.unwrap();
        let mut lines = Lines::new(pipes.stdout);
        assert_eq!(read_line(&mut lines).await, "ping");
        drop(pipes.stdin);
        let exit = process.shutdown(Grace::default()).await;
        assert!(exit.status.is_some_and(|status| status.success()));
    }

    #[tokio::test]
    async fn closing_stdin_is_enough_for_a_polite_child() {
        let (mut process, pipes) = Process::spawn(&sh("cat >/dev/null")).unwrap();
        drop(pipes.stdin);
        let started = Instant::now();
        let exit = process.shutdown(Grace::default()).await;
        assert!(started.elapsed() < Duration::from_millis(2500));
        assert!(exit.status.is_some_and(|status| status.success()));
    }

    #[tokio::test]
    async fn the_soft_shutdown_signals_the_whole_group() {
        let (mut process, pipes) = Process::spawn(&sh("sleep 300 & echo $!; wait")).unwrap();
        drop(pipes.stdin);
        let mut lines = Lines::new(pipes.stdout);
        let pid = read_line(&mut lines).await;
        assert!(ps_state(&pid).await.is_some());

        let grace = Grace {
            stdin: Duration::from_millis(100),
            term: Duration::from_secs(3),
        };
        let exit = process.shutdown(grace).await;
        assert_eq!(exit.status.and_then(|status| status.signal()), Some(15));
        assert_dies(&pid).await;
    }

    #[tokio::test]
    async fn a_child_that_ignores_sigterm_is_killed() {
        let (mut process, pipes) =
            Process::spawn(&sh("trap '' TERM; sleep 300 & echo $!; wait")).unwrap();
        drop(pipes.stdin);
        let mut lines = Lines::new(pipes.stdout);
        let pid = read_line(&mut lines).await;

        let grace = Grace {
            stdin: Duration::from_millis(100),
            term: Duration::from_millis(200),
        };
        let exit = process.shutdown(grace).await;
        assert_eq!(exit.status.and_then(|status| status.signal()), Some(9));
        assert_dies(&pid).await;
    }

    #[tokio::test]
    async fn nothing_in_the_group_outlives_a_clean_exit() {
        let (mut process, pipes) = Process::spawn(&sh("sleep 300 & echo $!; exit 0")).unwrap();
        drop(pipes.stdin);
        let mut lines = Lines::new(pipes.stdout);
        let pid = read_line(&mut lines).await;
        let exit = process.wait().await;
        assert!(exit.status.is_some_and(|status| status.success()));
        assert_dies(&pid).await;
    }

    #[tokio::test]
    async fn the_reaper_ends_a_group_nobody_supervises() {
        let (process, pipes) =
            Process::spawn(&sh("trap '' TERM; sleep 300 & echo $!; wait")).unwrap();
        let leader = process.id().unwrap();
        let mut lines = Lines::new(pipes.stdout);
        let child = read_line(&mut lines).await;
        std::mem::forget(process);
        std::mem::forget(pipes.stdin);
        std::mem::forget(lines);

        let grace = Grace {
            stdin: Duration::from_secs(1),
            term: Duration::from_secs(1),
        };
        crate::process::reap(&[leader], grace).unwrap();

        assert_dies(&child).await;
        assert_dies(&leader.to_string()).await;
    }

    #[tokio::test]
    async fn dropping_the_process_kills_the_group() {
        let (process, pipes) = Process::spawn(&sh("sleep 300 & echo $!; wait")).unwrap();
        drop(pipes.stdin);
        let mut lines = Lines::new(pipes.stdout);
        let pid = read_line(&mut lines).await;
        drop(process);
        assert_dies(&pid).await;
    }

    #[tokio::test]
    async fn scrubbed_variables_never_reach_the_child() {
        let env = env_with(&[
            ("PATH", "/usr/bin:/bin"),
            ("CLAUDECODE", "1"),
            ("ORCA_HANDLE", "1"),
            ("GIT_DIR", "/elsewhere"),
            ("KEEP", "yes"),
            ("ANTHROPIC_API_KEY", "key"),
        ]);
        let script = r#"echo "[$CLAUDECODE][$ORCA_HANDLE][$GIT_DIR][$KEEP][$ANTHROPIC_API_KEY]""#;
        let (mut process, pipes) =
            Process::spawn(&invocation("/bin/sh", &["-c", script], env)).unwrap();
        drop(pipes.stdin);
        let mut lines = Lines::new(pipes.stdout);
        assert_eq!(read_line(&mut lines).await, "[][][][yes][key]");
        process.wait().await;
    }

    #[test]
    fn a_command_line_becomes_a_program_and_arguments() {
        let env = env_with(&[("PATH", "/usr/bin:/bin")]);
        let invocation = Invocation::resolve_in(
            r#"sh -c "echo a b""#,
            vec!["extra".to_owned()],
            &[("FOO".to_owned(), "bar".to_owned())],
            env,
            &std::env::temp_dir(),
        )
        .unwrap();
        assert!(invocation.program.ends_with("sh"));
        assert!(invocation.program.is_absolute());
        assert_eq!(
            invocation.args,
            vec![
                OsString::from("-c"),
                OsString::from("echo a b"),
                OsString::from("extra")
            ]
        );
        assert_eq!(invocation.env.get("FOO"), Some(std::ffi::OsStr::new("bar")));
    }

    #[test]
    fn a_missing_absolute_program_fails_to_spawn_as_not_found() {
        let result = Process::spawn(&invocation(
            "/no/such/dir/program",
            &[],
            env_with(&[("PATH", "/bin")]),
        ));
        assert!(matches!(result, Err(Error::NotFound(_))));
    }

    #[rstest]
    #[case::executable("tool", 0o755, true)]
    #[case::not_executable("plain", 0o644, false)]
    #[case::missing("absent", 0o755, false)]
    fn finds_only_executable_files_on_the_path(
        #[case] name: &str,
        #[case] mode: u32,
        #[case] found: bool,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir
            .path()
            .join(if name == "absent" { "other" } else { name });
        std::fs::write(&file, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(mode)).unwrap();
        std::fs::create_dir(dir.path().join("adir")).unwrap();
        let path = std::env::join_paths([
            PathBuf::from("/definitely/not/here"),
            dir.path().to_path_buf(),
        ])
        .unwrap();
        let resolved = resolve_program(name, &path, "");
        assert_eq!(resolved.is_some(), found);
        if found {
            assert_eq!(resolved.unwrap(), dir.path().join(name));
        }
        assert_eq!(resolve_program("adir", &path, ""), None);
    }

    #[test]
    fn a_program_with_a_separator_is_used_as_given() {
        assert_eq!(
            resolve_program("./bin/tool", std::ffi::OsStr::new("/bin"), ""),
            Some(PathBuf::from("./bin/tool"))
        );
        assert_eq!(resolve_program("", std::ffi::OsStr::new("/bin"), ""), None);
    }

    #[tokio::test]
    async fn probe_reports_the_first_line_of_the_version() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fake-agent");
        std::fs::write(
            &script,
            "#!/bin/sh\necho \"fake-agent 1.2.3 ($1)\"\necho extra\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let command = format!("\"{}\"", script.display());
        let version = crate::probe(&command, crate::HarnessKind::Claude)
            .await
            .unwrap();
        assert_eq!(version, "fake-agent 1.2.3 (--version)");
    }

    #[tokio::test]
    async fn probe_reports_a_failing_command() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("broken-agent");
        std::fs::write(&script, "#!/bin/sh\necho 'not logged in' >&2\nexit 2\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let command = format!("\"{}\"", script.display());
        let error = crate::probe(&command, crate::HarnessKind::Pi)
            .await
            .unwrap_err();
        assert!(matches!(&error, Error::Probe { detail, .. } if detail == "not logged in"));
    }

    #[tokio::test]
    async fn probe_reports_a_missing_command() {
        let error = crate::probe("no-such-program-for-nitpick", crate::HarnessKind::Codex)
            .await
            .unwrap_err();
        assert!(matches!(error, Error::NotFound(_)));
    }
}

mod drain {
    use std::time::Duration;

    use tokio::task::JoinHandle;

    use crate::process::drain;

    fn finishing_after(delay: Duration) -> Option<JoinHandle<()>> {
        Some(tokio::spawn(async move { tokio::time::sleep(delay).await }))
    }

    #[tokio::test]
    async fn a_cancelled_drain_leaves_the_reader_for_the_next_one() {
        let mut task = finishing_after(Duration::from_millis(200));
        tokio::select! {
            () = drain(&mut task, Duration::from_secs(5)) => panic!("the reader was still running"),
            () = tokio::time::sleep(Duration::from_millis(20)) => {}
        }
        assert!(task.is_some());
        drain(&mut task, Duration::from_secs(5)).await;
        assert!(task.is_none());
    }

    #[tokio::test]
    async fn a_reader_that_outlasts_the_limit_is_given_up_on() {
        let mut task = finishing_after(Duration::from_secs(60));
        drain(&mut task, Duration::from_millis(20)).await;
        assert!(task.is_none());
    }

    #[tokio::test]
    async fn no_reader_is_nothing_to_wait_for() {
        let mut task = None;
        drain(&mut task, Duration::from_secs(5)).await;
        assert!(task.is_none());
    }
}
