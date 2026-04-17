// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! Archiving and extraction of archives.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, anyhow, bail};
use composable_stream::ComposableStreamBuilder;

use crate::decryption::{self, DecryptionContext, DecryptionReport};
use crate::stats::{MeteredStream, ReadStats, StreamStats};
use crate::util::debug_panic;
pub use crate::util::tar::TarSizeCalculator;
use crate::verification::VerificationOutput;
use crate::{BackupId, CreateBackupError, ExtractBackupEventHandler};

pub(crate) use self::ArchivingContext as Context;
use self::errors::*;

// WARN: Do not change as doing so would break backward compatibility.
const METADATA_FILE_NAME: &str = "metadata.json";

pub mod errors {
    #[derive(Debug, thiserror::Error)]
    pub enum CannotArchive {
        #[error("Failed computing expected size")]
        FailedComputingExpectedSize(#[source] anyhow::Error),

        #[error("Missing file: '{0}'.")]
        MissingFile(std::path::PathBuf),
    }
}

#[derive(Debug)]
pub struct ArchivingContext {
    pub blueprints: HashMap<u8, ArchiveBlueprint>,
}

#[derive(Clone)]
pub struct ArchiveBlueprint {
    pub version: u8,
    pub paths: Vec<(String, PathBuf)>,
}

impl ArchiveBlueprint {
    pub fn new<Dst, Src>(version: u8, paths: impl IntoIterator<Item = (Dst, Src)>) -> Self
    where
        Dst: ToString,
        Src: AsRef<std::path::Path>,
    {
        Self {
            version,
            paths: paths
                .into_iter()
                .map(|(dst, src)| (dst.to_string(), src.as_ref().to_path_buf()))
                .collect(),
        }
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct BackupInternalMetadata {
    pub(crate) version: u8,
}

// MARK: - Archiving

/// Returns the expected size of the archive
pub(crate) fn check_archiving_will_succeed<D: AdditionalData>(
    blueprint: &ArchiveBlueprint,
    additional_data: &Option<D>,
) -> Result<u64, CannotArchive> {
    let additional_data_size = match additional_data {
        Some(data) => data
            .expected_size()
            .map_err(CannotArchive::FailedComputingExpectedSize)?,
        None => 0u64,
    };

    for (_, local_path) in blueprint.paths.iter() {
        if !local_path.exists() {
            return Err(CannotArchive::MissingFile(local_path.to_owned()));
        }
    }

    let expected_size = TarSizeCalculator::estimate_tar_size(&blueprint.paths)
        .map_err(CannotArchive::FailedComputingExpectedSize)?
        // Add `metadata.json` (≈13 bytes).
        + TarSizeCalculator::file_entry_size(METADATA_FILE_NAME, 13)
        + additional_data_size;

    Ok(expected_size)
}

pub trait AdditionalData {
    /// TIP: Use [`TarSizeCalculator`] if needed.
    ///
    /// [`TarSizeCalculator`]: crate::archiving::TarSizeCalculator
    fn expected_size(&self) -> Result<u64, anyhow::Error>;

    fn append<W: std::io::Write>(self, builder: &mut tar::Builder<W>) -> Result<(), anyhow::Error>;
}

impl AdditionalData for () {
    fn expected_size(&self) -> Result<u64, anyhow::Error> {
        Ok(0)
    }

    fn append<W: std::io::Write>(
        self,
        _builder: &mut tar::Builder<W>,
    ) -> Result<(), anyhow::Error> {
        Ok(())
    }
}

fn archive_writer<W: Write, D: AdditionalData>(
    builder: &mut tar::Builder<W>,
    blueprint: &ArchiveBlueprint,
    additional_data: Option<D>,
) -> Result<(), anyhow::Error> {
    // Add in-memory data first, to avoid filesystem I/O if it fails.
    if let Some(additional_data) = additional_data {
        additional_data
            .append(builder)
            .context("Could not archive additional data")?;
    }

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

/// NOTE: We don’t start from zero as the Prose Pod API has to send its own
///   backup to the Prose Pod Server. The Pod Server then merges it with
///   the rest of the server’s data and creates the backup file.
pub(crate) fn archive<W: Write, D: AdditionalData>(
    blueprint: &ArchiveBlueprint,
    additional_data: Option<D>,
) -> ComposableStreamBuilder<impl FnOnce(W) -> Result<tar::Builder<W>, CreateBackupError>> {
    ComposableStreamBuilder {
        make: move |writer: W| {
            let mut builder: tar::Builder<_> = tar::Builder::new(writer);

            add_metadata_file(
                &BackupInternalMetadata {
                    version: blueprint.version,
                },
                &mut builder,
            )
            .map_err(CreateBackupError::ArchivingFailed)?;

            archive_writer(&mut builder, blueprint, additional_data)
                .map_err(CreateBackupError::ArchivingFailed)?;

            Ok(builder)
        },
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

#[derive(Debug)]
pub struct ExtractionOutput<'a> {
    /// Backup archives are unpacked in a temporary directory, that gets
    /// deleted when this is dropped. Drop when done processing data.
    pub tmp_dir: tempfile::TempDir,

    /// Blueprint of the extracted backup.
    ///
    /// Its paths are guaranteed to exist in [`tmp_dir`].
    ///
    /// [`tmp_dir`]: ExtractionOutput::tmp_dir
    pub blueprint: &'a ArchiveBlueprint,

    /// Metadata stored inside of the backup.
    pub(crate) metadata: BackupInternalMetadata,
}

#[derive(Debug, thiserror::Error)]
pub enum ExtractionError {
    #[error(transparent)]
    VerificationError(#[from] crate::verification::VerificationError),

    #[error("Invalid backup")]
    InvalidBackup(#[source] anyhow::Error),

    #[error("Unknown backup version: {0}.")]
    UnknownBackupVersion(u8),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl From<std::io::Error> for ExtractionError {
    fn from(error: std::io::Error) -> Self {
        Self::Other(anyhow::Error::from(error))
    }
}

#[derive(Debug, Default)]
pub struct ExtractionReport {
    pub extracted_bytes_count: u64,
}

struct RawReadStats<'a, H: ExtractBackupEventHandler> {
    backup_id: &'a BackupId,
    event_handler: &'a mut H,
}

impl<'a, H: ExtractBackupEventHandler> StreamStats for RawReadStats<'a, H> {
    fn record_chunk(&mut self, len: usize) {
        self.event_handler.on_raw_read(self.backup_id, len);
    }
}

pub(crate) fn extract<'a, EventHandler>(
    VerificationOutput { backup_path, .. }: &VerificationOutput,
    backup_id: &BackupId,
    blueprints: &'a HashMap<u8, ArchiveBlueprint>,
    decryption_context: &DecryptionContext,
    event_handler: &mut EventHandler,
) -> Result<ExtractionOutput<'a>, ExtractionError>
where
    EventHandler: ExtractBackupEventHandler,
{
    let backup_size = backup_path
        .metadata()
        .context("Could not read backup file metadata")
        .inspect_err(debug_panic)?
        .len();
    event_handler.on_restoration_start(backup_id, backup_size);

    let backup_file = std::fs::File::open(backup_path.as_path())
        .context("Could not open backup file")
        .inspect_err(debug_panic)?;

    let backup_reader = MeteredStream::new(
        backup_file,
        RawReadStats {
            backup_id,
            event_handler,
        },
    );

    // FIXME: https://docs.rs/sequoia-openpgp/2.1.0/sequoia_openpgp/parse/stream/struct.Decryptor.html
    //   > Signature verification and detection of ciphertext tampering requires processing the whole message first. Therefore, OpenPGP implementations supporting streaming operations necessarily must output unverified data. This has been a source of problems in the past. To alleviate this, we buffer the message first (up to 25 megabytes of net message data by default, see DEFAULT_BUFFER_SIZE), and verify the signatures if the message fits into our buffer. Nevertheless it is important to treat the data as unverified and untrustworthy until you have seen a positive verification. See Decryptor::message_processed for more information.
    let mut decryption_report = DecryptionReport::default();
    let mut decryption_stats = ReadStats::default();
    let archive_reader = decryption::reader(
        backup_reader,
        &decryption_context,
        &backup_id,
        &mut decryption_stats,
        &mut decryption_report,
    )?;

    #[cfg(feature = "compression-zstd")]
    let archive_reader: Box<dyn std::io::Read> = if backup_id.extensions.contains(&Box::from("zst"))
    {
        let decoder = zstd::Decoder::new(archive_reader).context("Cannot decompress")?;
        Box::new(decoder)
    } else {
        Box::new(archive_reader)
    };

    let mut decompression_stats = ReadStats::new();
    let archive_reader = MeteredStream::new(archive_reader, &mut decompression_stats);

    let mut extraction_report = ExtractionReport::default();
    let res = extract_archive_(archive_reader, blueprints, &mut extraction_report);

    event_handler.on_decryption_finished(backup_id, decryption_stats, decryption_report);
    event_handler.on_decompression_finished(backup_id, decompression_stats);
    event_handler.on_extraction_finished(backup_id, extraction_report);

    res
}

pub(crate) fn extract_archive_<'a, R>(
    archive_reader: R,
    blueprints: &'a HashMap<u8, ArchiveBlueprint>,
    report: &mut ExtractionReport,
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

        tracing::trace!("{} {:>6} {}", type_char, size, path.display());

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
            report.extracted_bytes_count += entry_size;
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
    tracing::debug!("Backup is version {}.", metadata.version);

    // Find where to extract entries based on the backup version.
    // NOTE: This is done before computing-heavy tasks, to fail faster.
    let Some(blueprint) = blueprints.get(&metadata.version) else {
        return Err(ExtractionError::UnknownBackupVersion(metadata.version));
    };
    tracing::debug!("Selected blueprint: {blueprint:#?}");

    // Extract into temporary directory.
    let tmp_dir = tempfile::TempDir::new()
        .context("Could not create temporary directory to extract the backup in")
        .map_err(ExtractionError::Other)?;
    tracing::debug!(
        "Extracting backup in `{path}`…",
        path = tmp_dir.path().display()
    );
    for entry in entries {
        let mut entry = entry?;

        // Unpack the archive entry.
        entry.unpack_in(tmp_dir.path()).with_context(|| {
            format!(
                "Failed extracting `{entry_path:?}` in `{dest}`",
                entry_path = entry.path(),
                dest = tmp_dir.path().display()
            )
        })?;

        if let Ok(entry_size) = entry.header().entry_size() {
            report.extracted_bytes_count += entry_size;
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

        let extracted_files = fs::read_dir(tmp_dir.path())
            .with_context(|| format!("Failed reading `{}`", tmp_dir.path().display()))?;
        for entry in extracted_files.into_iter() {
            match entry {
                Ok(entry) => {
                    // Mark path as visited.
                    // NOTE: Not warning if some path isn’t expected by the
                    //   blueprint as there are legitimate use cases for it.
                    expected_paths.remove(&entry.file_name());
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

    tracing::debug!(
        path = tmp_dir.path().display().to_string(),
        "Extraction finished."
    );

    Ok(ExtractionOutput {
        tmp_dir,
        blueprint,
        metadata,
    })
}

// MARK: - Boilerplate

impl std::fmt::Debug for ArchiveBlueprint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self { version, paths } = self;

        f.debug_struct("ArchiveBlueprint")
            .field("version", version)
            .field("paths", &crate::util::fmt::AsMap(paths))
            .finish()
    }
}
