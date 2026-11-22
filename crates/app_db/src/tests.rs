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

fn session(id: &str, brief_delivered: bool, instructions: &str) -> AgentSession {
    AgentSession {
        id: id.to_string(),
        brief_delivered,
        instructions: instructions.to_string(),
    }
}

fn fresh(id: &str) -> AgentSession {
    session(id, false, "")
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
    block_on(db.set_agent_session("/a", "worktree", "claude", fresh("s1"), NOW)).unwrap();
    assert_eq!(
        block_on(db.agent_session("/a", "worktree", "claude")).unwrap(),
        Some(fresh("s1"))
    );
    drop(db);

    let conn = rusqlite::Connection::open(&path).unwrap();
    let version: i64 = conn
        .query_row("SELECT user_version FROM pragma_user_version", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(version, MIGRATIONS.len() as i64);
    assert_eq!(version, 3);
}

#[test]
fn a_version_two_database_loses_its_messages_and_keeps_its_sessions() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("nitpick.db");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(MIGRATIONS[0]).unwrap();
    conn.execute_batch(MIGRATIONS[1]).unwrap();
    conn.pragma_update(None, "user_version", 2).unwrap();
    conn.execute(
        "INSERT INTO agent_sessions (repo, review, agent, session_id, updated_at)
         VALUES ('/a', 'worktree', 'claude', 's1', ?1)",
        [NOW],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO agent_messages (repo, review, role, kind, text, head, created_at)
         VALUES ('/a', 'worktree', 'user', 'chat', 'hi', 'h', ?1)",
        [NOW],
    )
    .unwrap();
    drop(conn);

    let db = Db::open(&path).unwrap();
    assert_eq!(
        block_on(db.agent_session("/a", "worktree", "claude")).unwrap(),
        Some(fresh("s1"))
    );
    let tables: i64 = block_on(db.call(|conn| {
        Ok(conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'agent_messages'",
            [],
            |row| row.get(0),
        )?)
    }))
    .unwrap();
    assert_eq!(tables, 0);
}

#[test]
fn setting_a_session_replaces_the_previous_id_for_that_key_only() {
    let db = Db::open_in_memory().unwrap();
    assert_eq!(
        block_on(db.agent_session("/a", "worktree", "claude")).unwrap(),
        None
    );
    drop(db.set_agent_session("/a", "worktree", "claude", fresh("one"), NOW));
    drop(db.set_agent_session("/a", "worktree", "codex", fresh("other"), NOW));
    drop(db.set_agent_session("/a", "worktree", "claude", fresh("two"), NOW + 1));
    let stored = |agent: &str| block_on(db.agent_session("/a", "worktree", agent)).unwrap();
    assert_eq!(stored("claude"), Some(fresh("two")));
    assert_eq!(stored("codex"), Some(fresh("other")));
    let count: i64 = block_on(db.call(|conn| {
        Ok(conn.query_row("SELECT COUNT(*) FROM agent_sessions", [], |row| row.get(0))?)
    }))
    .unwrap();
    assert_eq!(count, 2);
}

#[test]
fn an_update_changes_only_the_session_it_names() {
    let db = Db::open_in_memory().unwrap();
    drop(db.set_agent_session("/a", "worktree", "claude", fresh("one"), NOW));
    drop(db.update_agent_session(
        "/a",
        "worktree",
        "claude",
        session("one", true, "fp"),
        NOW + 1,
    ));
    let stored = || block_on(db.agent_session("/a", "worktree", "claude")).unwrap();
    assert_eq!(stored(), Some(session("one", true, "fp")));
    drop(db.update_agent_session(
        "/a",
        "worktree",
        "claude",
        session("another", true, "x"),
        NOW + 2,
    ));
    assert_eq!(stored(), Some(session("one", true, "fp")));
    drop(db.set_agent_session("/a", "worktree", "claude", fresh("two"), NOW + 3));
    assert_eq!(stored(), Some(fresh("two")));
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
        drop(db.set_agent_session(repo, review, agent, fresh("s"), NOW));
    }
    drop(db.clear_agent_session("/a", "worktree", "claude", None));
    let stored = |repo: &str, review: &str, agent: &str| {
        block_on(db.agent_session(repo, review, agent)).unwrap()
    };
    assert_eq!(stored("/a", "worktree", "claude"), None);
    assert!(stored("/a", "worktree", "codex").is_some());
    assert!(stored("/a", "mr:github:o/r:1", "claude").is_some());
    assert!(stored("/b", "worktree", "claude").is_some());
}

#[test]
fn a_clear_naming_an_id_leaves_another_windows_session() {
    let db = Db::open_in_memory().unwrap();
    drop(db.set_agent_session("/a", "worktree", "claude", fresh("theirs"), NOW));
    drop(db.clear_agent_session("/a", "worktree", "claude", Some("mine")));
    let stored = || block_on(db.agent_session("/a", "worktree", "claude")).unwrap();
    assert_eq!(stored(), Some(fresh("theirs")));
    drop(db.clear_agent_session("/a", "worktree", "claude", Some("theirs")));
    assert_eq!(stored(), None);
}

#[test]
fn pruning_drops_only_old_sessions() {
    let db = Db::open_in_memory().unwrap();
    let old = NOW - AGENT_RETENTION_SECS - 1;
    let edge = NOW - AGENT_RETENTION_SECS;
    drop(db.set_agent_session("/a", "worktree", "old", fresh("s"), old));
    drop(db.set_agent_session("/a", "worktree", "edge", fresh("s"), edge));
    drop(db.set_agent_session("/a", "worktree", "new", fresh("s"), NOW));
    assert_eq!(block_on(db.prune_agent(NOW)).unwrap(), 1);
    let stored = |agent: &str| block_on(db.agent_session("/a", "worktree", agent)).unwrap();
    assert_eq!(stored("old"), None);
    assert!(stored("edge").is_some());
    assert!(stored("new").is_some());
}
