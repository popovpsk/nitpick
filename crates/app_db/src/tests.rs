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
