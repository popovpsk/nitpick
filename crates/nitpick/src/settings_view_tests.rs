use std::path::{Path, PathBuf};

use super::hidden_roots;

fn paths(list: &[&str]) -> Vec<PathBuf> {
    list.iter().map(PathBuf::from).collect()
}

#[test]
fn the_opener_comes_first_then_every_other_repository_once_by_name() {
    let roots = hidden_roots(
        Some(Path::new("/w/zeta")),
        &paths(&["/w/beta", "/w/zeta"]),
        paths(&["/w/Alpha", "/w/beta"]).into_iter(),
    );
    assert_eq!(roots, paths(&["/w/zeta", "/w/Alpha", "/w/beta"]));
    assert!(hidden_roots(None, &[], std::iter::empty()).is_empty());
}
