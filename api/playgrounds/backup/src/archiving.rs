// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, anyhow, bail};
use bytes::Bytes;

use crate::writer_chain::WriterChainBuilder;
use crate::{BackupInternalMetadata, CreateBackupError};

pub const CURRENT_BACKUP_VERSION: u8 = 1;

// WARN: Do not change as doing so would break backwards compatibility.
pub(crate) const METADATA_FILE_NAME: &'static str = "metadata.json";

#[derive(Debug)]
pub struct ArchivingConfig {
    pub version: u8,
    pub paths: Vec<(&'static str, PathBuf)>,
    pub api_archive_name: &'static str,
    _private: (),
}

impl Default for ArchivingConfig {
    fn default() -> Self {
        Self::version(CURRENT_BACKUP_VERSION).unwrap()
    }
}

impl ArchivingConfig {
    pub fn version(version: u8) -> Result<Self, anyhow::Error> {
        Self::new(version, "/")
    }

    pub fn new(version: u8, prefix: impl AsRef<Path>) -> Result<Self, anyhow::Error> {
        match version {
            1 => Ok(Self {
                version,
                paths: vec![
                    ("prosody-data", prefix.as_ref().join("var/lib/prosody")),
                    ("prosody-config", prefix.as_ref().join("etc/prosody")),
                ],
                api_archive_name: "prose-pod-api-data",
                _private: (),
            }),
            n => Err(anyhow!("Unknown backup version: {n}")),
        }
    }
}

pub(crate) fn check_archiving_will_succeed(
    archiving_config: &ArchivingConfig,
) -> Result<(), CreateBackupError> {
    for (_, local_path) in archiving_config.paths.iter() {
        if !local_path.exists() {
            return Err(CreateBackupError::MissingFile(local_path.to_owned()));
        }
    }

    Ok(())
}

fn archive_writer<W: Write>(
    builder: &mut tar::Builder<W>,
    archiving_config: &ArchivingConfig,
) -> Result<(), anyhow::Error> {
    for (archive_path, local_path) in archiving_config.paths.iter() {
        let path = Path::new(local_path);

        if path.is_file() {
            builder
                .append_path_with_name(path, archive_path)
                .with_context(|| format!("Could not archive file at '{}'", local_path.display()))?;
        } else if path.is_dir() {
            builder
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
    pub(crate) fn archive<InnerWriter, OuterWriter>(
        self,
        prose_pod_api_data: Bytes,
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

                add_metadata_file(
                    &BackupInternalMetadata {
                        version: archiving_config.version,
                    },
                    &mut builder,
                )
                .map_err(CreateBackupError::CannotArchive)?;

                append_data(
                    prose_pod_api_data,
                    archiving_config.api_archive_name,
                    &mut builder,
                )
                .map_err(CreateBackupError::CannotArchive)?;

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

                Ok(res)
            },
        }
    }
}

fn add_metadata_file<W: std::io::Write>(
    metadata: &BackupInternalMetadata,
    builder: &mut tar::Builder<W>,
) -> Result<(), anyhow::Error> {
    let metadata_bytes = serde_json::to_vec(metadata)?;

    let mut header = tar::Header::new_gnu();
    header.set_size(metadata_bytes.len() as u64);
    header.set_cksum();

    builder.append_data(
        &mut header,
        METADATA_FILE_NAME,
        std::io::Cursor::new(metadata_bytes),
    )?;

    Ok(())
}

fn append_data<W: std::io::Write>(
    data: Bytes,
    path: &'static str,
    builder: &mut tar::Builder<W>,
) -> Result<(), anyhow::Error> {
    let mut header = tar::Header::new_gnu();
    header.set_size(data.len() as u64);
    header.set_cksum();

    builder.append_data(&mut header, path, std::io::Cursor::new(data))?;

    Ok(())
}

// MARK: - Unarchiving

pub struct ExtractionSuccess {
    /// The total amount of data restored on the Prose Pod Server.
    pub restored_bytes_count: u64,

    /// The Prose Pod Server cannot restore Prose Pod API data as it doesn’t
    /// have access to its file system. Here is where it’s stored before being
    /// sent to the Prose Pod API and restored there.
    pub prose_pod_api_data: fs::File,

    /// Backups archives are unpacked in a temporary directory, that gets
    /// deleted when this is dropped. In order for [`prose_pod_api_data`] to
    /// stay available, this needs to stay alive. Drop when done sending data.
    ///
    /// [`prose_pod_api_data`]: ExtractionSuccess::prose_pod_api_data
    pub tmp_dir: tempfile::TempDir,
}

pub(crate) fn extract_archive<R>(
    archive_reader: R,
    location: impl AsRef<Path>,
) -> Result<ExtractionSuccess, anyhow::Error>
where
    R: std::io::Read,
{
    use std::ffi::OsString;

    let mut extracted_bytes: u64 = 0;

    let mut archive = tar::Archive::new(archive_reader);

    let mut entries = archive.entries()?;

    #[inline]
    fn log_extracted_entry<R: std::io::Read>(entry: &tar::Entry<R>) -> Result<(), anyhow::Error> {
        let path = entry.path()?;
        let size = entry.header().size()?;
        let entry_type = entry.header().entry_type();

        let type_char = match entry_type {
            tar::EntryType::Directory => 'd',
            tar::EntryType::Regular => 'f',
            tar::EntryType::Symlink => 'l',
            _ => '?',
        };

        tracing::debug!("{} {:>6} {}", type_char, size, path.display());

        Ok(())
    }

    let metadata: BackupInternalMetadata = {
        let entry = match entries.next() {
            Some(Ok(entry)) => entry,
            Some(Err(err)) => return Err(anyhow::Error::new(err).context("Backup invalid")),
            None => return Err(anyhow!("Backup empty.")),
        };

        if let Ok(entry_size) = entry.header().entry_size() {
            extracted_bytes += entry_size;
        }

        if tracing::enabled!(tracing::Level::DEBUG) {
            log_extracted_entry(&entry)?;
        }

        let path = entry.path()?;

        if path != Path::new(METADATA_FILE_NAME) {
            return Err(anyhow!(
                "Backup invalid: Metadata file not found (first entry: {path:?})."
            ));
        }

        serde_json::from_reader(entry)?
    };

    let archiving_config = ArchivingConfig::new(metadata.version, location)?;
    let mut extract_paths: HashMap<OsString, PathBuf> = archiving_config
        .paths
        .into_iter()
        .map(|(a, b)| (OsString::from(a), b))
        .collect();

    let tmp = tempfile::TempDir::new()?;
    for entry in entries {
        let mut entry = entry?;

        entry.unpack_in(tmp.path())?;

        if let Ok(entry_size) = entry.header().entry_size() {
            extracted_bytes += entry_size;
        }

        #[cfg(debug_assertions)]
        log_extracted_entry(&entry)?;
    }

    let extracted_files = fs::read_dir(tmp.path())?;
    for entry in extracted_files.into_iter() {
        match entry {
            Ok(entry) => {
                let entry_name = entry.file_name();

                if entry_name == OsString::from(archiving_config.api_archive_name) {
                    continue;
                }

                let Some(dst) = extract_paths.remove(&entry_name) else {
                    tracing::warn!(
                        "Don’t know where to extract '{src}', skipping.",
                        src = entry_name.display()
                    );
                    continue;
                };

                crate::util::safe_replace(entry.path(), &dst)?;
            }
            Err(err) => tracing::error!("{err:?}"),
        }
    }

    if !extract_paths.is_empty() {
        return Err(anyhow!(
            "Backup invalid: Missing data ({:?}).",
            extract_paths.keys().collect::<Vec<_>>()
        ));
    }

    let api_archive = fs::File::open(tmp.path().join(archiving_config.api_archive_name))?;

    Ok(ExtractionSuccess {
        restored_bytes_count: extracted_bytes,
        prose_pod_api_data: api_archive,
        tmp_dir: tmp,
    })
}
