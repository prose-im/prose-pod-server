// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::{
    fs::File,
    os::unix::fs::OpenOptionsExt as _,
    path::{Path, PathBuf},
};

use anyhow::Context as _;

use super::ObjectStore;

/// Read and write backups on disk.
pub struct FsStore {
    directory: PathBuf,
    overwrite: bool,
    mode: u32,
}

impl FsStore {
    pub fn overwrite(mut self, overwrite: bool) -> Self {
        self.overwrite = overwrite;
        self
    }

    pub fn mode(mut self, mode: u32) -> Self {
        self.mode = mode;
        self
    }

    pub fn directory(mut self, directory: impl AsRef<Path>) -> Self {
        self.directory = directory.as_ref().to_path_buf();
        self
    }
}

impl Default for FsStore {
    fn default() -> Self {
        Self {
            directory: PathBuf::new(),
            overwrite: false,
            mode: 0o600,
        }
    }
}

impl ObjectStore for FsStore {
    type Writer = File;
    type Reader = File;

    fn writer(&self, file_name: &str) -> Result<Self::Writer, anyhow::Error> {
        assert!(
            !file_name.starts_with("/"),
            "File name should not start with a `/`"
        );

        File::options()
            .create(true)
            .create_new(!self.overwrite)
            .write(true)
            .truncate(self.overwrite)
            .mode(self.mode)
            .open(self.directory.join(file_name))
            .context("Failed opening file")
    }

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
