// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::{collections::VecDeque, fs::File, sync::Arc};

use tempfile::TempDir;
use tokio::sync::RwLock;

use crate::{config::CachingConfig, util::debug_panic_or_log_error};

use super::prelude::*;

pub struct CachedStore<S> {
    store: S,
    cache: Arc<RwLock<StoreCache>>,
    max_cache_size_bytes: Option<u64>,
}

#[derive(Default)]
pub struct StoreCache {
    entries: VecDeque<CacheEntry>,
    total_size: u64,
}

struct CacheEntry {
    key: String,
    tmp_dir: Arc<TempDir>,
    size: u64,
}

impl<S> CachedStore<S>
where
    S: std::ops::Deref + Sync,
    S::Target: ObjectStore,
{
    pub fn new(store: S, cache: Arc<RwLock<StoreCache>>, caching_config: &CachingConfig) -> Self {
        Self {
            store,
            cache,
            max_cache_size_bytes: caching_config
                .max_backup_cache_size
                .as_ref()
                .map(crate::util::BytesAmount::as_bytes),
        }
    }

    pub fn inner(&self) -> &S {
        &self.store
    }

    pub async fn cache(&self, key: String, tmp_dir: Arc<TempDir>) {
        let mut cache = self.cache.write().await;

        let Ok(metadata) = tmp_dir.path().metadata() else {
            debug_panic_or_log_error!(
                "Cannot read metadata at `{path}`",
                path = tmp_dir.path().display()
            );
            return;
        };
        let size = metadata.len();

        if let Some(max_size) = self.max_cache_size_bytes {
            // Ensure object fits in cache.
            if size > max_size {
                tracing::debug!(
                    "Object `{key}` is larger than allowed cache size ({size} > {max_size}), not caching."
                );
                return;
            }

            // Purge cache entries to make space if needed.
            while cache.total_size + size > max_size {
                let Some(entry) = cache.entries.pop_front() else {
                    debug_panic_or_log_error!(
                        "Cache size was desynchronized. This is a logic error."
                    );

                    // Resynchronize the cache size (no more entries).
                    cache.total_size = 0;

                    // Resume to caching (we know that `size <= max_size`
                    // and now `total_size == 0`).
                    break;
                };

                tracing::debug!(
                    "Object `{cached_key}` purged from cache, to make space for `{key}`.",
                    cached_key = entry.key
                );
                cache.total_size = cache.total_size.saturating_sub(entry.size);

                // NOTE: The entry’s temporary directory will be deleted when
                //   `entry` is dropped (i.e. now).
            }
        }

        tracing::debug!("Caching `{key}`.");
        cache.entries.push_back(CacheEntry { key, tmp_dir, size });
        // SAFETY: We’ll never reach 18 446 744 TB.
        cache.total_size += size;
    }

    /// Remove a cached entry.
    pub async fn remove(&self, key: &str) {
        let mut cache = self.cache.write().await;

        let mut indices: Vec<(usize, u64)> = Vec::with_capacity(1);
        for (index, entry) in cache.entries.iter().enumerate() {
            if entry.key == key {
                indices.push((index, entry.size));
            }
        }

        for (index, size) in indices {
            // NOTE: The entry’s temporary directory will be deleted when
            //   the entry value is dropped (i.e. right after `.remove`).
            cache.entries.remove(index);
            cache.total_size = cache.total_size.saturating_sub(size);
        }

        tracing::debug!("Object `{key}` removed from cache, as requested.");
    }
}

#[async_trait::async_trait]
impl<S> ObjectStore for CachedStore<S>
where
    S: std::ops::Deref + Sync,
    S::Target: ObjectStore,
{
    #[inline]
    async fn writer(&self, key: &str) -> Result<Box<DynObjectWriter>, anyhow::Error> {
        self.store.writer(key).await
    }

    #[inline]
    async fn reader(&self, key: &str) -> Result<Box<DynObjectReader>, ReadObjectError> {
        let cache = self.cache.read().await;

        for entry in cache.entries.iter() {
            if entry.key == key {
                let path = entry.tmp_dir.path().join(&entry.key);
                match File::open(&path) {
                    Ok(file) => {
                        tracing::debug!(
                            "Object `{key}` was cached. Reading from `{path}`.",
                            path = path.display()
                        );
                        return Ok(Box::new(file));
                    }
                    Err(err) => {
                        debug_panic_or_log_error!(
                            "Failed opening `{path}`: {err:?}",
                            path = path.display()
                        );

                        // Skip reading from cache (we’ve found the entry
                        // and it’s broken).
                        break;
                    }
                }
            }
        }
        drop(cache);

        self.store.reader(key).await
    }

    #[inline]
    async fn exists(&self, key: &str) -> Result<bool, anyhow::Error> {
        self.store.exists(key).await
    }

    #[inline]
    async fn find(&self, prefix: &str) -> Result<Vec<ObjectMetadata>, anyhow::Error> {
        self.store.find(prefix).await
    }

    #[inline]
    async fn list_all_after(&self, prefix: &str) -> Result<Vec<ObjectMetadata>, anyhow::Error> {
        self.store.list_all_after(prefix).await
    }

    #[inline]
    async fn metadata(&self, key: &str) -> Result<ObjectMetadata, ReadObjectError> {
        self.store.metadata(key).await
    }

    #[inline]
    async fn download_url(
        &self,
        key: &str,
        ttl: &std::time::Duration,
    ) -> Result<String, anyhow::Error> {
        self.store.download_url(key, ttl).await
    }

    #[inline]
    async fn delete(&self, key: &str) -> Result<DeletedState, anyhow::Error> {
        let deleted_state = self.store.delete(key).await?;

        // Purge cache entry.
        self.remove(key).await;

        Ok(deleted_state)
    }

    #[inline]
    async fn delete_all(&self, prefix: &str) -> Result<BulkDeleteOutput, anyhow::Error> {
        self.store.delete_all(prefix).await
    }
}
