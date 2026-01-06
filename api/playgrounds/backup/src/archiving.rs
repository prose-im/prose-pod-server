// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};

use crate::CreateBackupError;
use crate::writer_chain::WriterChainBuilder;

#[derive(Debug)]
pub struct ArchivingConfig {
    pub paths: Vec<(PathBuf, &'static str)>,
    _private: (),
}

impl ArchivingConfig {
    pub fn new(prefix: impl AsRef<Path>) -> Self {
        Self {
            paths: vec![
                (prefix.as_ref().join("var/lib/prosody"), "prosody-data"),
                (prefix.as_ref().join("etc/prosody"), "prosody-config"),
            ],
            _private: (),
        }
    }
}

impl Default for ArchivingConfig {
    fn default() -> Self {
        Self::new("/")
    }
}

pub(crate) fn check_archiving_will_succeed(
    archiving_config: &ArchivingConfig,
) -> Result<(), CreateBackupError> {
    for (local_path, _) in archiving_config.paths.iter() {
        let path = Path::new(local_path);

        if !path.exists() {
            return Err(CreateBackupError::MissingFile(path.to_path_buf()));
        }
    }

    Ok(())
}

pub(crate) fn archive_writer<W: Write>(
    archive: &mut tar::Builder<W>,
    archiving_config: &ArchivingConfig,
) -> Result<(), anyhow::Error> {
    for (local_path, archive_path) in archiving_config.paths.iter() {
        let path = Path::new(local_path);

        if path.is_file() {
            archive
                .append_path_with_name(path, archive_path)
                .with_context(|| format!("Could not archive file at '{}'", local_path.display()))?;
        } else if path.is_dir() {
            archive
                .append_dir_all(archive_path, path)
                .with_context(|| {
                    format!("Could not archive directory at '{}'", local_path.display())
                })?;
        } else {
            bail!("'{}' does not exist.", local_path.display())
        }
    }

    Ok(())
}

impl<M, F> WriterChainBuilder<M, F> {
    /// NOTE: We don’t start from zero as the Prose Pod API has to send its own
    ///   backup to the Prose Pod Server. The Pod Server then merges it with
    ///   the rest of the server’s data and creates the backup file.
    pub(crate) fn archive<InnerWriter, OuterWriter, R: std::io::Read>(
        self,
        archive: tar::Archive<R>,
        archiving_config: &ArchivingConfig,
    ) -> WriterChainBuilder<
        impl FnOnce(InnerWriter) -> Result<OuterWriter, CreateBackupError>,
        impl FnOnce(OuterWriter) -> Result<InnerWriter, CreateBackupError>,
    >
    where
        InnerWriter: Write,
        M: FnOnce(tar::Builder<InnerWriter>) -> Result<OuterWriter, CreateBackupError>,
        F: FnOnce(OuterWriter) -> Result<tar::Builder<InnerWriter>, CreateBackupError>,
    {
        let Self { make, finalize, .. } = self;

        WriterChainBuilder {
            make: move |writer: InnerWriter| {
                let mut builder: tar::Builder<_> = tar::Builder::new(writer);

                merge_archives(archive, &mut builder).map_err(CreateBackupError::CannotArchive)?;

                archive_writer(&mut builder, archiving_config)
                    .map_err(CreateBackupError::CannotArchive)?;

                make(builder)
            },

            finalize: move |writer: OuterWriter| {
                let writer = finalize(writer)?;

                let res = writer
                    // NOTE: Flushes the stream if needed.
                    .into_inner()
                    .context("Could not init archive")
                    .map_err(CreateBackupError::CannotArchive)?;

                println!("Archiving finalized.");

                Ok(res)
            },
        }
    }
}

fn merge_archives<R: std::io::Read, W: std::io::Write>(
    mut archive: tar::Archive<R>,
    builder: &mut tar::Builder<W>,
) -> Result<(), anyhow::Error> {
    for entry in archive.entries()? {
        let entry = entry?;
        let header = entry.header().to_owned();
        builder.append(&header, entry)?;
    }

    Ok(())
}
