use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui::{App, AppContext as _, Context, Entity, Global, Task};
use review_core::ViewedStore;

const SAVE_DELAY: Duration = Duration::from_millis(250);

pub struct SharedViewed {
    store: ViewedStore,
    dirty: bool,
    _save: Task<()>,
}

#[derive(Default)]
struct ViewedStores(HashMap<PathBuf, Entity<SharedViewed>>);

impl Global for ViewedStores {}

fn viewed_dir() -> PathBuf {
    crate::settings_store::app_dir(dirs::data_dir()).join("reviews")
}

pub fn shared_for(root: &Path, cx: &mut App) -> Entity<SharedViewed> {
    if let Some(entity) = cx.default_global::<ViewedStores>().0.get(root) {
        return entity.clone();
    }
    let store = ViewedStore::load(&viewed_dir(), root, crate::recent::now());
    let entity = cx.new(|_| SharedViewed {
        store,
        dirty: false,
        _save: Task::ready(()),
    });
    cx.default_global::<ViewedStores>()
        .0
        .insert(root.to_path_buf(), entity.clone());
    entity
}

pub fn flush_all(cx: &mut App) {
    let stores: Vec<Entity<SharedViewed>> = cx
        .default_global::<ViewedStores>()
        .0
        .values()
        .cloned()
        .collect();
    for entity in stores {
        entity.update(cx, |shared, _| shared.flush());
    }
}

impl SharedViewed {
    pub fn store(&self) -> &ViewedStore {
        &self.store
    }

    pub fn set(&mut self, marks: &[(&str, &str)], viewed: bool, cx: &mut Context<Self>) {
        let now = crate::recent::now();
        for (path, fingerprint) in marks {
            self.store.set_viewed(path, fingerprint, viewed, now);
        }
        self.dirty = true;
        self._save = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_DELAY).await;
            let Ok(Some((path, bytes))) = this.update(cx, |this, _| {
                this.dirty = false;
                this.store.snapshot()
            }) else {
                return;
            };
            cx.background_spawn(async move {
                if let Err(err) = review_core::write_atomic(&path, &bytes) {
                    log::warn!("saving viewed marks: {err}");
                }
            })
            .await;
        });
        cx.notify();
    }

    fn flush(&mut self) {
        if self.dirty {
            self.dirty = false;
            self._save = Task::ready(());
            if let Err(err) = self.store.save() {
                log::warn!("saving viewed marks: {err}");
            }
        }
    }
}
