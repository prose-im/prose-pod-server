// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! Archiving and extraction of archives.

use std::collections::HashMap;
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, anyhow, bail};
use composable_stream::ComposableStreamBuilder;

use crate::decryption::{self, DecryptionContext, DecryptionEventHandler};
use crate::event_handlers::NoopEventHandler;
use crate::restoration::ExtractionError;
use crate::stats::{MeteredStream, NoopStats};
use crate::util::debug_panic;
pub use crate::util::tar::TarSizeCalculator;
use crate::verification::VerificationOutput;
use crate::{BackupId, CreateBackupError};

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
    pub paths: Vec<(OsString, PathBuf)>,
}

impl ArchiveBlueprint {
    pub fn new<Dst, Src>(version: u8, paths: impl IntoIterator<Item = (Dst, Src)>) -> Self
    where
        Dst: Into<OsString>,
        Src: Into<PathBuf>,
    {
        Self {
            version,
            paths: paths
                .into_iter()
                .map(|(dst, src)| (dst.into(), src.into()))
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
            return Err(CannotArchive::MissingFile(PathBuf::clone(local_path)));
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

// MARK: - Unarchiving

#[derive(Debug)]
pub struct GetMetadataOutput<'a> {
    /// Blueprint of the backup.
    pub blueprint: &'a ArchiveBlueprint,

    /// Metadata stored inside of the backup.
    #[allow(dead_code)]
    pub(crate) metadata: BackupInternalMetadata,
}

#[allow(unused_variables)]
pub trait ExtractBackupEventHandler: Send + Sync {
    #[inline]
    fn on_extraction_progress(&mut self, backup_id: &BackupId, len: u64) {}
}

#[derive(Debug, Default)]
pub struct ExtractionReport {
    pub extracted_bytes_count: u64,
}

impl ExtractBackupEventHandler for ExtractionReport {
    fn on_extraction_progress(&mut self, _backup_id: &BackupId, entry_size: u64) {
        self.extracted_bytes_count += entry_size;
    }
}

pub(crate) fn archive_reader<'r>(
    backup_reader: impl std::io::Read + Send + Sync + 'r,
    backup_id: &'r BackupId,
    decryption_context: &'r DecryptionContext,
    stats: impl crate::stats::StreamStats + 'r,
    decryption_event_handler: &'r mut impl DecryptionEventHandler,
    decompression_stats: impl crate::stats::StreamStats + 'r,
) -> Result<tar::Archive<impl std::io::Read + 'r>, ExtractionError> {
    // FIXME: https://docs.rs/sequoia-openpgp/2.1.0/sequoia_openpgp/parse/stream/struct.Decryptor.html
    //   > Signature verification and detection of ciphertext tampering requires processing the whole message first. Therefore, OpenPGP implementations supporting streaming operations necessarily must output unverified data. This has been a source of problems in the past. To alleviate this, we buffer the message first (up to 25 megabytes of net message data by default, see DEFAULT_BUFFER_SIZE), and verify the signatures if the message fits into our buffer. Nevertheless it is important to treat the data as unverified and untrustworthy until you have seen a positive verification. See Decryptor::message_processed for more information.
    let archive_reader = decryption::reader(
        backup_reader,
        &decryption_context,
        &backup_id,
        stats,
        decryption_event_handler,
    )?;

    #[cfg(feature = "compression-zstd")]
    let archive_reader: Box<dyn std::io::Read> = if backup_id.extensions.contains(&Box::from("zst"))
    {
        let decoder = zstd::Decoder::new(archive_reader).context("Cannot decompress")?;
        Box::new(decoder)
    } else {
        Box::new(archive_reader)
    };

    let archive_reader = MeteredStream::new(archive_reader, decompression_stats);

    Ok(tar::Archive::new(archive_reader))
}

pub(crate) fn read_metadata<R: std::io::Read>(
    entries: &mut tar::Entries<R>,
    backup_id: &BackupId,
    event_handler: &mut impl ExtractBackupEventHandler,
) -> Result<BackupInternalMetadata, ExtractionError> {
    // Read first archive entry, which should be the metadata file.
    let metadata: BackupInternalMetadata = {
        let entry = match entries.next() {
            Some(Ok(entry)) => entry,
            Some(Err(err)) => return Err(ExtractionError::InvalidBackup(anyhow::Error::from(err))),
            None => return Err(ExtractionError::InvalidBackup(anyhow!("Backup empty."))),
        };

        if let Ok(entry_size) = entry.header().entry_size() {
            event_handler.on_extraction_progress(backup_id, entry_size);
        }

        #[cfg(debug_assertions)]
        log_extracted_entry(&entry)?;

        let path = entry.path()?;

        if path != Path::new(METADATA_FILE_NAME) {
            return Err(ExtractionError::InvalidBackup(anyhow!(
                "Metadata file not found (first entry: {path:?})."
            )));
        }

        json::from_reader(entry).context("Invalid metadata file")?
    };

    Ok(metadata)
}

// TODO: Move to an event handler?
#[cfg(debug_assertions)]
#[inline]
pub(crate) fn log_extracted_entry<R: std::io::Read>(
    entry: &tar::Entry<R>,
) -> Result<(), anyhow::Error> {
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

pub(crate) fn get_metadata<'a>(
    VerificationOutput { backup_path, .. }: &VerificationOutput,
    backup_id: &BackupId,
    decryption_context: &DecryptionContext,
    decryption_event_handler: &mut impl DecryptionEventHandler,
    blueprints: &'a HashMap<u8, ArchiveBlueprint>,
) -> Result<GetMetadataOutput<'a>, ExtractionError> {
    let backup_file = std::fs::File::open(backup_path.as_path())
        .context("Could not open backup file")
        .inspect_err(debug_panic)?;

    let mut archive = archive_reader(
        backup_file,
        backup_id,
        decryption_context,
        NoopStats,
        decryption_event_handler,
        NoopStats,
    )?;

    let metadata = {
        let mut entries = archive.entries().map_err(anyhow::Error::from)?;
        read_metadata(&mut entries, backup_id, &mut NoopEventHandler)?
    };

    // Find where to extract entries based on the backup version.
    // NOTE: This is done before computing-heavy tasks, to fail faster.
    let Some(blueprint) = blueprints.get(&metadata.version) else {
        return Err(ExtractionError::UnknownBackupVersion(metadata.version));
    };

    // NOTE: We do not walk the entire archive to check for bad contents, as
    //   it doesn’t seem necessary and would be far too expensive for a simple
    //   “get details” operation. Restoration will fail anyway, it’s okay if
    //   this is not 100% accurate.

    Ok(GetMetadataOutput {
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
