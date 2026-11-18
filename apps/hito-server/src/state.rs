use std::sync::Arc;

use hito_storage::Store;

use crate::config::Config;
use crate::hub::Hub;
use crate::limiter::Limiter;

/// Shared by every request. Cheap to clone.
#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub config: Arc<Config>,
    pub hub: Arc<Hub>,
    pub limiter: Arc<Limiter>,
    /// An Argon2id hash of a random password, checked when a login names an unknown account so
    /// that both cases take the same time.
    pub dummy_hash: Arc<str>,
}
