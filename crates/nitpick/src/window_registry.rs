use std::collections::BTreeMap;
use std::path::PathBuf;

use gpui::{App, Global, Window, WindowId};

use crate::recent::RecentRepos;

#[derive(Default)]
pub struct WindowRepos {
    quitting: bool,
    order: u64,
    windows: BTreeMap<WindowId, (u64, Option<PathBuf>)>,
}

impl Global for WindowRepos {}

pub fn register(window: &Window, cx: &mut App) {
    let id = window.window_handle().window_id();
    let registry = cx.default_global::<WindowRepos>();
    registry.order += 1;
    let order = registry.order;
    registry.windows.entry(id).or_insert((order, None));
}

pub fn set_repo(window: &Window, repo: Option<PathBuf>, cx: &mut App) {
    let id = window.window_handle().window_id();
    let registry = cx.default_global::<WindowRepos>();
    registry.order += 1;
    let order = registry.order;
    let entry = registry.windows.entry(id).or_insert((order, None));
    entry.1 = repo;
    persist(cx);
}

pub fn remove(id: WindowId, cx: &mut App) {
    let registry = cx.default_global::<WindowRepos>();
    let removed = registry.windows.remove(&id).is_some();
    let quitting = registry.quitting;
    if removed && !quitting && has_windows(cx) {
        persist(cx);
    }
}

pub fn repo_of(id: WindowId, cx: &App) -> Option<Option<PathBuf>> {
    cx.try_global::<WindowRepos>()?
        .windows
        .get(&id)
        .map(|(_, repo)| repo.clone())
}

pub fn has_windows(cx: &App) -> bool {
    cx.try_global::<WindowRepos>()
        .is_some_and(|registry| !registry.windows.is_empty())
}

pub fn quitting(cx: &mut App) {
    cx.default_global::<WindowRepos>().quitting = true;
}

fn persist(cx: &mut App) {
    let mut entries: Vec<(u64, PathBuf)> = cx
        .default_global::<WindowRepos>()
        .windows
        .values()
        .filter_map(|(order, repo)| repo.clone().map(|r| (*order, r)))
        .collect();
    entries.sort_by_key(|(order, _)| *order);
    let mut repos: Vec<PathBuf> = Vec::with_capacity(entries.len());
    for (_, repo) in entries {
        if !repos.contains(&repo) {
            repos.push(repo);
        }
    }
    cx.global_mut::<RecentRepos>().set_open_windows(repos);
}
