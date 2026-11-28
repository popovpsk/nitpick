use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

use ::agent::brief::{self, Brief};
use ::agent::process::Environment;
use ::agent::{Entry, EntryRole, Event, Failure, FailureKind, Outcome, prompt};
use forge::ForgeKind;
use git_repo::Repo;
use gpui::SharedString;
use review_core::{DiffLink, FileStatus, Side};
use rstest::rstest;
use ui_resources::i18n::Lang;

use super::claims::{self, Claim};
use super::context::{
    BriefInputs, DiscussionInput, FileInput, WorkdirPlan, assemble, disk_matches, language_name,
    local_head, resolve_workdir, review_key, thread_position,
};
use super::links::{
    self, AskInputs, AskPlan, LinkKind, Target, locate_selection, patch_sides, plan_ask,
};
use super::logic::{
    self, Guard, Launching, LoadAction, OverviewState, Problem, Request, classify, coalesce,
    compose_chat, exit_text, limit_text, load_action, overview_state, problem_text,
};
use super::messages::{Kind, Role};
use super::{LinkPlan, Lookup, PendingSelection, Phase, locate_in, unread_after};
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
#[case::none_auto(None, "h1", true, false, false, LoadAction::Overview)]
#[case::none_auto_local(None, "h1", true, true, false, LoadAction::Overview)]
#[case::none_manual(None, "h1", false, false, false, LoadAction::Nothing)]
#[case::none_manual_local(None, "h1", false, true, false, LoadAction::Nothing)]
#[case::none_auto_with_an_earlier_conversation(None, "h1", true, false, true, LoadAction::Nothing)]
#[case::none_auto_local_with_an_earlier_conversation(
    None,
    "h1",
    true,
    true,
    true,
    LoadAction::Nothing
)]
#[case::current_auto(Some("h1"), "h1", true, false, false, LoadAction::Nothing)]
#[case::current_manual(Some("h1"), "h1", false, false, false, LoadAction::Nothing)]
#[case::current_auto_local(Some("h1"), "h1", true, true, false, LoadAction::Nothing)]
#[case::outdated_auto(Some("h0"), "h1", true, false, false, LoadAction::Update)]
#[case::outdated_manual(Some("h0"), "h1", false, false, false, LoadAction::Nothing)]
#[case::outdated_auto_local(Some("h0"), "h1", true, true, false, LoadAction::Nothing)]
#[case::outdated_manual_local(Some("h0"), "h1", false, true, false, LoadAction::Nothing)]
fn overview_decision_table(
    #[case] stored: Option<&str>,
    #[case] head: &str,
    #[case] auto: bool,
    #[case] local: bool,
    #[case] resumable: bool,
    #[case] expected: LoadAction,
) {
    assert_eq!(
        load_action(&overview(stored, head), auto, local, resumable),
        expected
    );
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
    assert_eq!(Request::Overview.kind(), Kind::Overview);
    assert_eq!(
        Request::Update {
            old_head: String::new()
        }
        .kind(),
        Kind::Overview
    );
    assert_eq!(
        Request::Chat {
            prompt: String::new()
        }
        .kind(),
        Kind::Chat
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
    let text = compose_chat(&brief, "what is this?", true, &HashMap::new(), None);
    assert!(text.starts_with("# Review context"));
    assert!(text.contains("please rename"));
    assert!(text.ends_with("what is this?"));
}

#[test]
fn an_unchanged_thread_set_adds_no_prefix() {
    let brief = brief_with(vec![("t1", false, "please rename")]);
    let seen = brief::fingerprints(&brief.threads);
    assert_eq!(
        compose_chat(&brief, "and then?", false, &seen, None),
        "and then?"
    );
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
    let text = compose_chat(&after, "next", false, &seen, None);
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
    let text = compose_chat(&brief, "hello", false, &HashMap::new(), None);
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

fn link(path: &str, range: Option<(Side, u32, u32)>) -> DiffLink {
    DiffLink {
        path: path.to_owned(),
        range,
    }
}

#[rstest]
#[case::diff("diff:a.rs#R1", LinkKind::Diff)]
#[case::diff_any_case("DIFF:a.rs", LinkKind::Diff)]
#[case::https("https://example.com", LinkKind::Browser)]
#[case::mailto("mailto:a@example.com", LinkKind::Browser)]
#[case::file("file:///etc/passwd", LinkKind::Ignored)]
#[case::javascript("javascript:alert(1)", LinkKind::Ignored)]
#[case::relative("src/a.rs", LinkKind::Ignored)]
#[case::empty("", LinkKind::Ignored)]
#[case::multibyte("diff\u{e9}x", LinkKind::Ignored)]
fn only_diff_and_web_links_are_followed(#[case] url: &str, #[case] expected: LinkKind) {
    assert_eq!(links::classify(url), expected);
}

const LINK_PATCH: &str = "diff --git a/a.rs b/a.rs
--- a/a.rs
+++ b/a.rs
@@ -1,3 +1,3 @@
 one
-two
+TWO
 three
";

fn plan(link: &DiffLink, files: &[(&str, Option<&str>)]) -> LinkPlan {
    let maps = review_core::line_maps(LINK_PATCH);
    links::plan_link(link, files.iter().copied(), |file| {
        link.locate(&maps[file.min(maps.len() - 1)])
    })
}

#[rstest]
#[case::new_line(link("a.rs", Some((Side::New, 2, 2))), Some((Side::New, 2)))]
#[case::old_line(link("a.rs", Some((Side::Old, 2, 2))), Some((Side::Old, 2)))]
#[case::range_outside_the_patch(
    link("a.rs", Some((Side::New, 50, 60))),
    None
)]
#[case::line_zero(
    link("a.rs", Some((Side::New, 0, 0))),
    None
)]
#[case::whole_file(link("a.rs", None), None)]
fn a_link_scrolls_to_the_first_line_the_patch_has(
    #[case] link: DiffLink,
    #[case] scroll: Option<(Side, u32)>,
) {
    assert_eq!(
        plan(&link, &[("a.rs", Some("a.rs"))]),
        LinkPlan::Open(Target { file: 0, scroll })
    );
}

#[test]
fn a_range_scrolls_to_its_first_line_inside_the_patch() {
    assert_eq!(
        plan(&link("a.rs", Some((Side::New, 0, 3))), &[("a.rs", None)]),
        LinkPlan::Open(Target {
            file: 0,
            scroll: Some((Side::New, 1))
        })
    );
}

#[rstest]
#[case::by_new_path("b.rs", Some(1))]
#[case::by_old_path("was.rs", Some(1))]
#[case::unknown("c.rs", None)]
fn a_link_finds_its_file_by_path_or_old_path(#[case] path: &str, #[case] file: Option<usize>) {
    let files = [("a.rs", None), ("b.rs", Some("was.rs"))];
    let expected = match file {
        Some(file) => LinkPlan::Open(Target { file, scroll: None }),
        None => LinkPlan::NotInReview,
    };
    assert_eq!(plan(&link(path, None), &files), expected);
}

#[test]
fn a_path_equal_to_one_file_and_the_old_path_of_another_prefers_the_current_name() {
    let files = [("x.rs", Some("a.rs")), ("a.rs", None)];
    assert_eq!(
        plan(&link("a.rs", None), &files),
        LinkPlan::Open(Target {
            file: 1,
            scroll: None
        })
    );
}

#[rstest]
#[case::same_side_ordered(Side::New, 4, Side::New, 9, (Side::New, 4, 9))]
#[case::same_side_reversed(Side::Old, 9, Side::Old, 4, (Side::Old, 4, 9))]
#[case::single_line(Side::New, 5, Side::New, 5, (Side::New, 5, 5))]
#[case::mixed_sides_end_on_the_pointer(Side::Old, 3, Side::New, 8, (Side::New, 8, 8))]
fn a_selection_becomes_one_side_range(
    #[case] side: Side,
    #[case] start: u32,
    #[case] end_side: Side,
    #[case] end: u32,
    #[case] expected: (Side, u32, u32),
) {
    assert_eq!(links::selection_span(side, start, end_side, end), expected);
}

#[rstest]
#[case::plain(false, false, Side::New, true)]
#[case::old_side_plain(false, false, Side::Old, true)]
#[case::commit_view(true, false, Side::New, false)]
#[case::interdiff_new(false, true, Side::New, true)]
#[case::interdiff_old(false, true, Side::Old, false)]
fn a_selection_is_linked_only_where_its_numbers_are_the_reviews(
    #[case] commit_view: bool,
    #[case] since: bool,
    #[case] side: Side,
    #[case] expected: bool,
) {
    assert_eq!(
        links::can_link_selection(commit_view, since, side),
        expected
    );
}

fn pending() -> PendingSelection {
    PendingSelection::new(
        "src/a.rs".to_owned(),
        Some((Side::New, 10, 12)),
        "let x = 1;\nlet y = 2;",
    )
}

#[test]
fn the_marker_is_the_link_the_agent_gets() {
    let pending = pending();
    assert_eq!(pending.marker, "[src/a.rs R10-12](diff:src/a.rs#R10-R12)");
}

#[test]
fn a_question_with_the_marker_carries_the_selected_text() {
    let pending = pending();
    let input = format!("why is {} here?", pending.marker);
    let question = links::compose_question(&input, Some(&pending));
    assert_eq!(question.shown, input);
    assert_eq!(
        question.prompt,
        "why is here?\n\nSelected: [src/a.rs R10-12](diff:src/a.rs#R10-R12)\n```\nlet x = 1;\nlet y = 2;\n```\n"
    );
}

#[rstest]
#[case::at_the_start("M why is this here?", "why is this here?")]
#[case::in_the_middle("why is M here?", "why is here?")]
#[case::at_the_end("why is this here? M", "why is this here?")]
#[case::marker_only("  M  ", "")]
#[case::multi_line_question(
    "first line\nM second line\n\nthird",
    "first line\nsecond line\n\nthird"
)]
#[case::own_line_between_paragraphs("One.\n\nM\n\nTwo.", "One.\n\nTwo.")]
#[case::own_line_between_lines("One.\nM\nTwo.", "One.\nTwo.")]
#[case::own_line_at_the_end("One.\n\nM", "One.")]
#[case::own_line_with_indentation("One.\n  M  \nTwo.", "One.\nTwo.")]
#[case::crlf("One.\r\nM\r\nTwo.", "One.\r\nTwo.")]
#[case::crlf_between_paragraphs("One.\r\n\r\nM\r\n\r\nTwo.", "One.\r\n\r\nTwo.")]
#[case::crlf_at_the_end("One.\r\n\r\nM", "One.")]
#[case::mixed_blank_line_crlf_then_lf("One.\r\n\nM\n\r\nTwo.", "One.\r\n\nTwo.")]
#[case::mixed_blank_line_lf_then_crlf("One.\n\r\nM\r\n\nTwo.", "One.\n\r\nTwo.")]
#[case::mixed_plain_lines("One.\nM\r\nTwo.", "One.\nTwo.")]
#[case::code_after_the_marker(
    "M what does this do?\n\n    fn f() {\n        g();\n    }",
    "what does this do?\n\n    fn f() {\n        g();\n    }"
)]
#[case::code_before_the_marker(
    "    let x = 1;\n    let y = 2;\nM why?",
    "let x = 1;\n    let y = 2;\nwhy?"
)]
#[case::spaces_inside_a_line_stay("a   b M c    d", "a   b c    d")]
fn only_the_marker_and_its_seam_leave_the_question(#[case] input: &str, #[case] expected: &str) {
    let marker = "[m](diff:a#R1)";
    let input = input.replace('M', marker);
    assert_eq!(links::without_marker(&input, marker), expected);
}

#[test]
fn a_multi_line_question_keeps_its_lines_in_the_prompt() {
    let pending = pending();
    let input = format!("why?\n\n{}\n\n    indented\n- item", pending.marker);
    let question = links::compose_question(&input, Some(&pending));
    assert_eq!(question.shown, input);
    assert!(
        question
            .prompt
            .starts_with("why?\n\n    indented\n- item\n\nSelected: [src/a.rs R10-12]")
    );
}

#[test]
fn a_question_that_is_only_the_marker_sends_only_the_selection() {
    let pending = pending();
    let question = links::compose_question(&format!("  {}  ", pending.marker), Some(&pending));
    assert_eq!(question.shown, pending.marker);
    assert!(question.prompt.starts_with("Selected: [src/a.rs R10-12]"));
}

#[rstest]
#[case::deleted_marker("what does this do?")]
#[case::edited_marker("what does [src/a.rs R10-13](diff:src/a.rs#R10-R13) do?")]
fn a_question_without_the_marker_goes_out_as_typed(#[case] input: &str) {
    let question = links::compose_question(input, Some(&pending()));
    assert_eq!(question.shown, input);
    assert_eq!(question.prompt, input);
}

#[test]
fn a_question_without_a_pending_selection_is_trimmed_text() {
    let question = links::compose_question("  hello \n", None);
    assert_eq!(question.shown, "hello");
    assert_eq!(question.prompt, "hello");
}

#[test]
fn a_long_selection_is_cut_at_a_character_boundary() {
    let text = "é".repeat(30_000);
    let pending = PendingSelection::new("a.rs".to_owned(), Some((Side::New, 1, 1)), &text);
    assert_eq!(pending.text.chars().count(), 20_000);
}

#[rstest]
#[case::empty_input("", None, "[m](diff:a#R1) ")]
#[case::appended_after_text("why", None, "why [m](diff:a#R1) ")]
#[case::appended_after_whitespace("why ", None, "why [m](diff:a#R1) ")]
#[case::replaces_the_old_marker(
    "see [o](diff:a#R2) now",
    Some("[o](diff:a#R2)"),
    "see [m](diff:a#R1) now"
)]
#[case::old_marker_already_gone("see now", Some("[o](diff:a#R2)"), "see now [m](diff:a#R1) ")]
fn the_marker_goes_into_the_input(
    #[case] current: &str,
    #[case] old: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(
        links::input_with_marker(current, old, "[m](diff:a#R1)"),
        expected
    );
}

fn failed() -> Outcome {
    Outcome::Failed(failure(FailureKind::Other, "boom"))
}

#[rstest]
#[case::first_turn_completed(false, true, Outcome::Completed, true)]
#[case::first_turn_interrupted(false, true, Outcome::Interrupted, true)]
#[case::first_turn_failed_stays_pending(false, true, failed(), false)]
#[case::first_turn_lost_stays_pending(
    false,
    true,
    Outcome::Failed(failure(FailureKind::SessionLost, "gone")),
    false
)]
#[case::turn_without_brief_proves_nothing(false, false, Outcome::Completed, false)]
#[case::cancelled_with_brief_delivers_nothing(false, true, Outcome::Cancelled, false)]
#[case::cancelled_without_brief_delivers_nothing(false, false, Outcome::Cancelled, false)]
#[case::delivered_stays_after_cancel(true, true, Outcome::Cancelled, true)]
#[case::delivered_stays_after_failure(true, false, failed(), true)]
#[case::delivered_stays_after_chat(true, false, Outcome::Completed, true)]
#[case::delivered_stays_when_carried_again_fails(true, true, failed(), true)]
fn the_brief_counts_as_delivered_when_an_answered_turn_carried_it(
    #[case] was: bool,
    #[case] carried: bool,
    #[case] outcome: Outcome,
    #[case] expected: bool,
) {
    assert_eq!(logic::brief_delivered(was, carried, &outcome), expected);
}

#[rstest]
#[case::completed(Outcome::Completed, true)]
#[case::interrupted(Outcome::Interrupted, true)]
#[case::cancelled(Outcome::Cancelled, false)]
#[case::failed(failed(), false)]
fn only_a_completed_or_stopped_turn_counts_as_answered(
    #[case] outcome: Outcome,
    #[case] expected: bool,
) {
    assert_eq!(logic::answered(&outcome), expected);
}

fn stored_session(id: &str, delivered: bool, instructions: &str) -> app_db::AgentSession {
    app_db::AgentSession {
        id: id.to_string(),
        brief_delivered: delivered,
        brief_head: "h1".to_string(),
        instructions: instructions.to_string(),
    }
}

#[test]
fn a_stored_session_is_adopted_with_its_flags_when_the_claim_succeeds() {
    let (resume, claim) = logic::adopt(Some(stored_session("adopt-a", true, "fp")), |id| {
        Claim::acquire(id, 1)
    })
    .unwrap();
    assert_eq!(
        resume,
        logic::Resume {
            id: "adopt-a".to_string(),
            brief_delivered: true,
            brief_head: "h1".to_string(),
            instructions: "fp".to_string(),
        }
    );
    assert_eq!(claim.id(), "adopt-a");
}

#[test]
fn a_session_another_window_holds_is_not_adopted() {
    let held = Claim::acquire("adopt-b", 1).unwrap();
    let taken = logic::adopt(Some(stored_session("adopt-b", true, "fp")), |id| {
        Claim::acquire(id, 2)
    });
    assert!(taken.is_none());
    drop(held);
    assert!(
        logic::adopt(Some(stored_session("adopt-b", true, "fp")), |id| {
            Claim::acquire(id, 2)
        })
        .is_some()
    );
}

#[test]
fn nothing_stored_adopts_nothing() {
    assert!(logic::adopt(None, |id| Claim::acquire(id, 1)).is_none());
}

#[test]
fn a_claim_is_held_until_dropped_and_only_by_one_owner() {
    let id = "claim-c";
    assert!(!claims::is_held(id));
    let first = Claim::acquire(id, 1).unwrap();
    assert!(claims::is_held(id));
    assert!(Claim::acquire(id, 1).is_none());
    assert!(Claim::acquire(id, 2).is_none());
    drop(first);
    assert!(!claims::is_held(id));
    assert!(Claim::acquire(id, 2).is_some());
}

#[test]
fn owners_are_distinct() {
    assert_ne!(claims::next_owner(), claims::next_owner());
}

#[rstest]
#[case::chat_after_an_answered_overview(true, Some(true), true)]
#[case::chat_before_the_brief_went_out(true, Some(false), false)]
#[case::chat_without_a_process(true, None, false)]
#[case::overview_always_carries_the_brief(false, Some(true), false)]
#[case::overview_without_a_process(false, None, false)]
fn a_chat_goes_out_at_once_only_on_a_live_session_that_has_the_brief(
    #[case] chat: bool,
    #[case] live_delivered: Option<bool>,
    #[case] expected: bool,
) {
    assert_eq!(logic::goes_out_at_once(chat, live_delivered), expected);
}

#[test]
fn a_chat_after_an_overview_continues_the_same_process() {
    let overview = logic::brief_delivered(false, true, &Outcome::Completed);
    assert!(overview);
    assert!(logic::goes_out_at_once(true, Some(overview)));
    let stopped = logic::brief_delivered(false, true, &Outcome::Interrupted);
    assert!(logic::goes_out_at_once(true, Some(stopped)));
    let failed_overview = logic::brief_delivered(false, true, &failed());
    assert!(!logic::goes_out_at_once(true, Some(failed_overview)));
}

fn resume_with(instructions: &str) -> logic::Resume {
    logic::Resume {
        id: "s".to_string(),
        brief_delivered: true,
        brief_head: "h1".to_string(),
        instructions: instructions.to_string(),
    }
}

#[test]
fn a_resumed_session_gets_the_instructions_again_only_when_they_differ() {
    assert!(!logic::instructions_due(Some(&resume_with("a")), "a"));
    assert!(logic::instructions_due(Some(&resume_with("a")), "b"));
    assert!(logic::instructions_due(Some(&resume_with("")), "b"));
    assert!(!logic::instructions_due(None, "b"));
}

#[test]
fn updated_instructions_lead_the_first_message_of_a_resumed_session() {
    let sent = logic::with_updated_instructions(Some("RULES"), "question".to_string());
    assert!(sent.starts_with("The instructions for this conversation were updated."));
    assert!(sent.ends_with("RULES\n--- end of the updated instructions ---\n\nquestion"));
    assert_eq!(
        logic::with_updated_instructions(None, "question".to_string()),
        "question"
    );
}

#[rstest]
#[case::live_session_with_messages(true, true, false, true)]
#[case::nothing_ran(false, true, false, false)]
#[case::empty_conversation(true, false, false, false)]
#[case::already_noticed(true, true, true, false)]
fn a_settings_notice_needs_a_used_session_and_no_notice_before_it(
    #[case] used: bool,
    #[case] has_messages: bool,
    #[case] last_is_notice: bool,
    #[case] expected: bool,
) {
    assert_eq!(
        logic::settings_notice(used, has_messages, last_is_notice),
        expected
    );
}

#[rstest]
#[case::same("claude", "claude", None, "claude", "claude", None, true)]
#[case::padding(
    "claude",
    "claude --model x",
    Some("opus"),
    "claude",
    " claude --model x ",
    Some(" opus "),
    true
)]
#[case::blank_model_is_none("claude", "claude", None, "claude", "claude", Some("  "), true)]
#[case::command("claude", "claude", None, "claude", "claude --model", None, false)]
#[case::model("claude", "claude", Some("a"), "claude", "claude", Some("b"), false)]
#[case::profile("claude", "claude", None, "codex", "claude", None, false)]
fn launch_settings_compare_after_trimming(
    #[case] old_profile: &str,
    #[case] old_command: &str,
    #[case] old_model: Option<&str>,
    #[case] new_profile: &str,
    #[case] new_command: &str,
    #[case] new_model: Option<&str>,
    #[case] same: bool,
) {
    assert_eq!(
        Launching::new(old_profile, old_command, old_model, "")
            == Launching::new(new_profile, new_command, new_model, ""),
        same
    );
}

#[rstest]
#[case::same("Be terse.", "Be terse.", true)]
#[case::padding("Be terse.", "  Be terse.\n", true)]
#[case::blank_is_empty("", "  \n ", true)]
#[case::changed("Be terse.", "Be verbose.", false)]
#[case::added("", "Be terse.", false)]
#[case::removed("Be terse.", "", false)]
fn a_change_of_the_users_instructions_counts_as_a_launch_change(
    #[case] old: &str,
    #[case] new: &str,
    #[case] same: bool,
) {
    assert_eq!(
        Launching::new("claude", "claude", None, old)
            == Launching::new("claude", "claude", None, new),
        same
    );
}

#[test]
fn an_untaken_guard_disposes_its_value_when_dropped() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let disposed = Arc::new(AtomicUsize::new(0));
    fn dispose(count: Arc<AtomicUsize>) {
        count.fetch_add(1, Ordering::SeqCst);
    }
    drop(Guard::new(Arc::clone(&disposed), dispose));
    assert_eq!(disposed.load(Ordering::SeqCst), 1);

    let mut taken = Guard::new(Arc::clone(&disposed), dispose);
    assert!(taken.take().is_some());
    assert!(taken.take().is_none());
    drop(taken);
    assert_eq!(disposed.load(Ordering::SeqCst), 1);
}

#[test]
fn a_guard_dropped_on_another_thread_still_disposes() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let disposed = Arc::new(AtomicUsize::new(0));
    fn dispose(count: Arc<AtomicUsize>) {
        count.fetch_add(1, Ordering::SeqCst);
    }
    let guard = Guard::new(Arc::clone(&disposed), dispose);
    std::thread::spawn(move || drop(guard)).join().unwrap();
    assert_eq!(disposed.load(Ordering::SeqCst), 1);
}

#[rstest]
#[case::cancelled_overview(Request::Overview, Outcome::Cancelled, Some("h0"), Some("h1"))]
#[case::cancelled_overview_unset(Request::Overview, Outcome::Cancelled, None, Some("h1"))]
#[case::cancelled_update(Request::Update { old_head: "h0".to_owned() }, Outcome::Cancelled, None, Some("h1"))]
#[case::cancelled_chat(Request::Chat { prompt: "q".to_owned() }, Outcome::Cancelled, Some("h0"), Some("h0"))]
#[case::cancelled_chat_unset(Request::Chat { prompt: "q".to_owned() }, Outcome::Cancelled, None, None)]
#[case::completed_overview(Request::Overview, Outcome::Completed, Some("h0"), Some("h0"))]
#[case::interrupted_overview(Request::Overview, Outcome::Interrupted, None, None)]
#[case::failed_overview(Request::Overview, failed(), None, None)]
fn a_cancelled_overview_request_is_not_retried_for_the_same_head(
    #[case] request: Request,
    #[case] outcome: Outcome,
    #[case] current: Option<&str>,
    #[case] expected: Option<&str>,
) {
    assert_eq!(
        logic::auto_attempted_after(current, request.kind(), "h1", &outcome).as_deref(),
        expected
    );
}

#[rstest]
#[case::session_still_there(true, false, false)]
#[case::session_just_started(false, true, false)]
#[case::session_lost_meanwhile(false, false, true)]
#[case::both_present(true, true, false)]
fn a_dispatch_without_a_live_session_starts_one(
    #[case] live: bool,
    #[case] started_now: bool,
    #[case] expected: bool,
) {
    assert_eq!(logic::must_start(live, started_now), expected);
}

const SECTION: &str = "\
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,5 +1,6 @@
 fn main() {
-    let old = 1;
+    let added = 1;
+    let wrapped = a_rather_long_call(first, second, third);
     helper();
 }
 
@@ -10,3 +11,3 @@
 fn helper() {
-    -- not a header
+    ++ not a header
 }
\\ No newline at end of file
";

#[test]
fn a_patch_section_gives_each_side_its_lines_and_numbers() {
    let (old, new) = patch_sides(SECTION);
    assert_eq!(old.numbers, [1, 2, 3, 4, 5, 10, 11, 12]);
    assert_eq!(new.numbers, [1, 2, 3, 4, 5, 6, 11, 12, 13]);
    assert!(old.text.contains("    let old = 1;\n    helper();"));
    assert!(old.text.contains("    -- not a header"));
    assert!(new.text.contains("    ++ not a header"));
    assert!(!new.text.contains("diff --git"));
}

#[rstest]
#[case::inside_one_added_line("a_rather_long", None, Some((Side::New, 3, 3)))]
#[case::whole_added_line("    let added = 1;", None, Some((Side::New, 2, 2)))]
#[case::two_lines_with_a_trailing_break("    let added = 1;\n    let wrapped = a_rather_long_call(first, second, third);\n", None, Some((Side::New, 2, 3)))]
#[case::a_deleted_line("let old", None, Some((Side::Old, 2, 2)))]
#[case::across_the_hunk_gap("}\n\nfn helper() {", None, Some((Side::New, 5, 11)))]
#[case::missing("nowhere in the file", None, None)]
#[case::empty("", None, None)]
#[case::only_breaks("\n\n", None, None)]
#[case::crlf("let added = 1;\r\n    let wrapped", None, Some((Side::New, 2, 3)))]
#[case::a_context_line_prefers_the_new_side("fn main()", None, Some((Side::New, 1, 1)))]
#[case::a_context_line_follows_the_hovered_side("fn main()", Some((Side::Old, 1)), Some((Side::Old, 1, 1)))]
#[case::a_repeated_line_follows_the_hovered_line("}", Some((Side::New, 13)), Some((Side::New, 13, 13)))]
#[case::a_repeated_line_without_a_hover_takes_the_first("}", None, Some((Side::New, 5, 5)))]
fn a_text_selection_is_located_in_the_patch(
    #[case] text: &str,
    #[case] hovered: Option<(Side, u32)>,
    #[case] expected: Option<(Side, u32, u32)>,
) {
    let (old, new) = patch_sides(SECTION);
    assert_eq!(locate_selection(&old, &new, text, hovered), expected);
}

fn ask(
    gutter: Option<(usize, Side, u32, Side, u32)>,
    has_text: bool,
    text_file: Option<usize>,
    commit_view: bool,
    interdiff: bool,
) -> AskPlan {
    plan_ask(&AskInputs {
        gutter,
        has_text,
        text_file,
        commit_view,
        interdiff,
    })
}

#[rstest]
#[case::gutter_range(
    ask(Some((2, Side::New, 5, Side::New, 9)), true, Some(0), false, false),
    AskPlan::Attach { file: 2, range: Some((Side::New, 5, 9)) }
)]
#[case::gutter_range_ends_on_the_other_side(
    ask(Some((2, Side::Old, 5, Side::New, 9)), true, Some(0), false, false),
    AskPlan::Attach { file: 2, range: Some((Side::New, 9, 9)) }
)]
#[case::gutter_in_a_commit_view_is_refused(
    ask(Some((2, Side::New, 5, Side::New, 9)), true, Some(0), true, false),
    AskPlan::Refuse
)]
#[case::gutter_on_the_old_side_of_an_interdiff_links_the_file(
    ask(Some((2, Side::Old, 5, Side::Old, 9)), true, Some(0), false, true),
    AskPlan::Attach { file: 2, range: None }
)]
#[case::text_is_located(ask(None, true, Some(3), false, false), AskPlan::Locate { file: 3 })]
#[case::text_in_a_commit_view_is_refused(ask(None, true, Some(3), true, false), AskPlan::Refuse)]
#[case::text_in_an_interdiff_links_the_file(
    ask(None, true, Some(3), false, true),
    AskPlan::Attach { file: 3, range: None }
)]
#[case::no_text_only_opens(ask(None, false, Some(3), false, false), AskPlan::OpenOnly)]
#[case::no_text_in_a_commit_view_only_opens(
    ask(None, false, Some(3), true, false),
    AskPlan::OpenOnly
)]
#[case::no_file_only_opens(ask(None, true, None, false, false), AskPlan::OpenOnly)]
fn the_ask_plan_follows_what_is_selected(#[case] plan: AskPlan, #[case] expected: AskPlan) {
    assert_eq!(plan, expected);
}

#[test]
fn a_selection_without_lines_puts_the_file_link_and_the_text_in_the_question() {
    let pending = PendingSelection::new("docs/a b.md".to_owned(), None, "some prose");
    assert_eq!(pending.marker, "[docs/a b.md](diff:docs/a%20b.md)");
    let input = format!("why? {}", pending.marker);
    let question = links::compose_question(&input, Some(&pending));
    assert_eq!(question.shown, input);
    assert!(question.prompt.starts_with("why?"));
    assert!(
        question
            .prompt
            .contains("Selected: [docs/a%20b.md](diff:docs/a%20b.md)\n")
    );
    assert!(question.prompt.contains("some prose"));
    assert_eq!(
        review_core::diff_link::parse("diff:docs/a%20b.md").map(|link| link.range),
        Some(None)
    );
}

#[test]
fn locating_a_long_file_counts_lines_once() {
    let body: String = (0..200_000).map(|n| format!("+line {n} same\n")).collect();
    let section = format!("@@ -0,0 +1,200000 @@\n{body}");
    let (old, new) = patch_sides(&section);
    let started = std::time::Instant::now();
    let found = locate_selection(&old, &new, "same", Some((Side::New, 150_000)));
    assert_eq!(found, Some((Side::New, 150_000, 150_000)));
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
}

fn resume_for(delivered: bool, head: &str) -> logic::Resume {
    logic::Resume {
        id: "s".to_string(),
        brief_delivered: delivered,
        brief_head: head.to_string(),
        instructions: String::new(),
    }
}

#[rstest]
#[case::same_head(Some(resume_for(true, "h1")), "h1", (true, None))]
#[case::head_moved(Some(resume_for(true, "h1")), "h2", (false, Some("h1".to_string())))]
#[case::never_delivered(Some(resume_for(false, "")), "h2", (false, None))]
#[case::legacy_row_without_a_head(Some(resume_for(true, "")), "h2", (false, None))]
#[case::new_session(None, "h2", (false, None))]
fn a_resumed_session_has_the_brief_only_for_the_head_it_was_sent_for(
    #[case] resume: Option<logic::Resume>,
    #[case] head: &str,
    #[case] expected: (bool, Option<String>),
) {
    assert_eq!(logic::resumed_brief(resume.as_ref(), head), expected);
}

#[test]
fn a_chat_after_the_head_moved_carries_the_brief_and_a_note() {
    let brief = brief_with(vec![]);
    let text = compose_chat(
        &brief,
        "still fine?",
        true,
        &HashMap::new(),
        Some(("aaa111", "bbb222")),
    );
    assert!(text.contains("The review moved from aaa111 to bbb222 since you last saw it."));
    assert!(text.contains("git diff aaa111 bbb222"));
    assert!(text.ends_with("still fine?"));
    let local = compose_chat(
        &brief,
        "still fine?",
        true,
        &HashMap::new(),
        Some(("local:1", "local:2")),
    );
    assert!(local.contains("The review changed since you last saw it."));
    assert!(!local.contains("git diff local"));
}

#[test]
fn a_started_session_is_handed_to_the_panel_only_while_its_dispatch_is_current() {
    let id = "hand-over-x";
    let mut slot: Option<Claim> = None;
    let cancelled = Claim::acquire(id, 1).unwrap();
    assert!(!logic::hand_over(&mut slot, 5, 4, cancelled));
    assert!(slot.is_none());
    assert!(!claims::is_held(id));

    let kept = Claim::acquire(id, 1).unwrap();
    assert!(logic::hand_over(&mut slot, 5, 5, kept));
    assert!(claims::is_held(id));
    assert!(Claim::acquire(id, 2).is_none());
    let next = slot.as_ref().map(|claim| claim.id().to_owned());
    assert_eq!(next.as_deref(), Some(id));
    drop(slot);
    assert!(!claims::is_held(id));
}

#[test]
fn a_clone_of_a_claim_keeps_the_session_held_until_the_last_one_is_dropped() {
    let id = "clone-held-x";
    let claim = Claim::acquire(id, 1).unwrap();
    let hold = claim.clone();
    drop(claim);
    assert!(claims::is_held(id));
    drop(hold);
    assert!(!claims::is_held(id));
}

#[test]
fn a_new_conversation_leaves_a_session_another_window_holds() {
    let mine = stored_session("clear-mine", true, "fp");
    let theirs = stored_session("clear-theirs", true, "fp");
    let held = Claim::acquire("clear-theirs", 2).unwrap();
    assert_eq!(logic::clearable(Some(&mine)).as_deref(), Some("clear-mine"));
    assert_eq!(logic::clearable(Some(&theirs)), None);
    assert_eq!(logic::clearable(None), None);
    drop(held);
    assert_eq!(
        logic::clearable(Some(&theirs)).as_deref(),
        Some("clear-theirs")
    );
}

const OLD_HEAD: &str = "1111111111111111111111111111111111111111";
const NEW_HEAD: &str = "2222222222222222222222222222222222222222";

fn said(text: String) -> Entry {
    Entry {
        role: EntryRole::User,
        text,
    }
}

fn answer(text: &str) -> Entry {
    Entry {
        role: EntryRole::Assistant,
        text: text.into(),
    }
}

fn restored_row(role: Role, kind: Kind, head: &str, text: &str) -> logic::Restored {
    logic::Restored {
        role,
        kind,
        head: head.into(),
        text: text.into(),
    }
}

fn brief_at(head: &str) -> Brief {
    let mut brief = brief_with(vec![("t1", false, "please rename")]);
    brief.kind = brief::Kind::Branches {
        source: "feature".into(),
        target: "main".into(),
    };
    brief.head_sha = Some(head.into());
    brief.base_sha = Some("3333333333333333333333333333333333333333".into());
    brief
}

#[test]
fn a_restored_conversation_shows_what_the_user_saw() {
    let old = brief_at(OLD_HEAD);
    let new = brief_at(NEW_HEAD);
    let seen = HashMap::new();
    let selection = prompt::Selection {
        path: "src/a.rs",
        range: Some((Side::New, 3, 4)),
        text: "let x = 1;\n",
    };
    let entries = vec![
        said(logic::compose(
            &Request::Overview,
            &old,
            false,
            OLD_HEAD,
            true,
            &seen,
            None,
        )),
        answer("The overview."),
        said(compose_chat(&old, "Why the cache?", false, &seen, None)),
        answer("Speed."),
        said(logic::with_updated_instructions(
            Some(&prompt::instructions("Russian", true, "Be short.")),
            compose_chat(
                &new,
                &prompt::selection_question("And this?", &selection),
                true,
                &seen,
                Some((OLD_HEAD, NEW_HEAD)),
            ),
        )),
        answer("A constant."),
        said(logic::compose(
            &Request::Update {
                old_head: OLD_HEAD.into(),
            },
            &new,
            false,
            NEW_HEAD,
            true,
            &seen,
            None,
        )),
        answer("The updated overview."),
    ];
    assert_eq!(
        logic::restored(&entries, "fallback"),
        vec![
            restored_row(Role::Assistant, Kind::Overview, OLD_HEAD, "The overview."),
            restored_row(Role::User, Kind::Chat, OLD_HEAD, "Why the cache?"),
            restored_row(Role::Assistant, Kind::Chat, OLD_HEAD, "Speed."),
            restored_row(
                Role::User,
                Kind::Chat,
                NEW_HEAD,
                "And this?\n\n[src/a.rs R3-4](diff:src/a.rs#R3-R4)"
            ),
            restored_row(Role::Assistant, Kind::Chat, NEW_HEAD, "A constant."),
            restored_row(
                Role::Assistant,
                Kind::Overview,
                NEW_HEAD,
                "The updated overview."
            ),
        ]
    );
}

#[test]
fn a_restored_conversation_without_a_brief_uses_the_fallback_head() {
    let entries = vec![said("hello".into()), answer("hi"), answer("  ")];
    assert_eq!(
        logic::restored(&entries, "local:abc"),
        vec![
            restored_row(Role::User, Kind::Chat, "local:abc", "hello"),
            restored_row(Role::Assistant, Kind::Chat, "local:abc", "hi"),
        ]
    );
}

#[test]
fn the_latest_restored_overview_decides_the_overview_state() {
    let entries = vec![
        said(logic::compose(
            &Request::Overview,
            &brief_at(OLD_HEAD),
            false,
            OLD_HEAD,
            true,
            &HashMap::new(),
            None,
        )),
        answer("The overview."),
    ];
    let rows = logic::restored(&entries, "");
    let latest = rows
        .iter()
        .rev()
        .find(|row| row.kind == Kind::Overview)
        .map(|row| row.head.as_str());
    assert_eq!(
        overview_state(latest, NEW_HEAD),
        OverviewState::Outdated {
            stored: OLD_HEAD.into()
        }
    );
}
