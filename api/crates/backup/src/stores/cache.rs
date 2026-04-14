// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::collections::VecDeque;
use std::fs::File;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context as _;
use tokio::sync::RwLock;

use crate::stats::{MeteredStream, StreamStats, WriterStats};
use crate::util::PathGuard;
use crate::{config::CachingConfig, util::debug_panic_or_log_error};

use super::prelude::*;

pub struct CachedStore<S> {
    store: S,
    cache: Arc<RwLock<StoreCache>>,
    max_cache_size_bytes: Option<u64>,
    cache_dir: PathBuf,
}

#[derive(Default)]
pub struct StoreCache {
    entries: VecDeque<CacheEntry>,
    total_size: u64,
}

struct CacheEntry {
    key: String,
    path: Arc<PathGuard>,
    size: u64,
}

impl<S> CachedStore<S>
where
    S: std::ops::Deref + Sync,
    S::Target: ObjectStore,
{
    pub fn new(store: S, cache: Arc<RwLock<StoreCache>>, caching_config: &CachingConfig) -> Self {
        let cache_dir = &caching_config.cache_dir;
        debug_assert!(cache_dir.is_dir());

        Self {
            store,
            cache,
            max_cache_size_bytes: caching_config
                .max_backup_cache_size
                .as_ref()
                .map(crate::util::BytesAmount::as_bytes),
            cache_dir: cache_dir.to_owned(),
        }
    }

    pub fn inner(&self) -> &S {
        &self.store
    }

    async fn cache(&self, key: String, path: Arc<PathGuard>, size: u64) {
        let mut cache = self.cache.write().await;

        for entry in cache.entries.iter() {
            if entry.key == key {
                debug_panic_or_log_error!(
                    "Object `{key}` already exists, not caching `{path}`.",
                    path = path.display()
                );
                return;
            }
        }

        #[cfg(debug_assertions)]
        assert_eq!(size, path.metadata().unwrap().len());

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

        tracing::debug!("Caching `{key}` in `{path}`…", path = path.display());

        cache.entries.push_back(CacheEntry { key, path, size });

        // SAFETY: We’ll never reach 18 446 744 TB.
        cache.total_size += size;
    }

    pub async fn cached_reader(
        &self,
        key: &str,
    ) -> Result<CachedReader<Box<DynObjectReader>>, ReadObjectError> {
        use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};

        let cache = self.cache.read().await;

        for entry in cache.entries.iter() {
            if entry.key == key {
                match File::open(entry.path.as_ref()) {
                    Ok(file) => {
                        tracing::debug!(
                            "Object `{key}` was cached. Reading from `{path}`.",
                            path = entry.path.display()
                        );
                        return Ok(CachedReader::Cached {
                            reader: file,
                            path: Arc::clone(&entry.path),
                        });
                    }
                    Err(err) => {
                        debug_panic_or_log_error!(
                            "Failed opening `{path}`: {err:?}",
                            path = entry.path.display()
                        );

                        // Skip reading from cache (we’ve found the entry
                        // and it’s broken).
                        break;
                    }
                }
            }
        }
        drop(cache);

        // Open local file paths.
        // If permissions are not sufficient, avoids unnecessary network
        // calls (potentially billed).
        let object_path = self.cache_dir.join(key);

        tracing::debug!(
            "Will cache object `{key}` in `{path}`.",
            path = object_path.display()
        );
        let cache_file = std::fs::File::options()
            // Allow creating the file and writing to it.
            .create_new(true)
            .write(true)
            // Allow reading the file (necessary when verifying).
            .read(true)
            // Only allow read and write for the current user.
            // This is very important, as not doing so would virtually leak
            // data if the backup is unencrypted (default mode is `644`).
            .mode(0o600)
            .open(&object_path)
            .context("Failed opening a file path to cache the object at")
            .map_err(ReadObjectError::Other)?;
        if cfg!(debug_assertions) {
            let metadata = std::fs::metadata(&object_path).unwrap();
            debug_assert_eq!(metadata.permissions().mode(), 0o100600);
        }

        let reader = self.store.reader(key).await?;

        Ok(CachedReader::Caching {
            reader,
            writer: MeteredStream::new(cache_file, SizeRef(0)),
            key: key.to_owned(),
            path: Arc::new(PathGuard::new(object_path)),
        })
    }

    /// Persists the cache entry.
    pub async fn persist_cache<R>(&self, reader: CachedReader<R>) -> Arc<PathGuard> {
        match reader {
            CachedReader::Caching {
                writer, key, path, ..
            } => {
                let size = writer.into_stats();

                self.cache(key, Arc::clone(&path), *size).await;

                path
            }
            CachedReader::Cached { path, .. } => path,
        }
    }

    /// Helper for [`CachedStore::persist_cache`] then
    /// [`std::io::Seek::rewind`].
    pub async fn persist_cache_and_rewind<R>(
        &self,
        mut reader: CachedReader<R>,
    ) -> Result<CachedReader<R>, std::io::Error> {
        use std::io::Seek as _;

        match reader {
            CachedReader::Caching { .. } => {
                let path = self.persist_cache(reader).await;

                tracing::trace!("Rewinding (re-opening)…");

                // Re-open the file, with read access this time.
                let file = File::open(path.as_ref())?;

                // TODO: MADV_SEQUENTIAL?

                Ok(CachedReader::Cached { reader: file, path })
            }
            CachedReader::Cached {
                reader: ref mut file,
                ..
            } => {
                tracing::trace!("Rewinding…");
                file.rewind()?;
                Ok(reader)
            }
        }
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
    S: std::ops::Deref + std::fmt::Debug + Send + Sync,
    S::Target: ObjectStore,
{
    #[inline]
    async fn writer(&self, key: &str) -> Result<Box<DynObjectWriter>, anyhow::Error> {
        self.store.writer(key).await
    }

    #[inline]
    async fn reader(&self, key: &str) -> Result<Box<DynObjectReader>, ReadObjectError> {
        match self.cached_reader(key).await {
            Ok(reader) => Ok(Box::new(reader)),
            Err(err) => Err(err),
        }
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

// MARK: Cached reader

pub enum CachedReader<R> {
    Caching {
        reader: R,
        writer: MeteredStream<File, SizeRef>,
        key: String,
        path: Arc<PathGuard>,
    },
    Cached {
        reader: File,
        // NOTE: Keep a reference to the temporary path in case
        //   `CachedStore::cache` failed to store the cache entry
        //   (which would try to delete the file while it’s being read,
        //   resulting in a silent failure, resulting in a leaking cache
        //   potentially getting larger than `max_backup_cache_size`).
        path: Arc<PathGuard>,
    },
}

impl<R> std::io::Read for CachedReader<R>
where
    R: std::io::Read,
{
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, std::io::Error> {
        match self {
            CachedReader::Caching { reader, writer, .. } => {
                let n = reader.read(buf)?;

                // NOTE: The OS will flush before closing the file,
                //   no need to care about it.
                writer.write_all(&buf[..n])?;

                Ok(n)
            }
            CachedReader::Cached { reader, .. } => reader.read(buf),
        }
    }
}

impl<R> std::fmt::Debug for CachedReader<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Caching { key, path, .. } => f
                .debug_struct("Caching")
                .field("key", key)
                .field("path", path)
                .finish_non_exhaustive(),
            Self::Cached { path, .. } => f
                .debug_struct("Cached")
                .field("path", path)
                .finish_non_exhaustive(),
        }
    }
}

#[repr(transparent)]
pub struct SizeRef(u64);

impl StreamStats for SizeRef {
    fn record_chunk(&mut self, len: usize) {
        self.0 = self.0.saturating_add(len as u64);
    }

    #[cfg(debug_assertions)]
    fn record_duration(&mut self, _duration: &std::time::Duration) {}
}

impl WriterStats for SizeRef {
    fn record_flush(&mut self) {}
}

impl std::ops::Deref for SizeRef {
    type Target = u64;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

// MARK: - Boilerplate

impl<S> std::fmt::Debug for CachedStore<S>
where
    S: std::fmt::Debug,
{
    #[inline]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self {
            store,
            cache: _,
            max_cache_size_bytes,
            cache_dir,
        } = self;

        f.debug_struct("CachedStore")
            .field("store", store)
            .field("max_cache_size_bytes", max_cache_size_bytes)
            .field("cache_dir", cache_dir)
            .finish_non_exhaustive()
    }
}
