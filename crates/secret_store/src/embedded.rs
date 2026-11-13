use sha2::{Digest, Sha256};

use crate::crypto::Key;

const SEED: &str = "sloptaster-secret-store-public-seed";

pub(crate) fn key() -> Key {
    let digest = Sha256::new()
        .chain_update(b"sloptaster-secrets-v1")
        .chain_update(SEED.as_bytes())
        .finalize();
    Key::new(digest.into())
}
