use std::time::Duration;

use rstest::rstest;

use crate::{AcpFlavor, Error, HarnessKind, Launch, Session};

fn launch(kind: HarnessKind) -> Launch {
    Launch {
        kind,
        command: "claude".into(),
        cwd: std::env::temp_dir(),
        model: None,
        resume: None,
        instructions: String::new(),
        scratch: Default::default(),
    }
}

#[rstest]
#[case::pi(HarnessKind::Pi)]
#[case::opencode(HarnessKind::Acp(AcpFlavor::OpenCode))]
#[case::deepseek(HarnessKind::Acp(AcpFlavor::DeepSeek))]
#[case::generic(HarnessKind::Acp(AcpFlavor::Generic))]
fn harnesses_without_a_driver_are_unsupported(#[case] kind: HarnessKind) {
    assert!(matches!(
        Session::start(launch(kind)),
        Err(Error::Unsupported)
    ));
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

    use super::*;
    use crate::Event;
    use crate::lines::Lines;
    use crate::process::Environment;
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
}
