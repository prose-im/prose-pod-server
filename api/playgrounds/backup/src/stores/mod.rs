// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! Data stores.

#[cfg(feature = "destination_fs")]
pub mod file;
#[cfg(feature = "destination_s3")]
pub mod s3;

#[cfg(feature = "destination_fs")]
pub use self::file::{FsStore, FsStore as Fs};
#[cfg(feature = "destination_s3")]
pub use self::s3::{S3Store, S3Store as S3};

#[allow(async_fn_in_trait)]
pub trait ObjectStore {
    type Writer: std::io::Write + Send + Sync;

    type Reader: std::io::Read + Send + Sync;

    async fn writer(&self, key: &str) -> Result<Self::Writer, anyhow::Error>;

    /// Returns `None` if key does not exist.
    async fn reader(&self, key: &str) -> Result<Self::Reader, ReadObjectError>;

    /// Returns `None` if key does not exist or object too large.
    #[inline]
    async fn reader_if_not_too_large<'a>(
        &self,
        key: &'a str,
        max_size: u64,
    ) -> Result<Self::Reader, ReadSizedObjectError<'a>> {
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
}

pub struct ObjectMetadata {
    pub file_name: String,
    pub size_bytes: u64,
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
