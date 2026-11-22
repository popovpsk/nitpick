use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);
static HELD: Mutex<Vec<(String, u64)>> = Mutex::new(Vec::new());

pub(super) fn next_owner() -> u64 {
    NEXT_OWNER.fetch_add(1, Ordering::Relaxed)
}

pub(super) fn is_held(id: &str) -> bool {
    HELD.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .any(|(held, _)| held == id)
}

#[derive(Debug, Clone)]
pub(super) struct Claim(Arc<Held>);

#[derive(Debug)]
struct Held {
    id: String,
    owner: u64,
}

impl Claim {
    pub(super) fn acquire(id: &str, owner: u64) -> Option<Claim> {
        let mut held = HELD.lock().unwrap_or_else(PoisonError::into_inner);
        if held.iter().any(|(taken, _)| taken == id) {
            return None;
        }
        held.push((id.to_owned(), owner));
        Some(Claim(Arc::new(Held {
            id: id.to_owned(),
            owner,
        })))
    }

    pub(super) fn id(&self) -> &str {
        &self.0.id
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        let mut held = HELD.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(at) = held
            .iter()
            .position(|(id, owner)| *id == self.id && *owner == self.owner)
        {
            held.swap_remove(at);
        }
    }
}
