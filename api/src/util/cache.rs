// prose-pod-server
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;
use std::task::{Context, Poll, ready};

use tokio::time::Duration;
use tokio_util::time::{DelayQueue, delay_queue};

/// Inspired by [DelayQueue in tokio_util::time - Rust](https://docs.rs/tokio-util/latest/tokio_util/time/struct.DelayQueue.html#usage-1).
pub struct Cache<CacheKey, Value> {
    entries: HashMap<CacheKey, (Value, delay_queue::Key)>,
    expirations: DelayQueue<CacheKey>,
    pub(self) ttl: Duration,
}

impl<CacheKey, Value> Cache<CacheKey, Value>
where
    CacheKey: std::cmp::Eq + std::hash::Hash + Clone,
{
    pub fn new(ttl: Duration) -> Self {
        Self {
            entries: HashMap::new(),
            expirations: DelayQueue::new(),
            ttl,
        }
    }

    pub fn insert(&mut self, key: CacheKey, value: Value) -> Option<Value> {
        // NOTE: A TTL of 0 is used in tests.
        if self.ttl.is_zero() {
            return None;
        }

        let delay = self.expirations.insert(key.clone(), self.ttl.clone());

        self.entries.insert(key, (value, delay)).map(|(val, _)| val)
    }

    pub fn contains_key(&self, key: &CacheKey) -> bool {
        self.entries.contains_key(key)
    }

    pub fn get(&self, key: &CacheKey) -> Option<&Value> {
        self.entries.get(key).map(|&(ref v, _)| v)
    }

    pub fn remove(&mut self, key: &CacheKey) -> Option<Value> {
        match self.entries.remove(key) {
            Some((value, cache_key)) => {
                self.expirations.remove(&cache_key);
                Some(value)
            }
            None => None,
        }
    }

    pub fn poll_purge(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        while let Some(entry) = ready!(self.expirations.poll_expired(cx)) {
            self.entries.remove(entry.get_ref());
        }

        Poll::Ready(())
    }

    pub async fn purge_task(cache: Arc<tokio::sync::RwLock<Self>>) -> Result<(), Infallible> {
        use std::task::Waker;

        let ttl = cache.read().await.ttl.clone();

        // NOTE: A TTL of 0 is used in tests.
        if ttl.is_zero() {
            return Ok(());
        }

        let mut interval = tokio::time::interval(ttl);
        loop {
            interval.tick().await;

            let mut cache = cache.write().await;

            let mut cx = Context::from_waker(Waker::noop());

            let _ = cache.poll_purge(&mut cx);
        }
    }
}

impl<CacheKey, Value> std::fmt::Debug for Cache<CacheKey, Value>
where
    CacheKey: std::fmt::Debug,
    Value: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cache")
            .field("entries", &self.entries)
            .field("expirations", &self.expirations)
            .field("ttl", &self.ttl)
            .finish()
    }
}
