// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::io::Read;

pub trait BackupSource {
    type BackupReader: Read;
    type IntegrityCheckReader: Read;

    fn backup_reader(&self, backup_file_name: &str) -> Result<Self::BackupReader, anyhow::Error>;

    fn integrity_check_reader(
        &self,
        integrity_check_file_name: &str,
    ) -> Result<Self::IntegrityCheckReader, anyhow::Error>;
}

#[cfg(feature = "destination_s3")]
pub use self::s3::S3Source;
#[cfg(feature = "destination_s3")]
mod s3 {
    use std::fs::File;

    use super::BackupSource;

    pub struct S3Source;

    impl BackupSource for S3Source {
        type BackupReader = File;
        type IntegrityCheckReader = File;

        fn backup_reader(
            &self,
            backup_file_name: &str,
        ) -> Result<Self::BackupReader, anyhow::Error> {
            todo!()
        }

        fn integrity_check_reader(
            &self,
            integrity_check_file_name: &str,
        ) -> Result<Self::IntegrityCheckReader, anyhow::Error> {
            todo!()
        }
    }
}

#[cfg(feature = "destination_file")]
pub use self::file::FileSource;
#[cfg(feature = "destination_file")]
mod file {
    use std::path::{Path, PathBuf};
    use std::{fs::File, io};

    use anyhow::Context as _;

    use super::BackupSource;

    /// Read backups stored on disk.
    #[derive(Default)]
    pub struct FileSource {
        directory: PathBuf,
    }

    impl FileSource {
        pub fn new(directory: impl AsRef<Path>) -> Self {
            Self {
                directory: directory.as_ref().to_path_buf(),
            }
        }

        fn open(&self, path: impl AsRef<Path>) -> Result<File, io::Error> {
            File::options().read(true).open(self.directory.join(path))
        }
    }

    impl BackupSource for FileSource {
        type BackupReader = File;
        type IntegrityCheckReader = File;

        fn backup_reader(
            &self,
            backup_file_name: &str,
        ) -> Result<Self::BackupReader, anyhow::Error> {
            self.open(backup_file_name)
                .context("Could not open backup file")
        }

        fn integrity_check_reader(
            &self,
            integrity_check_file_name: &str,
        ) -> Result<Self::IntegrityCheckReader, anyhow::Error> {
            self.open(integrity_check_file_name)
                .context("Could not open backup integrity check file")
        }
    }
}
