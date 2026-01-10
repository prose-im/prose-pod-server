// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::{
    fs::{self, File},
    os::unix::fs::{MetadataExt, OpenOptionsExt as _},
    path::{Path, PathBuf},
};

use anyhow::Context;

use super::{ObjectMetadata, ObjectStore};

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

    async fn writer(&self, file_name: &str) -> Result<Self::Writer, anyhow::Error> {
        assert!(
            !file_name.starts_with("/"),
            "File name should not start with a `/`"
        );

        let path = self.directory.join(file_name);

        // tracing::debug!("Opening {} (write)…", path.display());

        File::options()
            .create(true)
            .create_new(!self.overwrite)
            .write(true)
            .truncate(self.overwrite)
            .mode(self.mode)
            .open(path)
            .context("Failed opening file (write)")
    }

    async fn reader(&self, file_name: &str) -> Result<Self::Reader, anyhow::Error> {
        assert!(
            !file_name.starts_with("/"),
            "File name should not start with a `/`"
        );

        let path = self.directory.join(file_name);

        // tracing::debug!("Opening {} (read)…", path.display());

        File::options()
            .read(true)
            .open(path)
            .context("Failed opening file (read)")
    }

    async fn find(&self, prefix: &str) -> Result<Vec<String>, anyhow::Error> {
        let files = fs::read_dir(&self.directory).context("Failed reading directory")?;

        let mut file_names = Vec::new();
        for entry in files.into_iter() {
            match entry {
                Ok(entry) => {
                    let file_name = entry
                        .file_name()
                        .into_string()
                        .expect("File names should only contain Unicode data");

                    if file_name.starts_with(prefix) {
                        file_names.push(file_name);
                    }
                }
                Err(err) => tracing::error!("{err:?}"),
            }
        }

        file_names.sort();

        Ok(file_names)
    }

    async fn list_all_after(&self, prefix: &str) -> Result<Vec<String>, anyhow::Error> {
        let files = fs::read_dir(&self.directory).context("Failed reading directory")?;

        let mut file_names = Vec::new();
        for entry in files.into_iter() {
            match entry {
                Ok(entry) => {
                    let file_name = entry
                        .file_name()
                        .into_string()
                        .expect("File names should only contain Unicode data");

                    if file_name.as_str() > prefix {
                        file_names.push(file_name);
                    }
                }
                Err(err) => tracing::error!("{err:?}"),
            }
        }

        file_names.sort();

        Ok(file_names)
    }

    async fn metadata(&self, file_name: &str) -> Result<ObjectMetadata, anyhow::Error> {
        let meta = fs::metadata(file_name).context("Failed getting file metadata")?;
        let created_at = meta.created().expect(
            "File creation date should be accessible on filesystems \
            where Prose is deployed",
        );

        Ok(ObjectMetadata {
            file_name: file_name.to_owned(),
            creation_date: created_at.into(),
            size: meta.size(),
        })
    }
}
