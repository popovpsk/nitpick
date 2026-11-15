use std::collections::HashMap;
use std::path::{Path, PathBuf};

use gpui::{App, AppContext as _, Context, Entity, Global};
use review_core::ViewedStore;

type Write = (Vec<(String, String)>, bool, i64);

pub struct SharedViewed {
    repo: String,
    store: ViewedStore,
    loading: Option<Vec<Write>>,
}

#[derive(Default)]
struct ViewedStores(HashMap<PathBuf, Entity<SharedViewed>>);

impl Global for ViewedStores {}

pub fn shared_for(root: &Path, cx: &mut App) -> Entity<SharedViewed> {
    if let Some(entity) = cx.default_global::<ViewedStores>().0.get(root) {
        return entity.clone();
    }
    let repo = root.to_string_lossy().into_owned();
    let load = crate::db::db(cx).map(|db| db.load_viewed(&repo, crate::recent::now()));
    let loading = load.is_some().then(Vec::new);
    let entity = cx.new(|cx| {
        if let Some(load) = load {
            cx.spawn(async move |this, cx| {
                let marks = load.await.unwrap_or_else(|err| {
                    log::warn!("loading viewed marks: {err}");
                    Vec::new()
                });
                let _ = this.update(cx, |this: &mut SharedViewed, cx| this.install(marks, cx));
            })
            .detach();
        }
        SharedViewed {
            repo,
            store: ViewedStore::new(),
            loading,
        }
    });
    cx.default_global::<ViewedStores>()
        .0
        .insert(root.to_path_buf(), entity.clone());
    entity
}

impl SharedViewed {
    pub fn store(&self) -> &ViewedStore {
        &self.store
    }

    fn install(&mut self, marks: Vec<app_db::ViewedMark>, cx: &mut Context<Self>) {
        let mut store = ViewedStore::from_marks(marks.iter().map(|mark| {
            (
                mark.path.as_str(),
                mark.fingerprint.as_str(),
                mark.viewed_at,
            )
        }));
        for (marks, viewed, at) in self.loading.take().unwrap_or_default() {
            for (path, fingerprint) in &marks {
                store.set_viewed(path, fingerprint, viewed, at);
            }
        }
        self.store = store;
        cx.notify();
    }

    pub fn set(&mut self, marks: &[(&str, &str)], viewed: bool, cx: &mut Context<Self>) {
        let now = crate::recent::now();
        for (path, fingerprint) in marks {
            self.store.set_viewed(path, fingerprint, viewed, now);
        }
        let owned: Vec<(String, String)> = marks
            .iter()
            .map(|(path, fingerprint)| (path.to_string(), fingerprint.to_string()))
            .collect();
        if let Some(db) = crate::db::db(cx) {
            let write = db.set_viewed(&self.repo, owned.clone(), viewed, now);
            cx.background_spawn(async move {
                if let Err(err) = write.await {
                    log::warn!("saving viewed marks: {err}");
                }
            })
            .detach();
        }
        if let Some(pending) = self.loading.as_mut() {
            pending.push((owned, viewed, now));
        }
        cx.notify();
    }
}
