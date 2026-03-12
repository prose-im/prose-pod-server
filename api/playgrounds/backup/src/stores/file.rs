// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::{
    fs::{self, File},
    os::unix::fs::{MetadataExt, OpenOptionsExt as _},
    path::{Path, PathBuf},
};

use anyhow::Context as _;

use crate::config::StorageFsConfig;

use super::prelude::*;

/// Read and write backups on disk.
pub struct FsStore {
    pub directory: PathBuf,
    pub overwrite: bool,
    pub mode: u32,
}

impl Default for FsStore {
    #[inline(always)]
    fn default() -> Self {
        Self {
            directory: PathBuf::new(),
            overwrite: false,
            mode: 0o600,
        }
    }
}

impl FsStore {
    pub fn try_from_config(
        config: &StorageFsConfig,
        min_permissions: u32,
    ) -> Result<Self, anyhow::Error> {
        let StorageFsConfig {
            directory,
            overwrite,
            mode,
        } = config;

        match validate_mode(mode, &min_permissions) {
            ModeResult::Ok => {}
            ModeResult::Warn => tracing::warn!(
                "Permissions `{mode:#o}` are not all necessary for `{directory}`. \
                Recommended: `{min_permissions:#o}`. \
                Make sure you wrote an octal number in the configuration \
                (e.g. `0o600` and not `600`).",
                directory = directory.display()
            ),
            ModeResult::Err => anyhow::bail!(
                "Invalid permissions `{mode:#o}` for `{directory}`. \
                Recommended: `{min_permissions:#o}`. \
                Make sure you wrote an octal number in the configuration \
                (e.g. `0o600` and not `600`).",
                directory = directory.display()
            ),
        }

        Ok(Self {
            directory: PathBuf::clone(directory),
            overwrite: *overwrite,
            mode: **mode,
        })
    }
}

enum ModeResult {
    Ok,
    Warn,
    Err,
}

// TODO: Add tests.
fn validate_mode(mode: &u32, min_permissions: &u32) -> ModeResult {
    if mode == min_permissions {
        // Exit early if exact match.
        ModeResult::Ok
    } else if mode & 0o117 != 0 {
        ModeResult::Err
    } else if mode & min_permissions != *min_permissions {
        ModeResult::Err
    } else if mode ^ min_permissions != 0 {
        ModeResult::Warn
    } else {
        ModeResult::Ok
    }
}

#[async_trait::async_trait]
impl ObjectStore for FsStore {
    async fn writer(&self, file_name: &str) -> Result<Box<DynObjectWriter>, anyhow::Error> {
        assert!(
            !file_name.starts_with("/"),
            "File name should not start with a `/`"
        );
        // Safety check: Do not allow unsafe permission bits.
        assert!(self.mode & 0o117 == 0);

        let path = self.directory.join(file_name);

        // tracing::debug!("Opening {} (write)…", path.display());

        let writer = File::options()
            .create(true)
            .create_new(!self.overwrite)
            .write(true)
            .truncate(self.overwrite)
            .mode(self.mode)
            .open(path)
            .context("Failed opening file (write)")?;

        Ok(Box::new(writer))
    }

    async fn reader(&self, file_name: &str) -> Result<Box<DynObjectReader>, ReadObjectError> {
        assert!(
            !file_name.starts_with("/"),
            "File name should not start with a `/`"
        );

        let path = self.directory.join(file_name);

        // tracing::debug!("Opening {} (read)…", path.display());

        match File::options().read(true).open(path) {
            Ok(reader) => Ok(Box::new(reader)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                Err(ReadObjectError::ObjectNotFound(anyhow::Error::from(err)))
            }
            Err(err) => Err(ReadObjectError::Other(
                anyhow::Error::from(err).context("Failed opening file (read)"),
            )),
        }
    }

    async fn exists(&self, key: &str) -> Result<bool, anyhow::Error> {
        Ok(self.directory.join(key).exists())
    }

    async fn find(&self, prefix: &str) -> Result<Vec<ObjectMetadata>, anyhow::Error> {
        let files = fs::read_dir(&self.directory).context(format!(
            "Failed reading directory `{}`",
            self.directory.display()
        ))?;

        let mut results: Vec<ObjectMetadata> = Vec::new();
        for entry in files.into_iter() {
            match entry {
                Ok(entry) => {
                    let file_name = entry
                        .file_name()
                        .into_string()
                        .expect("File names should only contain Unicode data");

                    let meta = entry.metadata()?;

                    if file_name.starts_with(prefix) {
                        results.push(ObjectMetadata {
                            file_name,
                            size_bytes: meta.len(),
                        });
                    }
                }
                Err(err) => tracing::error!("{err:?}"),
            }
        }

        results.sort_unstable_by(|a, b| a.file_name.cmp(&b.file_name));

        Ok(results)
    }

    async fn list_all_after(&self, prefix: &str) -> Result<Vec<ObjectMetadata>, anyhow::Error> {
        let files = fs::read_dir(&self.directory).context(format!(
            "Failed reading directory `{}`",
            self.directory.display()
        ))?;

        let mut results: Vec<ObjectMetadata> = Vec::new();
        for entry in files.into_iter() {
            match entry {
                Ok(entry) => {
                    let file_name = entry
                        .file_name()
                        .into_string()
                        .expect("File names should only contain Unicode data");

                    let meta = entry.metadata()?;

                    if file_name.as_str() > prefix {
                        results.push(ObjectMetadata {
                            file_name,
                            size_bytes: meta.len(),
                        });
                    }
                }
                Err(err) => tracing::error!("{err:?}"),
            }
        }

        results.sort_unstable_by(|a, b| a.file_name.cmp(&b.file_name));

        Ok(results)
    }

    async fn metadata(&self, file_name: &str) -> Result<ObjectMetadata, ReadObjectError> {
        let file_path = self.directory.join(file_name);

        let meta = fs::metadata(&file_path)
            .context("Failed getting file metadata")
            .map_err(|err| ReadObjectError::ObjectNotFound(anyhow::Error::from(err)))?;

        Ok(ObjectMetadata {
            file_name: file_name.to_owned(),
            size_bytes: meta.size(),
        })
    }

    async fn download_url(
        &self,
        file_name: &str,
        _ttl: &std::time::Duration,
    ) -> Result<String, anyhow::Error> {
        match self.directory.join(file_name).to_str() {
            Some(str) => Ok(format!("file://{str}")),
            None => Err(anyhow::Error::msg("File path is non-Unicode.")),
        }
    }

    async fn delete(&self, file_name: &str) -> Result<DeletedState, anyhow::Error> {
        match std::fs::remove_file(self.directory.join(file_name)) {
            Ok(()) => Ok(DeletedState::Deleted),
            Err(err) => Err(anyhow::Error::from(err)),
        }
    }

    async fn delete_all(&self, prefix: &str) -> Result<BulkDeleteOutput, anyhow::Error> {
        let files = fs::read_dir(&self.directory).context(format!(
            "Failed reading directory `{}`",
            self.directory.display()
        ))?;

        let mut output = BulkDeleteOutput::default();
        for entry in files.into_iter() {
            match entry {
                Ok(entry) => {
                    let file_name = entry
                        .file_name()
                        .into_string()
                        .expect("File names should only contain Unicode data");

                    if file_name.as_str() > prefix {
                        match self.delete(&file_name).await {
                            Ok(_) => output.deleted.push(file_name),
                            Err(err) => output.errors.push(
                                anyhow::Error::from(err)
                                    .context(format!("File `{file_name}` not deleted")),
                            ),
                        }
                    }
                }
                Err(err) => output.errors.push(anyhow::Error::from(err).context(format!(
                    "Invalid entry in directory `{}`",
                    self.directory.display()
                ))),
            }
        }

        Ok(output)
    }
}

impl super::Finalizable for File {
    fn finalize(self: Box<Self>) -> Result<(), anyhow::Error> {
        Ok(())
    }
}

impl super::ObjectWriter for File {}

// MARK: Builder

pub struct FsStoreBuilder {
    res: FsStore,
}

impl FsStore {
    #[inline(always)]
    pub fn builder() -> FsStoreBuilder {
        FsStoreBuilder {
            res: Self::default(),
        }
    }
}

impl FsStoreBuilder {
    #[inline(always)]
    pub fn overwrite(mut self, overwrite: bool) -> Self {
        self.res.overwrite = overwrite;
        self
    }

    #[inline(always)]
    pub fn mode(mut self, mode: u32) -> Self {
        self.res.mode = mode;
        self
    }

    #[inline(always)]
    pub fn directory(mut self, directory: impl AsRef<Path>) -> Self {
        self.res.directory = directory.as_ref().to_path_buf();
        self
    }

    #[inline(always)]
    pub fn build(self) -> FsStore {
        self.res
    }
}
