use std::time::Duration;

use rstest::rstest;

use crate::session::driver_for;
use crate::{AcpFlavor, Error, HarnessKind, Launch, Session};

fn launch(kind: HarnessKind) -> Launch {
    Launch {
        kind,
        command: "claude".into(),
        cwd: std::env::temp_dir(),
        model: None,
        resume: None,
        history: false,
        instructions: String::new(),
        scratch: Default::default(),
    }
}

#[rstest]
#[case::claude(HarnessKind::Claude)]
#[case::codex(HarnessKind::Codex)]
#[case::opencode(HarnessKind::Acp(AcpFlavor::OpenCode))]
#[case::deepseek(HarnessKind::Acp(AcpFlavor::DeepSeek))]
#[case::generic(HarnessKind::Acp(AcpFlavor::Generic))]
#[case::pi(HarnessKind::Pi)]
fn every_harness_has_a_driver(#[case] kind: HarnessKind) {
    assert!(driver_for(kind).is_ok());
}

#[tokio::test]
async fn shutting_down_everything_with_no_sessions_returns_at_once() {
    let registry = crate::registry::Registry::default();
    tokio::time::timeout(Duration::from_secs(1), registry.shutdown_all())
        .await
        .unwrap();
    assert_eq!(registry.live(), 0);
}

#[cfg(unix)]
mod unix {
    use std::sync::Arc;

    use tokio::io::AsyncWriteExt;

    use super::super::group::assert_dies;
    use super::*;
    use crate::Event;
    use crate::Failure;
    use crate::FailureKind;
    use crate::lines::Lines;
    use crate::process::Environment;
    use crate::process::Grace;
    use crate::registry::Registry;
    use crate::session::{BoxFuture, Channels, Command, Driver, Plan, Transport, start_in};

    struct EchoDriver {
        args: Vec<String>,
    }

    impl Driver for EchoDriver {
        fn plan(&self, _launch: &Launch) -> Result<Plan, Error> {
            Ok(Plan {
                args: self.args.clone(),
                env: vec![("NITPICK_TEST".into(), "1".into())],
                ..Plan::default()
            })
        }

        fn run(&self, _launch: Launch, transport: Transport, channels: Channels) -> BoxFuture {
            Box::pin(async move {
                let Transport { reader, mut writer } = transport;
                let mut lines = Lines::new(reader);
                loop {
                    tokio::select! {
                        command = channels.commands.recv_async() => match command {
                            Ok(Command::Prompt(text)) => {
                                let line = format!("{text}\n");
                                if writer.write_all(line.as_bytes()).await.is_err() {
                                    break;
                                }
                            }
                            Ok(Command::Interrupt) => {
                                let _ = channels.events.send(Event::Notice("interrupted".into()));
                            }
                            Ok(Command::Shutdown) | Err(_) => break,
                        },
                        line = lines.next_line() => match line {
                            Ok(Some(line)) => {
                                let _ = channels.events.send(Event::Text(line));
                            }
                            _ => break,
                        },
                    }
                }
            })
        }
    }

    fn launch_with(command: &str, dir: &std::path::Path) -> Launch {
        Launch {
            command: command.into(),
            cwd: dir.to_path_buf(),
            ..launch(HarnessKind::Claude)
        }
    }

    fn start(
        registry: &Arc<Registry>,
        command: &str,
        dir: &std::path::Path,
    ) -> Result<(Session, flume::Receiver<Event>), Error> {
        start_in(
            Arc::clone(registry),
            Environment::from_process(),
            Arc::new(EchoDriver { args: Vec::new() }),
            launch_with(command, dir),
        )
    }

    async fn next(events: &flume::Receiver<Event>) -> Event {
        tokio::time::timeout(Duration::from_secs(10), events.recv_async())
            .await
            .expect("an event arrived in time")
            .expect("the event channel is open")
    }

    #[tokio::test]
    async fn prompts_reach_the_process_and_its_output_becomes_events() {
        let dir = tempfile::tempdir().unwrap();
        let registry = Arc::new(Registry::default());
        let (session, events) = start(&registry, "cat", dir.path()).unwrap();
        assert_eq!(registry.live(), 1);

        session.prompt("hello".into());
        assert_eq!(next(&events).await, Event::Text("hello".into()));
        session.interrupt();
        assert_eq!(next(&events).await, Event::Notice("interrupted".into()));

        session.shutdown().await;
        assert_eq!(next(&events).await, Event::Exited(None));
        assert_eq!(registry.live(), 0);
        assert!(session.control().is_finished());
        session.prompt("ignored".into());
    }

    #[tokio::test]
    async fn the_driver_gets_the_command_words_then_its_own_arguments_and_environment() {
        let dir = tempfile::tempdir().unwrap();
        let registry = Arc::new(Registry::default());
        let (session, events) = start_in(
            Arc::clone(&registry),
            Environment::from_process(),
            Arc::new(EchoDriver {
                args: vec!["second".into()],
            }),
            launch_with(r#"sh -c "echo $NITPICK_TEST-$0-$1; cat" first"#, dir.path()),
        )
        .unwrap();
        assert_eq!(next(&events).await, Event::Text("1-first-second".into()));
        session.shutdown().await;
    }

    #[tokio::test]
    async fn a_dying_process_reports_its_stderr() {
        let dir = tempfile::tempdir().unwrap();
        let registry = Arc::new(Registry::default());
        let (_session, events) =
            start(&registry, r#"sh -c "echo boom >&2; exit 3""#, dir.path()).unwrap();
        assert_eq!(next(&events).await, Event::Exited(Some("boom".into())));
        tokio::time::timeout(Duration::from_secs(5), async {
            while registry.live() > 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn a_silent_exit_reports_its_status() {
        let dir = tempfile::tempdir().unwrap();
        let registry = Arc::new(Registry::default());
        let (_session, events) = start(&registry, r#"sh -c "exit 4""#, dir.path()).unwrap();
        match next(&events).await {
            Event::Exited(Some(message)) => assert!(message.contains('4'), "{message}"),
            other => panic!("unexpected event {other:?}"),
        }
    }

    #[tokio::test]
    async fn dropping_every_handle_ends_the_session() {
        let dir = tempfile::tempdir().unwrap();
        let registry = Arc::new(Registry::default());
        let (session, events) = start(&registry, "cat", dir.path()).unwrap();
        let clone = session.clone();
        drop(session);
        assert_eq!(registry.live(), 1);
        drop(clone);
        assert_eq!(next(&events).await, Event::Exited(None));
    }

    #[tokio::test]
    async fn shutdown_all_ends_every_session_of_its_registry_only() {
        let dir = tempfile::tempdir().unwrap();
        let ours = Arc::new(Registry::default());
        let theirs = Arc::new(Registry::default());
        let (_one, one_events) = start(&ours, "cat", dir.path()).unwrap();
        let (_two, two_events) = start(&ours, "cat", dir.path()).unwrap();
        let (other, other_events) = start(&theirs, "cat", dir.path()).unwrap();
        assert_eq!(ours.live(), 2);

        ours.shutdown_all().await;

        assert_eq!(next(&one_events).await, Event::Exited(None));
        assert_eq!(next(&two_events).await, Event::Exited(None));
        assert_eq!(ours.live(), 0);
        assert_eq!(theirs.live(), 1);
        other.prompt("still here".into());
        assert_eq!(next(&other_events).await, Event::Text("still here".into()));
        other.shutdown().await;
    }

    #[tokio::test]
    async fn a_missing_program_is_an_error_not_an_event() {
        let dir = tempfile::tempdir().unwrap();
        let registry = Arc::new(Registry::default());
        let result = start(&registry, "no-such-program-for-nitpick", dir.path());
        assert!(matches!(result, Err(Error::NotFound(_))));
        assert_eq!(registry.live(), 0);
    }

    #[tokio::test]
    async fn the_scratch_folder_is_created() {
        let dir = tempfile::tempdir().unwrap();
        let registry = Arc::new(Registry::default());
        let scratch = dir.path().join("a").join("b");
        let (session, _events) = start_in(
            Arc::clone(&registry),
            Environment::from_process(),
            Arc::new(EchoDriver { args: Vec::new() }),
            Launch {
                scratch: scratch.clone(),
                ..launch_with("cat", dir.path())
            },
        )
        .unwrap();
        assert!(scratch.is_dir());
        session.shutdown().await;
    }

    struct ScratchDriver {
        refuse: bool,
    }

    impl Driver for ScratchDriver {
        fn plan(&self, launch: &Launch) -> Result<Plan, Error> {
            if self.refuse {
                return Ok(Plan {
                    refusal: Some(Failure {
                        kind: FailureKind::SessionLost,
                        message: "gone".into(),
                    }),
                    ..Plan::default()
                });
            }
            std::fs::write(launch.scratch.join("settings.json"), "{}")?;
            Ok(Plan::default())
        }

        fn run(&self, _launch: Launch, _transport: Transport, _channels: Channels) -> BoxFuture {
            Box::pin(async {})
        }
    }

    struct StubbornDriver;

    impl Driver for StubbornDriver {
        fn plan(&self, _launch: &Launch) -> Result<Plan, Error> {
            Ok(Plan::default())
        }

        fn run(&self, _launch: Launch, transport: Transport, channels: Channels) -> BoxFuture {
            Box::pin(async move {
                let Transport { reader, writer } = transport;
                let mut lines = Lines::new(reader);
                if let Ok(Some(line)) = lines.next_line().await {
                    let _ = channels.events.send(Event::Text(line));
                }
                let _held = writer;
                std::future::pending::<()>().await;
            })
        }
    }

    fn start_scratch(
        registry: &Arc<Registry>,
        command: &str,
        dir: &std::path::Path,
        scratch: &std::path::Path,
        refuse: bool,
    ) -> Result<(Session, flume::Receiver<Event>), Error> {
        start_in(
            Arc::clone(registry),
            Environment::from_process(),
            Arc::new(ScratchDriver { refuse }),
            Launch {
                scratch: scratch.to_path_buf(),
                ..launch_with(command, dir)
            },
        )
    }

    #[tokio::test]
    async fn a_failed_start_removes_the_scratch_folder_it_created() {
        let dir = tempfile::tempdir().unwrap();
        let registry = Arc::new(Registry::default());
        let scratch = dir.path().join("session");
        let result = start_scratch(
            &registry,
            "no-such-program-for-nitpick",
            dir.path(),
            &scratch,
            false,
        );
        assert!(matches!(result, Err(Error::NotFound(_))));
        assert!(!scratch.exists());
        assert!(dir.path().exists());
    }

    #[tokio::test]
    async fn a_failed_start_leaves_a_scratch_folder_that_was_already_there() {
        let dir = tempfile::tempdir().unwrap();
        let registry = Arc::new(Registry::default());
        let scratch = dir.path().join("session");
        std::fs::create_dir(&scratch).unwrap();
        std::fs::write(scratch.join("keep.txt"), "mine").unwrap();
        let result = start_scratch(
            &registry,
            "no-such-program-for-nitpick",
            dir.path(),
            &scratch,
            false,
        );
        assert!(matches!(result, Err(Error::NotFound(_))));
        assert_eq!(
            std::fs::read_to_string(scratch.join("keep.txt")).unwrap(),
            "mine"
        );
    }

    #[tokio::test]
    async fn a_refused_session_leaves_no_scratch_folder() {
        let dir = tempfile::tempdir().unwrap();
        let registry = Arc::new(Registry::default());
        let scratch = dir.path().join("session");
        let (_session, events) =
            start_scratch(&registry, "cat", dir.path(), &scratch, true).unwrap();
        assert!(matches!(
            next(&events).await,
            Event::TurnFinished(crate::Outcome::Failed(_))
        ));
        assert!(!scratch.exists());
        assert_eq!(registry.live(), 0);
    }

    #[tokio::test]
    async fn a_started_session_keeps_the_scratch_folder_and_its_files() {
        let dir = tempfile::tempdir().unwrap();
        let registry = Arc::new(Registry::default());
        let scratch = dir.path().join("session");
        let (session, _events) =
            start_scratch(&registry, "cat", dir.path(), &scratch, false).unwrap();
        assert!(scratch.join("settings.json").is_file());
        session.shutdown().await;
        assert!(scratch.join("settings.json").is_file());
    }

    #[tokio::test]
    async fn the_registry_lists_the_groups_of_live_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let registry = Arc::new(Registry::default());
        assert!(registry.groups().is_empty());
        let (session, _events) = start(&registry, "cat", dir.path()).unwrap();
        let groups = registry.groups();
        assert_eq!(groups.len(), 1);
        assert_eq!(Some(groups[0]), session.control().leader());
        session.shutdown().await;
        assert!(registry.groups().is_empty());
    }

    #[tokio::test]
    async fn quitting_ends_a_group_that_ignores_stdin_and_sigterm_without_the_supervisor() {
        let dir = tempfile::tempdir().unwrap();
        let registry = Arc::new(Registry::default());
        let (session, events) = start_in(
            Arc::clone(&registry),
            Environment::from_process(),
            Arc::new(StubbornDriver),
            launch_with(
                r#"sh -c "trap '' TERM; sleep 300 & echo $!; wait""#,
                dir.path(),
            ),
        )
        .unwrap();
        let Event::Text(child) = next(&events).await else {
            panic!("expected the pid of the background command");
        };
        let leader = session.control().leader().unwrap().to_string();

        registry.quit(
            Grace {
                stdin: Duration::from_secs(1),
                term: Duration::from_secs(1),
            },
            &Environment::from_process(),
        );

        assert_dies(&child).await;
        assert_dies(&leader).await;
        assert_eq!(next(&events).await, Event::Exited(None));
    }
}
