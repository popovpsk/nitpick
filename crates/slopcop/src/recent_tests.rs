use std::path::{Path, PathBuf};

use tempfile::TempDir;

use super::{RecentRepos, StoredReview};

fn paths(recent: &RecentRepos) -> Vec<String> {
    recent
        .repos()
        .iter()
        .map(|r| r.path.to_string_lossy().into_owned())
        .collect()
}

fn store(dir: &TempDir) -> RecentRepos {
    RecentRepos::load(dir.path().join("recent.json"))
}

#[test]
fn opening_orders_most_recent_first_even_within_one_second() {
    let dir = TempDir::new().unwrap();
    let mut recent = store(&dir);
    for repo in ["/r/a", "/r/b", "/r/c", "/r/a"] {
        recent.touch(Path::new(repo), 100);
    }
    assert_eq!(paths(&recent), vec!["/r/a", "/r/c", "/r/b"]);
}

#[test]
fn added_repositories_follow_the_opened_ones_by_name_and_are_never_dropped() {
    let dir = TempDir::new().unwrap();
    let mut recent = store(&dir);
    for ix in 0..25 {
        recent.touch(Path::new(&format!("/opened/{ix:02}")), 1_000 + ix);
    }
    let mut bulk: Vec<PathBuf> = (0..30)
        .map(|ix| PathBuf::from(format!("/bulk/{ix:02}")))
        .collect();
    bulk.push(PathBuf::from("/bulk/05"));
    bulk.push(PathBuf::from("/opened/03"));
    assert_eq!(recent.add(&bulk), 30);
    assert_eq!(recent.add(&bulk), 0);
    let all = paths(&recent);
    assert_eq!(all.len(), 55);
    assert_eq!(all[0], "/opened/24");
    assert_eq!(all[24], "/opened/00");
    assert_eq!(all[25], "/bulk/00");
    assert_eq!(all[54], "/bulk/29");

    recent.touch(Path::new("/bulk/17"), 5);
    assert_eq!(paths(&recent)[0], "/bulk/17");
    assert_eq!(paths(&recent)[1], "/opened/24");
}

#[test]
fn never_opened_repositories_sort_by_folder_name_ignoring_case() {
    let dir = TempDir::new().unwrap();
    let mut recent = store(&dir);
    recent.add(&[
        PathBuf::from("/x/zeta"),
        PathBuf::from("/y/Alpha"),
        PathBuf::from("/x/beta"),
    ]);
    assert_eq!(paths(&recent), vec!["/y/Alpha", "/x/beta", "/x/zeta"]);
}

#[test]
fn the_existing_file_format_still_loads_and_round_trips() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("recent.json");
    std::fs::write(
        &file,
        r#"{"repos":[{"path":"/r/old","opened_at":5},{"path":"/r/new","opened_at":9,"last_review":{"kind":"branches","target":"main","source":"feature"}}],"open_windows":["/r/new"]}"#,
    )
    .unwrap();
    let mut recent = RecentRepos::load(file.clone());
    assert_eq!(paths(&recent), vec!["/r/new", "/r/old"]);
    assert_eq!(
        recent.last_review(Path::new("/r/new")),
        Some(&StoredReview::Branches {
            target: "main".into(),
            source: "feature".into(),
        })
    );
    assert_eq!(recent.open_windows(), &[PathBuf::from("/r/new")]);

    recent.add(&[PathBuf::from("/r/added")]);
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(json["repos"][0]["path"], "/r/new");
    assert_eq!(json["repos"][0]["last_review"]["kind"], "branches");
    assert_eq!(json["repos"][2]["path"], "/r/added");
    assert_eq!(json["repos"][2]["opened_at"], 0);
    assert_eq!(json["open_windows"][0], "/r/new");
    assert_eq!(paths(&RecentRepos::load(file)), paths(&recent));
}
