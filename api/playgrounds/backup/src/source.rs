// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::io::Read;

pub trait BackupSource {
    type Reader: Read;

    fn reader(&self, file_name: &str) -> Result<Self::Reader, anyhow::Error>;
}

#[cfg(feature = "destination_s3")]
pub use self::s3::S3Source;
#[cfg(feature = "destination_s3")]
mod s3 {
    use std::fs::File;

    use super::BackupSource;

    pub struct S3Source;

    impl BackupSource for S3Source {
        type Reader = File;

        fn reader(&self, key: &str) -> Result<Self::Reader, anyhow::Error> {
            todo!()
        }
    }
}

#[cfg(feature = "destination_file")]
pub use self::file::FileSource;
#[cfg(feature = "destination_file")]
mod file {
    use std::fs::File;
    use std::path::{Path, PathBuf};

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
    }

    impl BackupSource for FileSource {
        type Reader = File;

        fn reader(&self, file_name: &str) -> Result<Self::Reader, anyhow::Error> {
            assert!(
                !file_name.starts_with("/"),
                "File name should not start with a `/`"
            );

            File::options()
                .read(true)
                .open(self.directory.join(file_name))
                .context("Could not open backup file")
        }
    }
}
