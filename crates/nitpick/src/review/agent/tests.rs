use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

use ::agent::brief::{self, Brief};
use ::agent::process::Environment;
use ::agent::{Event, Failure, FailureKind};
use app_db::AgentKind;
use forge::ForgeKind;
use git_repo::Repo;
use gpui::SharedString;
use review_core::{FileStatus, Side};
use rstest::rstest;
use ui_resources::i18n::Lang;

use super::context::{
    BriefInputs, DiscussionInput, FileInput, WorkdirPlan, assemble, disk_matches, language_name,
    local_head, resolve_workdir, review_key, thread_position,
};
use super::logic::{
    self, LoadAction, OverviewState, Problem, Request, classify, coalesce, compose_chat, exit_text,
    limit_text, load_action, overview_state, problem_text,
};
use super::{Lookup, Phase, locate_in, unread_after};
use crate::agent_profiles::LoginHint;
use crate::review::ReviewSource;

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

#[rstest]
#[case::gitlab(
    ReviewSource::MergeRequest { forge: ForgeKind::GitLab, project: "group/app".into(), number: 12 },
    "mr:gitlab:group/app:12"
)]
#[case::github(
    ReviewSource::MergeRequest { forge: ForgeKind::GitHub, project: "owner/repo".into(), number: 7 },
    "mr:github:owner/repo:7"
)]
#[case::branches(
    ReviewSource::Branches { target: "main".into(), source: "feature/x".into() },
    "branches:main..feature/x"
)]
#[case::worktree(ReviewSource::WorkingTree, "worktree")]
fn review_key_names_the_review(#[case] source: ReviewSource, #[case] expected: &str) {
    assert_eq!(review_key(&source), expected);
}

#[test]
fn local_head_is_stable_and_follows_the_files() {
    let files = [("a.rs", "x..y"), ("b.rs", "p..q")];
    let same = local_head(files);
    assert_eq!(same, local_head(files));
    assert!(same.starts_with("local:"));
    assert_ne!(same, local_head([("a.rs", "x..z"), ("b.rs", "p..q")]));
    assert_ne!(same, local_head([("a.rs", "x..y")]));
    assert_ne!(same, local_head([("b.rs", "p..q"), ("a.rs", "x..y")]));
    assert_ne!(same, local_head([("a.rsx", "..y"), ("b.rs", "p..q")]));
    assert_ne!(local_head([("ab", "c")]), local_head([("a", "bc")]));
    assert_eq!(local_head([]), local_head([]));
}

#[rstest]
#[case::new_side(Some("a.rs"), Some("a.rs"), None, Some(5), Some(("a.rs", Side::New, 5)))]
#[case::old_side(Some("a.rs"), None, Some(3), None, Some(("a.rs", Side::Old, 3)))]
#[case::both_prefers_new(Some("o.rs"), Some("n.rs"), Some(3), Some(5), Some(("n.rs", Side::New, 5)))]
#[case::no_lines(Some("a.rs"), Some("a.rs"), None, None, None)]
#[case::no_path(None, None, Some(1), Some(1), None)]
fn thread_position_prefers_the_new_side(
    #[case] old_path: Option<&str>,
    #[case] new_path: Option<&str>,
    #[case] old_line: Option<u32>,
    #[case] new_line: Option<u32>,
    #[case] expected: Option<(&str, Side, u32)>,
) {
    let position = thread_position(old_path, new_path, old_line, new_line);
    assert_eq!(
        position.map(|p| (p.path, p.side, p.line)),
        expected.map(|(path, side, line)| (path.to_owned(), side, line))
    );
}

fn file(path: &str, status: FileStatus) -> FileInput {
    FileInput {
        path: path.to_owned(),
        old_path: Some(path.to_owned()),
        status,
        additions: 3,
        deletions: 1,
        binary: false,
        viewed: false,
        hidden: false,
    }
}

fn discussion(id: &str, resolved: bool) -> DiscussionInput {
    DiscussionInput {
        id: id.to_owned(),
        resolved,
        position: None,
        notes: vec![("Ann".to_owned(), format!("note of {id}"))],
    }
}

fn inputs(files: Vec<FileInput>, discussions: Vec<DiscussionInput>) -> BriefInputs {
    BriefInputs {
        kind: brief::Kind::Local { branch: None },
        base_sha: Some("base".to_owned()),
        head_sha: None,
        description: String::new(),
        commits: Vec::new(),
        files,
        discussions,
    }
}

#[test]
fn assembled_brief_lists_files_hidden_and_viewed() {
    let mut renamed = file("new.rs", FileStatus::Renamed);
    renamed.old_path = Some("old.rs".to_owned());
    let mut viewed = file("seen.rs", FileStatus::Modified);
    viewed.viewed = true;
    let mut hidden = file("lock.json", FileStatus::Modified);
    hidden.hidden = true;
    hidden.viewed = true;
    let mut binary = file("logo.png", FileStatus::Added);
    binary.binary = true;
    let brief = assemble(inputs(
        vec![
            renamed,
            viewed,
            hidden,
            binary,
            file("plain.rs", FileStatus::Deleted),
        ],
        Vec::new(),
    ));
    assert_eq!(brief.files.len(), 5);
    assert_eq!(brief.files[0].old_path.as_deref(), Some("old.rs"));
    assert_eq!(brief.files[0].status, brief::Status::Renamed);
    assert_eq!(brief.files[1].old_path, None);
    assert_eq!(
        (brief.files[1].added, brief.files[1].removed),
        (Some(3), Some(1))
    );
    assert_eq!((brief.files[3].added, brief.files[3].removed), (None, None));
    assert_eq!(brief.files[4].status, brief::Status::Deleted);
    assert_eq!(brief.viewed, vec!["seen.rs".to_owned()]);
    assert_eq!(brief.hidden, vec!["lock.json".to_owned()]);
    assert!(brief.disk_matches_head);
}

#[test]
fn assembled_brief_puts_unresolved_threads_first_in_a_stable_order() {
    let brief = assemble(inputs(
        Vec::new(),
        vec![
            discussion("a", true),
            discussion("b", false),
            discussion("c", true),
            discussion("d", false),
        ],
    ));
    let order: Vec<&str> = brief.threads.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(order, ["b", "d", "a", "c"]);
    assert_eq!(brief.threads[0].notes[0].author, "Ann");
    assert_eq!(brief.threads[0].notes[0].body, "note of b");
}

#[test]
fn the_size_cut_of_the_brief_keeps_unresolved_threads() {
    let mut loud = discussion("resolved-big", true);
    loud.notes = vec![("Bob".to_owned(), "x".repeat(brief::MAX_THREADS_CHARS))];
    let brief = assemble(inputs(
        Vec::new(),
        vec![loud, discussion("open-small", false)],
    ));
    let text = brief::render(&brief);
    assert!(text.contains("note of open-small"));
}

#[rstest]
#[case::same(Some("abc"), Some("abc"), true)]
#[case::other(Some("abc"), Some("abd"), false)]
#[case::unreadable(None, Some("abc"), false)]
#[case::no_head(Some("abc"), None, false)]
#[case::empty_head(Some(""), Some(""), false)]
fn disk_matches_compares_the_checkout_head(
    #[case] disk: Option<&str>,
    #[case] head: Option<&str>,
    #[case] expected: bool,
) {
    assert_eq!(disk_matches(disk, head), expected);
}

#[test]
fn language_names_are_english_names() {
    assert_eq!(language_name(Lang::En), "English");
    assert_eq!(language_name(Lang::Ru), "Russian");
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
        .args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn repository() -> (tempfile::TempDir, Repo, String) {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q"]);
    std::fs::write(dir.path().join("a.txt"), "a\n").unwrap();
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-qm", "one"]);
    let head = git(dir.path(), &["rev-parse", "HEAD"]);
    let repo = Repo::open(dir.path()).unwrap();
    (dir, repo, head)
}

#[test]
fn workdir_matches_when_the_checkout_is_at_the_reviewed_commit() {
    let (_dir, repo, head) = repository();
    let plan = WorkdirPlan {
        repo: repo.clone(),
        branch: None,
        head_sha: Some(head),
    };
    let workdir = resolve_workdir(&plan, None);
    assert_eq!(workdir.cwd, repo.root());
    assert!(workdir.disk_matches_head);
}

#[test]
fn workdir_differs_when_the_reviewed_commit_is_elsewhere() {
    let (_dir, repo, _) = repository();
    let plan = WorkdirPlan {
        repo: repo.clone(),
        branch: Some("no-such-branch".to_owned()),
        head_sha: Some("0".repeat(40)),
    };
    let workdir = resolve_workdir(&plan, None);
    assert_eq!(workdir.cwd, repo.root());
    assert!(!workdir.disk_matches_head);
}

#[test]
fn workdir_of_local_changes_is_the_root_and_always_matches() {
    let (_dir, repo, _) = repository();
    let plan = WorkdirPlan {
        repo: repo.clone(),
        branch: None,
        head_sha: None,
    };
    let workdir = resolve_workdir(&plan, None);
    assert_eq!(workdir.cwd, repo.root());
    assert!(workdir.disk_matches_head);
}

#[test]
fn workdir_keeps_the_given_checkout() {
    let (dir, repo, head) = repository();
    let plan = WorkdirPlan {
        repo,
        branch: None,
        head_sha: Some(head),
    };
    let workdir = resolve_workdir(&plan, Some(dir.path()));
    assert_eq!(workdir.cwd, dir.path());
    assert!(workdir.disk_matches_head);
}

fn overview(stored: Option<&str>, head: &str) -> OverviewState {
    overview_state(stored, head)
}

#[rstest]
#[case::none_auto(None, "h1", true, false, LoadAction::Overview)]
#[case::none_auto_local(None, "h1", true, true, LoadAction::Overview)]
#[case::none_manual(None, "h1", false, false, LoadAction::Nothing)]
#[case::none_manual_local(None, "h1", false, true, LoadAction::Nothing)]
#[case::current_auto(Some("h1"), "h1", true, false, LoadAction::Nothing)]
#[case::current_manual(Some("h1"), "h1", false, false, LoadAction::Nothing)]
#[case::current_auto_local(Some("h1"), "h1", true, true, LoadAction::Nothing)]
#[case::outdated_auto(Some("h0"), "h1", true, false, LoadAction::Update)]
#[case::outdated_manual(Some("h0"), "h1", false, false, LoadAction::Nothing)]
#[case::outdated_auto_local(Some("h0"), "h1", true, true, LoadAction::Nothing)]
#[case::outdated_manual_local(Some("h0"), "h1", false, true, LoadAction::Nothing)]
fn overview_decision_table(
    #[case] stored: Option<&str>,
    #[case] head: &str,
    #[case] auto: bool,
    #[case] local: bool,
    #[case] expected: LoadAction,
) {
    assert_eq!(load_action(&overview(stored, head), auto, local), expected);
}

#[test]
fn the_overview_button_updates_only_an_outdated_overview() {
    assert_eq!(
        Request::for_overview_button(&overview(Some("h0"), "h1")),
        Request::Update {
            old_head: "h0".to_owned()
        }
    );
    assert_eq!(
        Request::for_overview_button(&overview(Some("h1"), "h1")),
        Request::Overview
    );
    assert_eq!(
        Request::for_overview_button(&overview(None, "h1")),
        Request::Overview
    );
}

#[test]
fn requests_are_stored_as_the_matching_message_kind() {
    assert_eq!(Request::Overview.kind(), AgentKind::Overview);
    assert_eq!(
        Request::Update {
            old_head: String::new()
        }
        .kind(),
        AgentKind::Overview
    );
    assert_eq!(
        Request::Chat {
            prompt: String::new()
        }
        .kind(),
        AgentKind::Chat
    );
}

fn failure(kind: FailureKind, message: &str) -> Failure {
    Failure {
        kind,
        message: message.to_owned(),
    }
}

#[rstest]
#[case::login(FailureKind::NotLoggedIn, Problem::NotLoggedIn)]
#[case::limit(FailureKind::UsageLimit, Problem::UsageLimit)]
#[case::context(FailureKind::ContextTooLong, Problem::ContextTooLong)]
#[case::model(FailureKind::ModelNotFound, Problem::ModelNotFound)]
#[case::other(FailureKind::Other, Problem::Other("harness text".to_owned()))]
fn failures_are_classified_by_kind(#[case] kind: FailureKind, #[case] expected: Problem) {
    assert_eq!(classify(&failure(kind, "harness text")), expected);
}

#[test]
fn problem_text_shows_the_login_hint_of_the_harness() {
    let en = Lang::En.strings();
    let command = problem_text(
        &Problem::NotLoggedIn,
        Some(LoginHint::Command("claude auth login")),
        en,
    );
    assert!(command.contains("claude auth login"));
    let env = problem_text(
        &Problem::NotLoggedIn,
        Some(LoginHint::EnvVar("DEEPSEEK_API_KEY")),
        en,
    );
    assert!(env.contains("DEEPSEEK_API_KEY"));
    assert_eq!(
        problem_text(&Problem::NotLoggedIn, None, en),
        en.assistant_login_generic.to_string()
    );
}

#[test]
fn problem_text_is_localized_for_known_problems() {
    for lang in Lang::all() {
        let s = lang.strings();
        assert_eq!(
            problem_text(&Problem::UsageLimit, None, s),
            s.assistant_usage_limit.to_string()
        );
        assert_eq!(
            problem_text(&Problem::ContextTooLong, None, s),
            s.assistant_context_too_long.to_string()
        );
        assert_eq!(
            problem_text(&Problem::ModelNotFound, None, s),
            s.assistant_model_not_found.to_string()
        );
    }
}

#[test]
fn other_problems_show_the_harness_text_cut_to_500_characters() {
    let en = Lang::En.strings();
    let long = "é".repeat(900);
    let text = problem_text(&Problem::Other(long), None, en);
    assert!(text.starts_with("The agent failed: "));
    assert_eq!(text.chars().filter(|c| *c == 'é').count(), 500);
    assert!(text.ends_with('\u{2026}'));
    assert_eq!(
        problem_text(&Problem::Other("  \n ".to_owned()), None, en),
        en.assistant_failed_plain.to_string()
    );
}

#[rstest]
#[case::short("  boom \n", "boom")]
#[case::empty("", "")]
fn limit_text_trims(#[case] text: &str, #[case] expected: &str) {
    assert_eq!(limit_text(text), expected);
}

#[test]
fn limit_text_keeps_exactly_500_characters_whole() {
    let exact = "x".repeat(500);
    assert_eq!(limit_text(&exact), exact);
    assert_eq!(limit_text(&format!("{exact}y")).chars().count(), 501);
}

#[test]
fn an_unexpected_exit_names_its_reason_or_says_nothing_more() {
    let en = Lang::En.strings();
    assert!(exit_text(Some("segfault"), en).contains("segfault"));
    assert_eq!(exit_text(None, en), en.assistant_exited_plain.to_string());
    assert_eq!(
        exit_text(Some(" "), en),
        en.assistant_exited_plain.to_string()
    );
}

fn brief_with(threads: Vec<(&str, bool, &str)>) -> Brief {
    assemble(inputs(
        Vec::new(),
        threads
            .into_iter()
            .map(|(id, resolved, body)| DiscussionInput {
                id: id.to_owned(),
                resolved,
                position: None,
                notes: vec![("Ann".to_owned(), body.to_owned())],
            })
            .collect(),
    ))
}

#[test]
fn the_first_message_of_a_session_carries_the_whole_brief() {
    let brief = brief_with(vec![("t1", false, "please rename")]);
    let text = compose_chat(&brief, "what is this?", true, &HashMap::new());
    assert!(text.starts_with("# Review context"));
    assert!(text.contains("please rename"));
    assert!(text.ends_with("what is this?"));
}

#[test]
fn an_unchanged_thread_set_adds_no_prefix() {
    let brief = brief_with(vec![("t1", false, "please rename")]);
    let seen = brief::fingerprints(&brief.threads);
    assert_eq!(compose_chat(&brief, "and then?", false, &seen), "and then?");
}

#[test]
fn only_new_and_changed_threads_get_the_prefix() {
    let before = brief_with(vec![("t1", false, "please rename"), ("t2", false, "ok")]);
    let seen = brief::fingerprints(&before.threads);
    let after = brief_with(vec![
        ("t1", false, "please rename"),
        ("t2", true, "ok"),
        ("t3", false, "brand new"),
    ]);
    let text = compose_chat(&after, "next", false, &seen);
    assert!(text.starts_with("New and changed comments since the last message:"));
    assert!(text.contains("brand new"));
    assert!(!text.contains("please rename"));
    assert!(text.ends_with("next"));
    let unseen = logic::unseen_threads(&after, &seen);
    let ids: Vec<&str> = unseen.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, ["t3", "t2"]);
}

#[test]
fn a_session_resumed_in_a_new_run_resends_every_thread_once() {
    let brief = brief_with(vec![("t1", false, "please rename")]);
    let text = compose_chat(&brief, "hello", false, &HashMap::new());
    assert!(text.contains("please rename"));
}

#[test]
fn bursts_of_text_are_merged_but_other_events_keep_their_place() {
    let events = vec![
        Event::Text("a".to_owned()),
        Event::Text("b".to_owned()),
        Event::Thinking("t".to_owned()),
        Event::Thinking("u".to_owned()),
        Event::Text("c".to_owned()),
        Event::ToolStarted {
            id: "1".to_owned(),
            title: "git log".to_owned(),
        },
        Event::Text("d".to_owned()),
        Event::Text("e".to_owned()),
    ];
    assert_eq!(
        coalesce(events),
        vec![
            Event::Text("ab".to_owned()),
            Event::Thinking("tu".to_owned()),
            Event::Text("c".to_owned()),
            Event::ToolStarted {
                id: "1".to_owned(),
                title: "git log".to_owned()
            },
            Event::Text("de".to_owned()),
        ]
    );
    assert!(coalesce(Vec::new()).is_empty());
}
