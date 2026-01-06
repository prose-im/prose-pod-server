// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

#[cfg(feature = "destination_fs")]
pub mod file;
#[cfg(feature = "destination_s3")]
pub mod s3;

#[cfg(feature = "destination_fs")]
pub use self::file::{FsStore, FsStore as Fs};
#[cfg(feature = "destination_s3")]
pub use self::s3::{S3Store, S3Store as S3};

pub trait ObjectStore {
    type Writer: std::io::Write + Send + Sync;
    type Reader: std::io::Read + Send + Sync;

    fn writer(&self, file_name: &str) -> Result<Self::Writer, anyhow::Error>;
    fn reader(&self, file_name: &str) -> Result<Self::Reader, anyhow::Error>;
}
