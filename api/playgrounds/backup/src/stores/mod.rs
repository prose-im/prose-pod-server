// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! Data stores.

mod cache;
#[cfg(feature = "destination_fs")]
pub mod file;
#[cfg(feature = "destination_s3")]
pub mod s3;

mod prelude {
    pub use super::{BulkDeleteOutput, DeletedState, ObjectMetadata, ObjectStore, ReadObjectError};

    pub type DynObjectWriter = dyn super::ObjectWriter;
    pub type DynObjectReader = dyn std::io::Read + Send + Sync;
}

pub use self::cache::CachedStore;
#[cfg(feature = "destination_fs")]
pub use self::file::FsStore;
use self::prelude::*;
#[cfg(feature = "destination_s3")]
pub use self::s3::S3Store;

#[async_trait::async_trait]
pub trait ObjectStore: Sync {
    async fn writer(&self, key: &str) -> Result<Box<DynObjectWriter>, anyhow::Error>;

    /// Returns `None` if key does not exist.
    async fn reader(&self, key: &str) -> Result<Box<DynObjectReader>, ReadObjectError>;

    /// Returns `None` if key does not exist or object too large.
    #[inline]
    async fn reader_if_not_too_large<'a>(
        &self,
        key: &'a str,
        max_size: u64,
    ) -> Result<Box<DynObjectReader>, ReadSizedObjectError<'a>> {
        let size = self.metadata(key).await?.size_bytes;

        if size <= max_size {
            (self.reader(key).await).map_err(ReadSizedObjectError::ReadFailed)
        } else {
            Err(ReadSizedObjectError::ObjectTooLarge {
                key,
                size,
                max_size,
            })
        }
    }

    async fn exists(&self, key: &str) -> Result<bool, anyhow::Error>;

    async fn find(&self, prefix: &str) -> Result<Vec<ObjectMetadata>, anyhow::Error>;

    async fn list_all_after(&self, prefix: &str) -> Result<Vec<ObjectMetadata>, anyhow::Error>;

    async fn list_all(&self) -> Result<Vec<ObjectMetadata>, anyhow::Error> {
        self.list_all_after("").await
    }

    async fn metadata(&self, key: &str) -> Result<ObjectMetadata, ReadObjectError>;

    async fn download_url(
        &self,
        key: &str,
        ttl: &std::time::Duration,
    ) -> Result<String, anyhow::Error>;

    #[must_use]
    async fn delete(&self, key: &str) -> Result<DeletedState, anyhow::Error>;

    #[must_use]
    async fn delete_all(&self, prefix: &str) -> Result<BulkDeleteOutput, anyhow::Error>;
}

pub struct ObjectMetadata {
    pub file_name: String,
    pub size_bytes: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum DeletedState {
    Deleted,
    MarkedForDeletion,
}

#[derive(Debug, Default)]
pub struct BulkDeleteOutput {
    pub deleted: Vec<String>,
    pub marked_for_deletion: Vec<String>,
    pub errors: Vec<anyhow::Error>,
}

#[derive(Debug, thiserror::Error)]
pub enum ReadObjectError {
    #[error(transparent)]
    ObjectNotFound(anyhow::Error),

    #[error(transparent)]
    Other(anyhow::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum ReadSizedObjectError<'a> {
    #[error(transparent)]
    ReadFailed(#[from] ReadObjectError),

    #[error("Object `{key}` too large ({size} > {max_size}).")]
    ObjectTooLarge {
        key: &'a str,
        size: u64,
        max_size: u64,
    },
}

pub trait Finalizable {
    fn finalize(self: Box<Self>) -> Result<(), anyhow::Error>;
}

pub trait ObjectWriter: std::io::Write + Finalizable + Send + Sync {}
