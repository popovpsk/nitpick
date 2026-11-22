use futures_executor::block_on;
use tempfile::TempDir;

use super::*;

const NOW: i64 = 1_800_000_000;

fn marks(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(path, fingerprint)| (path.to_string(), fingerprint.to_string()))
        .collect()
}

fn loaded(db: &Db, repo: &str, now: i64) -> Vec<(String, String)> {
    let mut list: Vec<(String, String)> = block_on(db.load_viewed(repo, now))
        .unwrap()
        .into_iter()
        .map(|mark| (mark.path, mark.fingerprint))
        .collect();
    list.sort();
    list
}

#[test]
fn viewed_marks_are_kept_per_repository() {
    let db = Db::open_in_memory().unwrap();
    drop(db.set_viewed(
        "/a",
        marks(&[("x.rs", "1..2"), ("y.rs", "3..4")]),
        true,
        NOW,
    ));
    drop(db.set_viewed("/b", marks(&[("x.rs", "9..9")]), true, NOW));
    drop(db.set_viewed("/a", marks(&[("y.rs", "3..4")]), false, NOW));
    assert_eq!(loaded(&db, "/a", NOW), marks(&[("x.rs", "1..2")]));
    assert_eq!(loaded(&db, "/b", NOW), marks(&[("x.rs", "9..9")]));
}

#[test]
fn queued_writes_apply_in_call_order() {
    let db = Db::open_in_memory().unwrap();
    for round in 0..50 {
        let viewed = round % 2 == 0;
        drop(db.set_viewed("/a", marks(&[("x.rs", "1..2")]), viewed, NOW));
    }
    block_on(db.barrier()).unwrap();
    assert!(loaded(&db, "/a", NOW).is_empty());
}

#[test]
fn old_marks_expire_on_load() {
    let db = Db::open_in_memory().unwrap();
    let old = NOW - VIEWED_RETENTION_SECS - 1;
    drop(db.set_viewed("/a", marks(&[("old.rs", "1..2")]), true, old));
    drop(db.set_viewed("/a", marks(&[("new.rs", "3..4")]), true, NOW));
    assert_eq!(loaded(&db, "/a", NOW), marks(&[("new.rs", "3..4")]));
}

#[test]
fn avatars_round_trip_and_age_out() {
    let db = Db::open_in_memory().unwrap();
    block_on(db.put_avatar("u1", vec![1, 2, 3], NOW)).unwrap();
    block_on(db.put_avatar("u2", vec![4], NOW - AVATAR_RETENTION_SECS - 1)).unwrap();
    assert_eq!(
        block_on(db.avatar("u1")).unwrap(),
        Some((vec![1, 2, 3], NOW))
    );
    assert_eq!(block_on(db.prune_avatars(NOW)).unwrap(), 1);
    assert_eq!(block_on(db.avatar("u2")).unwrap(), None);
}

#[test]
fn a_file_database_survives_reopening_and_refuses_a_newer_schema() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("nested").join("nitpick.db");
    let db = Db::open(&path).unwrap();
    block_on(db.set_viewed("/a", marks(&[("x.rs", "1..2")]), true, NOW)).unwrap();
    drop(db);
    let again = Db::open(&path).unwrap();
    assert_eq!(loaded(&again, "/a", NOW), marks(&[("x.rs", "1..2")]));
    drop(again);
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.pragma_update(None, "user_version", 99).unwrap();
    drop(conn);
    assert!(matches!(Db::open(&path), Err(Error::TooNew(99))));
}

#[test]
fn two_processes_can_create_the_database_together() {
    for _ in 0..20 {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nitpick.db");
        let opens: Vec<_> = (0..2)
            .map(|_| {
                let path = path.clone();
                std::thread::spawn(move || Db::open(&path).map(|_| ()))
            })
            .collect();
        for open in opens {
            open.join().unwrap().unwrap();
        }
    }
}

fn message(role: AgentRole, kind: AgentKind, text: &str, head: &str, at: i64) -> NewAgentMessage {
    NewAgentMessage {
        role,
        kind,
        text: text.to_string(),
        head: head.to_string(),
        created_at: at,
    }
}

fn texts(db: &Db, repo: &str, review: &str) -> Vec<String> {
    block_on(db.agent_messages(repo, review))
        .unwrap()
        .into_iter()
        .map(|message| message.text)
        .collect()
}

#[test]
fn a_version_one_database_upgrades_without_losing_viewed_marks() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("nitpick.db");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(MIGRATIONS[0]).unwrap();
    conn.pragma_update(None, "user_version", 1).unwrap();
    conn.execute(
        "INSERT INTO viewed (repo, path, fingerprint, viewed_at) VALUES ('/a', 'x.rs', '1..2', ?1)",
        [NOW],
    )
    .unwrap();
    drop(conn);

    let db = Db::open(&path).unwrap();
    assert_eq!(loaded(&db, "/a", NOW), marks(&[("x.rs", "1..2")]));
    block_on(db.set_agent_session("/a", "worktree", "claude", "s1", NOW)).unwrap();
    assert_eq!(
        block_on(db.agent_session("/a", "worktree", "claude")).unwrap(),
        Some("s1".to_string())
    );
    drop(db);

    let conn = rusqlite::Connection::open(&path).unwrap();
    let version: i64 = conn
        .query_row("SELECT user_version FROM pragma_user_version", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(version, MIGRATIONS.len() as i64);
    assert_eq!(version, 2);
}

#[test]
fn agent_messages_come_back_in_insertion_order() {
    let db = Db::open_in_memory().unwrap();
    let first = block_on(db.add_agent_message(
        "/a",
        "worktree",
        message(
            AgentRole::Assistant,
            AgentKind::Overview,
            "overview",
            "h1",
            NOW,
        ),
    ))
    .unwrap();
    for (text, at) in [("second", NOW + 5), ("third", NOW - 100), ("fourth", NOW)] {
        drop(db.add_agent_message(
            "/a",
            "worktree",
            message(AgentRole::User, AgentKind::Chat, text, "h1", at),
        ));
    }
    drop(db.add_agent_message(
        "/a",
        "branches:main..topic",
        message(AgentRole::Notice, AgentKind::Chat, "elsewhere", "h2", NOW),
    ));
    drop(db.add_agent_message(
        "/b",
        "worktree",
        message(AgentRole::Notice, AgentKind::Chat, "other repo", "h2", NOW),
    ));
    assert_eq!(
        texts(&db, "/a", "worktree"),
        ["overview", "second", "third", "fourth"]
    );
    let all = block_on(db.agent_messages("/a", "worktree")).unwrap();
    assert_eq!(all[0].id, first);
    assert_eq!(
        all[0],
        AgentMessage {
            id: first,
            role: AgentRole::Assistant,
            kind: AgentKind::Overview,
            text: "overview".to_string(),
            head: "h1".to_string(),
            created_at: NOW,
        }
    );
    assert!(all.windows(2).all(|pair| pair[0].id < pair[1].id));
    assert_eq!(texts(&db, "/a", "branches:main..topic"), ["elsewhere"]);
}

#[test]
fn an_unknown_stored_role_or_kind_does_not_fail_the_load() {
    let db = Db::open_in_memory().unwrap();
    drop(db.add_agent_message(
        "/a",
        "worktree",
        message(AgentRole::User, AgentKind::Chat, "fine", "h", NOW),
    ));
    block_on(db.call(|conn| {
        conn.execute(
            "INSERT INTO agent_messages (repo, review, role, kind, text, head, created_at)
             VALUES ('/a', 'worktree', 'tool', 'summary', 'odd', 'h', 1)",
            [],
        )?;
        Ok(())
    }))
    .unwrap();
    let list = block_on(db.agent_messages("/a", "worktree")).unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[1].text, "odd");
    assert_eq!(list[1].role, AgentRole::Notice);
    assert_eq!(list[1].kind, AgentKind::Chat);
}

#[test]
fn setting_a_session_replaces_the_previous_id_for_that_key_only() {
    let db = Db::open_in_memory().unwrap();
    assert_eq!(
        block_on(db.agent_session("/a", "worktree", "claude")).unwrap(),
        None
    );
    drop(db.set_agent_session("/a", "worktree", "claude", "one", NOW));
    drop(db.set_agent_session("/a", "worktree", "codex", "other", NOW));
    drop(db.set_agent_session("/a", "worktree", "claude", "two", NOW + 1));
    let session = |agent: &str| block_on(db.agent_session("/a", "worktree", agent)).unwrap();
    assert_eq!(session("claude"), Some("two".to_string()));
    assert_eq!(session("codex"), Some("other".to_string()));
    let count: i64 = block_on(db.call(|conn| {
        Ok(conn.query_row("SELECT COUNT(*) FROM agent_sessions", [], |row| row.get(0))?)
    }))
    .unwrap();
    assert_eq!(count, 2);
}

#[test]
fn clears_are_scoped_to_their_key() {
    let db = Db::open_in_memory().unwrap();
    for (repo, review, agent) in [
        ("/a", "worktree", "claude"),
        ("/a", "worktree", "codex"),
        ("/a", "mr:github:o/r:1", "claude"),
        ("/b", "worktree", "claude"),
    ] {
        drop(db.set_agent_session(repo, review, agent, "s", NOW));
        drop(db.add_agent_message(
            repo,
            review,
            message(AgentRole::User, AgentKind::Chat, agent, "h", NOW),
        ));
    }
    drop(db.clear_agent_session("/a", "worktree", "claude"));
    let session = |repo: &str, review: &str, agent: &str| {
        block_on(db.agent_session(repo, review, agent)).unwrap()
    };
    assert_eq!(session("/a", "worktree", "claude"), None);
    assert!(session("/a", "worktree", "codex").is_some());
    assert!(session("/a", "mr:github:o/r:1", "claude").is_some());
    assert!(session("/b", "worktree", "claude").is_some());
    assert_eq!(texts(&db, "/a", "worktree"), ["claude", "codex"]);

    drop(db.clear_agent_messages("/a", "worktree"));
    assert!(texts(&db, "/a", "worktree").is_empty());
    assert_eq!(texts(&db, "/a", "mr:github:o/r:1"), ["claude"]);
    assert_eq!(texts(&db, "/b", "worktree"), ["claude"]);
    assert!(session("/a", "worktree", "codex").is_some());
}

#[test]
fn pruning_drops_only_old_sessions_and_messages() {
    let db = Db::open_in_memory().unwrap();
    let old = NOW - AGENT_RETENTION_SECS - 1;
    let edge = NOW - AGENT_RETENTION_SECS;
    drop(db.set_agent_session("/a", "worktree", "old", "s", old));
    drop(db.set_agent_session("/a", "worktree", "edge", "s", edge));
    drop(db.set_agent_session("/a", "worktree", "new", "s", NOW));
    for (text, at) in [("old", old), ("edge", edge), ("new", NOW)] {
        drop(db.add_agent_message(
            "/a",
            "worktree",
            message(AgentRole::User, AgentKind::Chat, text, "h", at),
        ));
    }
    assert_eq!(block_on(db.prune_agent(NOW)).unwrap(), 2);
    let session = |agent: &str| block_on(db.agent_session("/a", "worktree", agent)).unwrap();
    assert_eq!(session("old"), None);
    assert!(session("edge").is_some());
    assert!(session("new").is_some());
    assert_eq!(texts(&db, "/a", "worktree"), ["edge", "new"]);
}
