// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

extern crate aws_sdk_s3 as s3;
extern crate sequoia_openpgp as openpgp;

mod archiving;
mod compression;
mod encryption;
mod gpg;
mod integrity;
pub mod stores;
mod util;
mod writer_chain;

use std::{
    collections::HashMap,
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

use anyhow::{Context, anyhow};
use bytes::Bytes;

use crate::{
    archiving::{METADATA_FILE_NAME, check_archiving_will_succeed},
    stores::{ObjectStore, S3Store},
    util::{safe_replace, stats_reader::StatsReader},
};

pub use self::{
    archiving::ArchivingConfig, archiving::CURRENT_BACKUP_VERSION as CURRENT_VERSION,
    compression::CompressionConfig, encryption::EncryptionConfig, integrity::IntegrityConfig,
};

// MARK: Service

pub type BackupService<BackupStore = S3Store, IntegrityCheckStore = S3Store> =
    ProseBackupService<BackupStore, IntegrityCheckStore>;

/// ```text
/// ## Create backup
///
/// Abstract:
///   File -> BackupWriter -> dyn BackupSink
///
/// Prod:
///   -> S3Sink
///
/// Tests:
///   -> FileSink
///
/// ## Restore backup
///
/// Abstract:
///   File -> BackupWriter -> dyn BackupSink
///
/// Prod:
///   S3Source -> BackupReader ->
///
/// Tests:
///   -> FileSink
/// ```
pub struct ProseBackupService<BackupStore, IntegrityCheckStore> {
    pub archiving_config: ArchivingConfig,
    pub compression_config: CompressionConfig,
    pub encryption_config: Option<EncryptionConfig>,
    pub integrity_config: Option<IntegrityConfig>,
    pub backup_store: BackupStore,
    pub integrity_check_store: IntegrityCheckStore,
}

impl<S1, S2> ProseBackupService<S1, S2>
where
    S1: ObjectStore,
    S2: ObjectStore,
{
    /// Everything is done in a single stream of the following shape:
    ///
    /// ```text
    ///                         ┌─/var/lib/prosody
    ///                         ├─/etc/prosody
    ///                         ├─/etc/prose
    ///                         ├─/var/lib/prose-pod-api
    ///                         ├─…
    ///                    ┌────┴────┐
    ///                    │ Archive │
    ///                    │  (tar)  │
    ///                    └────┬────┘
    ///                    ┌────┴─────┐
    ///                    │ Compress │
    ///                    │  (zstd)  │
    ///                    └────┬─────┘
    ///                         │ GPG encryption enabled?
    ///                         ◇──────┐
    ///                     Yes │      │ No
    ///                    ┌────┴────┐ │
    ///                    │ Encrypt | │
    ///                    |  (GPG)  │ │
    ///                    └────┬────┘ │
    ///                         ◇──────┘
    ///              ╺━┯━━━━━━━━┷━━━━━━━━┯━╸
    ///            ┌───┴────┐            │ GPG signing enabled?
    ///            | Upload |            ◇───────────┐
    ///            | backup |        Yes │           │ No
    ///            |  (S3)  |        ┌───┴───┐ ┌─────┴─────┐
    ///            └────────┘        │ Sign  | │   Hash    |
    ///                              | (GPG) │ | (SHA 256) │
    ///                              └───┬───┘ └─────┬─────┘
    ///                                  ◇───────────┘
    ///                         ┌────────┴─────────┐
    ///                         | Upload integrity |
    ///                         |    check (S3)    |
    ///                         └──────────────────┘
    /// ```
    pub async fn create_backup(
        &self,
        backup_name: &str,
        prose_pod_api_data: Bytes,
    ) -> Result<(String, String), CreateBackupError> {
        check_archiving_will_succeed(&self.archiving_config)?;

        let backup_file_name = if self.encryption_config.is_some() {
            format!("{backup_name}.tar.zst.gpg")
        } else {
            format!("{backup_name}.tar.zst")
        };

        let upload_backup = self
            .backup_store
            .writer(&backup_file_name)
            .await
            .map_err(CreateBackupError::CannotCreateSink)?;

        let mut integrity_check: Vec<u8> = Vec::new();

        let (mut gen_integrity_check, finalize_integrity_check) = writer_chain::builder()
            .integrity_check(self.integrity_config.as_ref())
            .build(&mut integrity_check)?;

        let (writer, finalize_backup) = writer_chain::builder()
            .archive(prose_pod_api_data, &self.archiving_config)
            .compress(&self.compression_config)
            .encrypt_if_possible(self.encryption_config.as_ref())
            .tee(&mut gen_integrity_check)
            .build(upload_backup)?;

        () = finalize_backup(writer)?;
        () = finalize_integrity_check(gen_integrity_check)?;

        let integrity_check_file_name = if self.integrity_config.is_some() {
            format!("{backup_file_name}.sig")
        } else {
            format!("{backup_file_name}.sha256")
        };

        let mut upload_integrity_check = self
            .integrity_check_store
            .writer(&integrity_check_file_name)
            .await
            .map_err(CreateBackupError::CannotCreateSink)?;

        let mut cursor = std::io::Cursor::new(integrity_check);
        std::io::copy(&mut cursor, &mut upload_integrity_check)
            .map_err(CreateBackupError::IntegrityCheckUploadFailed)?;

        Ok((backup_file_name, integrity_check_file_name))
    }

    pub async fn list_backups(&self) -> Result<Vec<String>, anyhow::Error> {
        self.backup_store.list_all().await
    }

    #[must_use]
    pub async fn restore_backup(
        &self,
        backup_name: &str,
        location: impl AsRef<Path>,
    ) -> Result<RestoreSuccess, anyhow::Error> {
        use crate::util::stats_reader::ReadStats;
        use crate::writer_chain::either::Either;
        use openpgp::parse::Parse as _;
        use openpgp::parse::stream::DecryptorBuilder;

        let integrity_checks = (self.integrity_check_store)
            .find(backup_name)
            .await
            .context("Failed listing integrity checks")?;

        // Check signature first.
        let signature_name = integrity_checks
            .iter()
            .find(|name| name.as_str() == format!("{backup_name}.sig").as_str());

        if let Some(signature_name) = signature_name {
            self.check_backup_integrity(&backup_name, &signature_name)
                .await
                .context("Integrity check failed")?;

            let backup_reader = (self.backup_store)
                .reader(backup_name)
                .await
                .map_err(ExtractBackupError::CannotCreateReader)?;

            let mut raw_read_stats = ReadStats::new();
            let backup_reader = StatsReader::new(backup_reader, &mut raw_read_stats);

            let mut decryption_stats = ReadStats::new();
            let compressed_archive_reader = if backup_name.ends_with(".gpg") {
                if let Some(config) = self.encryption_config.as_ref() {
                    let decryptor = DecryptorBuilder::from_reader(backup_reader)
                        .context("Failed creating decryptor")?
                        .with_policy(config.policy.as_ref(), None, config)
                        .map_err(ExtractBackupError::DecryptionFailed)?;

                    let decryptor = StatsReader::new(decryptor, &mut decryption_stats);

                    Either::A(decryptor)
                } else {
                    return Err(anyhow!(
                        "Encryption not configured. Cannot find private keys.",
                    ));
                }
            } else {
                if cfg!(debug_assertions) {
                    eprintln!("NOT DECRYPTING");
                }

                Either::B(backup_reader)
            };

            let archive_bytes =
                zstd::Decoder::new(compressed_archive_reader).context("Cannot decompress")?;

            let mut decompression_stats = ReadStats::new();
            let archive_bytes = StatsReader::new(archive_bytes, &mut decompression_stats);

            let restore_result = extract_archive(archive_bytes, location)
                .map_err(ExtractBackupError::ExtractionFailed)?;
            let unarchived_size = restore_result.restored_bytes_count;

            println!("Stats:");
            println!("  Read:         {raw_read_stats}");
            println!("  Decrypted:    {decryption_stats}");
            println!("  Decompressed: {decompression_stats}");
            println!("  Unarchived:   {unarchived_size}B");

            fn size_ratio(read: u64, reference: &ReadStats) -> f64 {
                let read: u32 = read.min(u64::from(u32::MAX)) as u32;
                let reference: u32 = reference.bytes_read().min(u64::from(u32::MAX)) as u32;
                f64::from(read) / f64::from(reference)
            }
            println!("Size ratios:");
            println!(
                "  Raw read:      {:.2}x",
                size_ratio(raw_read_stats.bytes_read(), &raw_read_stats)
            );
            println!(
                "  Decryption:    {:.2}x",
                size_ratio(decryption_stats.bytes_read(), &raw_read_stats)
            );
            println!(
                "  Decompression: {:.2}x",
                size_ratio(decompression_stats.bytes_read(), &raw_read_stats)
            );
            println!(
                "  Unarchiving:   {:.2}x",
                size_ratio(unarchived_size, &raw_read_stats)
            );

            return Ok(restore_result);
        }

        // FIXME: Do not look for hash if signature is mandatory.

        // Check hash if no signature exist.
        let todo = "Hash check";

        Ok(todo!())
    }
}

pub struct RestoreSuccess {
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
    /// [`prose_pod_api_data`]: RestoreSuccess::prose_pod_api_data
    pub tmp_dir: tempfile::TempDir,
}

fn extract_archive<R>(
    archive_reader: R,
    location: impl AsRef<Path>,
) -> Result<RestoreSuccess, anyhow::Error>
where
    R: std::io::Read,
{
    use std::ffi::OsString;

    let mut extracted_bytes: u64 = 0;

    let mut archive = tar::Archive::new(archive_reader);

    let mut entries = archive.entries()?;

    #[cfg(debug_assertions)]
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

        println!("{} {:>6} {}", type_char, size, path.display());

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

        #[cfg(debug_assertions)]
        log_extracted_entry(&entry)?;

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
                    eprintln!(
                        "Don’t know where to extract '{src}', skipping.",
                        src = entry_name.display()
                    );
                    continue;
                };

                safe_replace(entry.path(), &dst)?;
            }
            Err(err) => eprintln!("{err:?}"),
        }
    }

    if !extract_paths.is_empty() {
        return Err(anyhow!(
            "Backup invalid: Missing data ({:?}).",
            extract_paths.keys().collect::<Vec<_>>()
        ));
    }

    let api_archive = fs::File::open(tmp.path().join(archiving_config.api_archive_name))?;

    Ok(RestoreSuccess {
        restored_bytes_count: extracted_bytes,
        prose_pod_api_data: api_archive,
        tmp_dir: tmp,
    })
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct BackupInternalMetadata {
    version: u8,
}

// MARK: Errors

#[derive(Debug, thiserror::Error)]
pub enum CreateBackupError {
    #[error("Cannot create backup: '{0}' does not exist.")]
    MissingFile(std::path::PathBuf),

    #[error("Cannot create backup sink: {0:?}")]
    CannotCreateSink(anyhow::Error),

    #[error("Cannot create backup archive: {0:?}")]
    CannotArchive(anyhow::Error),

    #[error("Cannot compress backup archive: {0:?}")]
    CannotCompress(anyhow::Error),

    #[error("Backup archive compression failed: {0:?}")]
    CompressionFailed(anyhow::Error),

    #[error("Cannot encrypt backup: {0:?}")]
    CannotEncrypt(anyhow::Error),

    #[error("Backup encryption failed: {0:?}")]
    EncryptionFailed(anyhow::Error),

    #[error("Cannot compute backup integrity check: {0:?}")]
    CannotComputeIntegrityCheck(anyhow::Error),

    #[error("Failed computing backup integrity check: {0:?}")]
    IntegrityCheckGenerationFailed(anyhow::Error),

    #[error("Failed uploading backup integrity check: {0:?}")]
    IntegrityCheckUploadFailed(std::io::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum ExtractBackupError {
    #[error("Cannot create reader: {0:?}")]
    CannotCreateReader(anyhow::Error),

    #[error("Cannot decrypt backup: {0:?}")]
    CannotDecrypt(&'static str),

    #[error("Backup decryption failed: {0:?}")]
    DecryptionFailed(anyhow::Error),

    #[error("Backup decompression failed: {0:?}")]
    DecompressionFailed(std::io::Error),

    #[error("Backup extraction failed: {0:?}")]
    ExtractionFailed(anyhow::Error),
}
