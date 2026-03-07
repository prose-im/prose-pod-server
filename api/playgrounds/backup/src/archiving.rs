// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, anyhow, bail};

use crate::decryption::{self, DecryptionContext, DecryptionReport};
use crate::stats::{ReadStats, StatsReader};
use crate::util::debug_panic;
use crate::verification::VerificationOutput;
use crate::writer_chain::WriterChainBuilder;
use crate::{BackupFileNameComponents, CreateBackupError};

// WARN: Do not change as doing so would break backward compatibility.
const METADATA_FILE_NAME: &'static str = "metadata.json";

#[non_exhaustive]
#[derive(Debug)]
pub struct ArchiveBlueprint {
    pub version: u8,
    pub paths: Vec<(String, PathBuf)>,
}

impl ArchiveBlueprint {
    // NOTE: The reason why we have this trivial constructor is so we can later
    //   add non-breaking support for in-memory reads instead of forcing one
    //   to store files.
    pub fn from_paths(version: u8, paths: Vec<(String, PathBuf)>) -> Self {
        Self { version, paths }
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct BackupInternalMetadata {
    version: u8,
}

// MARK: - Archiving

pub(crate) fn check_archiving_will_succeed(
    blueprint: &ArchiveBlueprint,
) -> Result<(), CreateBackupError> {
    for (_, local_path) in blueprint.paths.iter() {
        if !local_path.exists() {
            return Err(CreateBackupError::MissingFile(local_path.to_owned()));
        }
    }

    Ok(())
}

fn archive_writer<W: Write>(
    builder: &mut tar::Builder<W>,
    blueprint: &ArchiveBlueprint,
) -> Result<(), anyhow::Error> {
    for (archive_path, local_path) in blueprint.paths.iter() {
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
        blueprint: &ArchiveBlueprint,
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
                        version: blueprint.version,
                    },
                    &mut builder,
                )
                .map_err(CreateBackupError::CannotArchive)?;

                archive_writer(&mut builder, blueprint)
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
    let metadata_bytes = json::to_vec(metadata)?;

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

// MARK: - Extraction (unarchiving)

pub struct ExtractionOutput<'a> {
    /// Backup archives are unpacked in a temporary directory, that gets
    /// deleted when this is dropped. Drop when done processing data.
    ///
    /// [`prose_pod_api_data`]: ExtractionSuccess::prose_pod_api_data
    pub tmp_dir: tempfile::TempDir,

    /// Blueprint of the extracted backup.
    ///
    /// Its paths are guaranteed to exist in [`tmp_dir`].
    ///
    /// [`tmp_dir`]: ExtractionSuccess::tmp_dir
    pub blueprint: &'a ArchiveBlueprint,
}

#[derive(Debug, Default)]
pub struct ExtractionStats {
    pub raw_read_stats: ReadStats,
    pub decryption_stats: ReadStats,
    pub decompression_stats: ReadStats,

    /// Total amount of data extracted in [`ExtractionOutput::tmp_dir`].
    pub extracted_bytes_count: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum ExtractionError {
    #[error(transparent)]
    VerificationError(#[from] crate::verification::VerificationError),

    #[error("Invalid backup")]
    InvalidBackup(#[source] anyhow::Error),

    #[error("Unknown backup version: {0}")]
    UnknownBackupVersion(u8),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl From<std::io::Error> for ExtractionError {
    fn from(error: std::io::Error) -> Self {
        Self::Other(anyhow::Error::from(error))
    }
}

pub(crate) fn extract<'a>(
    VerificationOutput { backup_path, .. }: &VerificationOutput,
    parsed_backup_name: &BackupFileNameComponents<'_>,
    blueprints: &'a HashMap<u8, ArchiveBlueprint>,
    decryption_context: &DecryptionContext,
    decryption_report: &mut DecryptionReport,
    stats: &mut ExtractionStats,
) -> Result<ExtractionOutput<'a>, ExtractionError> {
    let backup_file = std::fs::File::open(backup_path)
        .context("Could not open backup file")
        .inspect_err(debug_panic)?;

    let backup_reader = StatsReader::new(backup_file, &mut stats.raw_read_stats);

    // FIXME: https://docs.rs/sequoia-openpgp/2.1.0/sequoia_openpgp/parse/stream/struct.Decryptor.html
    //   > Signature verification and detection of ciphertext tampering requires processing the whole message first. Therefore, OpenPGP implementations supporting streaming operations necessarily must output unverified data. This has been a source of problems in the past. To alleviate this, we buffer the message first (up to 25 megabytes of net message data by default, see DEFAULT_BUFFER_SIZE), and verify the signatures if the message fits into our buffer. Nevertheless it is important to treat the data as unverified and untrustworthy until you have seen a positive verification. See Decryptor::message_processed for more information.
    let compressed_archive_reader = decryption::reader(
        backup_reader,
        &decryption_context,
        &parsed_backup_name,
        &mut stats.decryption_stats,
        decryption_report,
    )?;

    let archive_bytes =
        zstd::Decoder::new(compressed_archive_reader).context("Cannot decompress")?;

    let archive_bytes = StatsReader::new(archive_bytes, &mut stats.decompression_stats);

    extract_archive_(archive_bytes, blueprints, &mut stats.extracted_bytes_count)
}

pub(crate) fn extract_archive_<'a, R>(
    archive_reader: R,
    blueprints: &'a HashMap<u8, ArchiveBlueprint>,
    extracted_bytes_count: &mut u64,
) -> Result<ExtractionOutput<'a>, ExtractionError>
where
    R: std::io::Read,
{
    use std::ffi::OsString;

    let mut archive = tar::Archive::new(archive_reader);

    let mut entries = archive.entries().map_err(anyhow::Error::from)?;

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

    // Read first archive entry, which should be the metadata file.
    let metadata: BackupInternalMetadata = {
        let entry = match entries.next() {
            Some(Ok(entry)) => entry,
            Some(Err(err)) => return Err(ExtractionError::InvalidBackup(anyhow::Error::from(err))),
            None => return Err(ExtractionError::InvalidBackup(anyhow!("Backup empty."))),
        };

        if let Ok(entry_size) = entry.header().entry_size() {
            *extracted_bytes_count += entry_size;
        }

        if tracing::enabled!(tracing::Level::DEBUG) {
            log_extracted_entry(&entry)?;
        }

        let path = entry.path()?;

        if path != Path::new(METADATA_FILE_NAME) {
            return Err(ExtractionError::InvalidBackup(anyhow!(
                "Metadata file not found (first entry: {path:?})."
            )));
        }

        json::from_reader(entry).context("Invalid metadata file")?
    };

    // Find where to extract entries based on the backup version.
    // NOTE: This is done before computing-heavy tasks, to fail faster.
    let Some(blueprint) = blueprints.get(&metadata.version) else {
        return Err(ExtractionError::UnknownBackupVersion(metadata.version));
    };
    // Soundness check.
    assert_eq!(blueprint.version, metadata.version);

    // Extract into temporary directory.
    let tmp_dir = tempfile::TempDir::new()
        .context("Could not create temporary directory to extract the backup in")
        .map_err(ExtractionError::Other)?;
    for entry in entries {
        let mut entry = entry?;

        // Unpack the archive entry.
        entry.unpack_in(tmp_dir.path())?;

        if let Ok(entry_size) = entry.header().entry_size() {
            *extracted_bytes_count += entry_size;
        }

        #[cfg(debug_assertions)]
        log_extracted_entry(&entry)?;
    }

    // Make sure all expected paths were present,
    // before attempting the real restoration.
    {
        let mut expected_paths: HashSet<OsString> = blueprint
            .paths
            .iter()
            .map(|(src, _)| OsString::from(&src))
            .collect();

        let extracted_files = fs::read_dir(tmp_dir.path())?;
        for entry in extracted_files.into_iter() {
            match entry {
                Ok(entry) => {
                    let entry_name = entry.file_name();

                    // Mark path as visited.
                    if !expected_paths.remove(&entry_name) {
                        tracing::warn!(
                            "Extracted unknown entry '{src}'.",
                            src = entry_name.display()
                        );
                        continue;
                    };
                }
                Err(err) => tracing::error!("{err:?}"),
            }
        }

        if !expected_paths.is_empty() {
            return Err(ExtractionError::InvalidBackup(anyhow!(
                "Missing data ({expected_paths:?})."
            )));
        }
    }

    Ok(ExtractionOutput { tmp_dir, blueprint })
}
