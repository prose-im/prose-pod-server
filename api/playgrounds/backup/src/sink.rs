// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::io::Write;

/// Where to save backups.
/// By default, backups are uploaded to a S3-compliant object storage bucket.
pub trait BackupSink {
    type BackupWriter: Write + Send + Sync;
    type IntegrityCheckWriter: Write + Send + Sync;

    fn backup_writer(&self, backup_file_name: &str) -> Result<Self::BackupWriter, anyhow::Error>;

    fn integrity_check_writer(
        &self,
        integrity_check_file_name: &str,
    ) -> Result<Self::IntegrityCheckWriter, anyhow::Error>;
}

#[cfg(feature = "destination_s3")]
pub use self::s3::S3Sink;
#[cfg(feature = "destination_s3")]
mod s3 {
    use std::fs::File;

    use super::BackupSink;

    pub struct S3Sink;

    impl BackupSink for S3Sink {
        type BackupWriter = File;
        type IntegrityCheckWriter = File;

        fn backup_writer(
            &self,
            backup_file_name: &str,
        ) -> Result<Self::BackupWriter, anyhow::Error> {
            todo!()
        }

        fn integrity_check_writer(
            &self,
            integrity_check_file_name: &str,
        ) -> Result<Self::IntegrityCheckWriter, anyhow::Error> {
            todo!()
        }
    }
}

#[cfg(feature = "destination_file")]
pub use self::file::FileSink;
#[cfg(feature = "destination_file")]
mod file {
    use std::{
        fs::File,
        io,
        os::unix::fs::OpenOptionsExt as _,
        path::{Path, PathBuf},
    };

    use anyhow::Context as _;

    use super::BackupSink;

    pub struct FileSink {
        prefix: PathBuf,
        overwrite: bool,
        mode: u32,
    }

    impl FileSink {
        pub fn overwrite(mut self, overwrite: bool) -> Self {
            self.overwrite = overwrite;
            self
        }

        pub fn mode(mut self, mode: u32) -> Self {
            self.mode = mode;
            self
        }

        pub fn prefix(mut self, prefix: impl AsRef<Path>) -> Self {
            self.prefix = prefix.as_ref().to_path_buf();
            self
        }

        fn open(&self, path: impl AsRef<Path>) -> Result<File, io::Error> {
            if !self.prefix.is_absolute() {
                assert!(!path.as_ref().starts_with("/"), "Path should not start ");
            } else {
                panic!("OK");
            }

            File::options()
                .create(true)
                .create_new(!self.overwrite)
                .write(true)
                .truncate(self.overwrite)
                .mode(self.mode)
                .open(self.prefix.join(path))
        }
    }

    impl Default for FileSink {
        fn default() -> Self {
            Self {
                prefix: PathBuf::new(),
                overwrite: false,
                mode: 0o600,
            }
        }
    }

    impl BackupSink for FileSink {
        type BackupWriter = File;
        type IntegrityCheckWriter = File;

        fn backup_writer(
            &self,
            backup_file_name: &str,
        ) -> Result<Self::BackupWriter, anyhow::Error> {
            self.open(backup_file_name)
                .context("Could not create backup file")
        }

        fn integrity_check_writer(
            &self,
            integrity_check_file_name: &str,
        ) -> Result<Self::IntegrityCheckWriter, anyhow::Error> {
            self.open(integrity_check_file_name)
                .context("Could not create backup integrity check file")
        }
    }
}
