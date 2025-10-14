// prose-pod-api
//
// Copyright: 2024–2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::{collections::HashMap, sync::Arc};

use tokio::sync::RwLock;

use crate::{
    models::{AuthToken, BareJid, Password},
    util::Cache,
};

/// A place to store service accounts secrets (e.g. auth tokens).
///
/// WARN: This must NOT be used to save user tokens!
///
/// NOTE: If performance becomes a problem because `RwLock`s apply on
///   whole maps, use [`dashmap`](https://crates.io/crates/dashmap).
///   But beware of sharding overhead if we do a lot of batch processing
///   that iterates over all keys/values.
///   If performance issues come from long batch processing locking maps
///   for too long, consider splitting batches into smaller ones. `tokio`’s
///   `RwLock` are fair therefore releasing the locks for some time will
///   let all tasks in the queue to run before the next batch runs.
#[derive(Debug, Clone)]
pub struct SecretsStore {
    pub passwords: Arc<RwLock<HashMap<BareJid, Password>>>,
    pub tokens_cache: Arc<RwLock<Cache<BareJid, AuthToken>>>,
}

impl SecretsStore {
    pub(crate) fn new(app_config: &crate::AppConfig) -> Self {
        // NOTE: To make sure there is no instant where tokens are invalid,
        //   we can refresh them before they expire. `token_ttl` is in the
        //   hours range and refresing tokens takes milliseconds so we can
        //   safely refresh at 97% of the TTL without the time it takes to
        //   refresh making us go over the TTL.
        // NOTE: `0.96875` is almost like `0.97` but IEEE-754-exact (1 - 1/32).
        let tokens_cache_ttl = app_config.auth.token_ttl.mul_f32(0.96875);

        Self {
            passwords: Default::default(),
            tokens_cache: Arc::new(RwLock::new(Cache::new(tokens_cache_ttl))),
        }
    }
}
