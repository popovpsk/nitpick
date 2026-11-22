use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

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

    fn lock(&self) -> MutexGuard<'_, HashMap<u64, Arc<Control>>> {
        self.sessions.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

pub fn shutdown_all() -> impl Future<Output = ()> + Send + 'static + use<> {
    Registry::global().shutdown_all()
}
