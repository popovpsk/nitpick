use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use crate::process::{Grace, reap};
use crate::session::Control;

#[derive(Default)]
pub struct Registry {
    sessions: Mutex<HashMap<u64, Arc<Control>>>,
}

static GLOBAL: OnceLock<Arc<Registry>> = OnceLock::new();

impl Registry {
    pub fn global() -> Arc<Registry> {
        Arc::clone(GLOBAL.get_or_init(|| Arc::new(Registry::default())))
    }

    pub fn insert(&self, control: Arc<Control>) {
        self.lock().insert(control.id(), control);
    }

    pub fn remove(&self, id: u64) {
        self.lock().remove(&id);
    }

    pub fn live(&self) -> usize {
        self.lock().len()
    }

    pub fn shutdown_all(&self) -> impl Future<Output = ()> + Send + 'static + use<> {
        let controls: Vec<Arc<Control>> = self.lock().values().cloned().collect();
        let waits: Vec<_> = controls.iter().map(|control| control.shutdown()).collect();
        async move {
            for wait in waits {
                wait.await;
            }
        }
    }

    pub fn groups(&self) -> Vec<u32> {
        self.lock()
            .values()
            .filter_map(|control| control.leader())
            .collect()
    }

    pub fn quit(&self, grace: Grace) {
        let controls: Vec<Arc<Control>> = self.lock().values().cloned().collect();
        if controls.is_empty() {
            return;
        }
        let groups: Vec<u32> = controls
            .iter()
            .filter_map(|control| control.leader())
            .collect();
        if let Err(error) = reap(&groups, grace) {
            log::warn!("starting the process reaper: {error}");
        }
        for control in &controls {
            control.request_shutdown();
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<u64, Arc<Control>>> {
        self.sessions.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

pub fn shutdown_all() -> impl Future<Output = ()> + Send + 'static + use<> {
    Registry::global().shutdown_all()
}

pub fn quit() {
    Registry::global().quit(Grace::default());
}
