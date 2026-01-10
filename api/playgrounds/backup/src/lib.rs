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
mod stats;
pub mod stores;
mod util;
mod writer_chain;

use std::path::Path;

use anyhow::{Context, anyhow};
use bytes::Bytes;
use time::format_description::well_known::Rfc3339;

use crate::{
    archiving::{ExtractionSuccess, check_archiving_will_succeed, extract_archive},
    stats::{StatsReader, print_stats},
    stores::{ObjectStore, S3Store},
    util::hash,
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

// MARK: Create backup

impl<S1: ObjectStore, S2: ObjectStore> ProseBackupService<S1, S2> {
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

        let now = time::UtcDateTime::now();
        let backup_name = format!(
            "{backup_name}_{now}",
            now = now
                .replace_millisecond(0)
                .unwrap()
                .format(&Rfc3339)
                .unwrap()
        );

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
}

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

// MARK: List

#[derive(Debug)]
pub struct BackupDto {
    /// Instead of this being the unique name of the backup, it’s
    pub id: u64,

    pub metadata: BackupMetadataPartialDto,

    /// To prevent accidental restoration of corrupted or untrusted backups,
    /// a unique identifier used for restoration is exposed here. It is the
    /// unique backup name out of simplicity, but this should be considered
    /// **an implementation detail**.
    ///
    /// It wouldn’t be possible to restore corrupted or untrusted backups
    /// anyway, but at least having an optional ID here makes it explicit
    /// for UI implementators. They can then hide or grey out the “Restore”
    /// button when this action is impossible. Note that a “tooltip” or info
    /// message with the reason why the action is impossible can be created
    /// by inspecting [`metadata`].
    ///
    /// [`metadata`]: BackupDto::metadata
    pub restore_id: Option<String>,
}

/// ⚠️ Beware that [`can_be_restored`] is a one-sided indicator. `true` doesn’t
/// mean a restore will succeed for sure. More computation is required to know
/// if the backup is intact and using trusted keys for example. `false`,
/// however, means restoring this backup is impossible and the option shouldn’t
/// be presented to a user.
///
/// [`can_be_restored`]: BackupMetadataPartialDto::can_be_restored
#[derive(Debug)]
pub struct BackupMetadataPartialDto {
    pub name: Box<str>,
    pub created_at: time::UtcDateTime,
    pub is_signed: bool,
    pub is_encrypted: bool,
    pub can_be_restored: bool,
}

/// [`BackupMetadataPartialDto`] with additional data that requires expensive
/// computation.
///
/// ⚠️ Beware that [`BackupMetadataFullDto::can_be_restored`] might differ from
/// [`BackupMetadataPartialDto::can_be_restored`] as the latter doesn’t know if
/// the backup is intact or using trusted keys.
#[derive(Debug)]
pub struct BackupMetadataFullDto {
    pub name: Box<str>,
    pub created_at: time::UtcDateTime,
    pub is_intact: bool,
    pub is_signed: bool,
    pub is_signature_trusted: bool,
    pub is_signature_valid: bool,
    pub is_encrypted: bool,
    pub is_encryption_valid: bool,
    pub is_trusted: bool,
    pub can_be_restored: bool,
}

impl<S1: ObjectStore, S2: ObjectStore> ProseBackupService<S1, S2> {
    pub async fn list_backups(&self) -> Result<Vec<BackupDto>, anyhow::Error> {
        // NOTE: S3 lists objects in alphabetically ascending order and has
        //   no way to list in reverse order or ist by “last modified” date
        //   (even ascending). Therefore, we have no choice but to list ALL
        //   backups by name. It’s acceptable because backups will likely be
        //   deleted every once in a while which means we won’t end up with a
        //   _very_ large number. Integrity checks should never be deleted
        //   therefore it might grow bigger, but by using `StartAfter` we can
        //   limit the number of results to roughly the number of backups still
        //   stored. We might get a large amount of results if backups are
        //   created daily and deleted using tiered retention, but even then it
        //   would still take years to reach the 1000 objects per request limit
        //   causing a second page fetch.

        let backups = self.backup_store.list_all().await?;

        // NOTE: S3 results are sorted in alphabetically ascending order,
        //   and backup names use RFC 3339 timestamps which are alphabetically
        //   sortable. The first result is the oldest backup.
        let Some(oldest_backup) = backups.first() else {
            return Ok(vec![]);
        };

        let checks = self
            .integrity_check_store
            .list_all_after(oldest_backup)
            .await?;

        let todo = "Make this dynamic via configuration.";
        let signing_is_mandatory = false;
        let encryption_is_mandatory = false;

        let mut dtos: Vec<BackupDto> = Vec::with_capacity(backups.len());

        for backup_name in backups.into_iter() {
            let is_signed = checks.contains(&format!("{backup_name}.sig"));
            let is_encrypted = backup_name.ends_with(".gpg");

            let can_be_restored = true
                && (!signing_is_mandatory || is_signed)
                && (!encryption_is_mandatory || is_encrypted);

            let created_at = match parse_timestamp(&backup_name) {
                Ok(timestamp) => timestamp,
                Err(err) => {
                    tracing::warn!("Skipping `{backup_name}`: {err:?}");
                    continue;
                }
            };

            dtos.push(BackupDto {
                id: hash(&backup_name),
                metadata: BackupMetadataPartialDto {
                    name: backup_name.clone().into_boxed_str(),
                    created_at,
                    is_signed,
                    is_encrypted,
                    can_be_restored,
                },
                restore_id: if can_be_restored {
                    Some(backup_name)
                } else {
                    None
                },
            });
        }

        Ok(dtos)
    }
}

fn parse_timestamp(backup_name: &str) -> Result<time::UtcDateTime, anyhow::Error> {
    let Some(basename) = backup_name.split(".").next() else {
        anyhow::bail!("File has no extension.");
    };
    if basename.is_empty() {
        anyhow::bail!("File has no basename.");
    };
    let Some(suffix) = basename.split("_").last() else {
        anyhow::bail!("File `{basename}` is missing the timestamp suffix.");
    };
    match time::UtcDateTime::parse(suffix, &Rfc3339) {
        Ok(timestamp) => Ok(timestamp),
        Err(err) => {
            Err(anyhow::Error::from(err).context("Could not parse timestamp from file name"))
        }
    }
}

// MARK: Restore

impl<S1: ObjectStore, S2: ObjectStore> ProseBackupService<S1, S2> {
    #[must_use]
    pub async fn restore_backup(
        &self,
        backup_name: &str,
        location: impl AsRef<Path>,
    ) -> Result<ExtractionSuccess, anyhow::Error> {
        use crate::stats::ReadStats;
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
                .context("Cannot create reader")?;

            let mut raw_read_stats = ReadStats::new();
            let backup_reader = StatsReader::new(backup_reader, &mut raw_read_stats);

            let mut decryption_stats = ReadStats::new();
            let compressed_archive_reader = if backup_name.ends_with(".gpg") {
                if let Some(config) = self.encryption_config.as_ref() {
                    let decryptor = DecryptorBuilder::from_reader(backup_reader)
                        .context("Failed creating decryptor builder")?
                        .with_policy(config.policy.as_ref(), None, config)
                        .context("Failed creating decryptor")?;

                    let decryptor = StatsReader::new(decryptor, &mut decryption_stats);

                    Either::A(decryptor)
                } else {
                    return Err(anyhow!(
                        "Encryption not configured. Cannot find private keys.",
                    ));
                }
            } else {
                tracing::debug!("NOT DECRYPTING");

                Either::B(backup_reader)
            };

            let archive_bytes =
                zstd::Decoder::new(compressed_archive_reader).context("Cannot decompress")?;

            let mut decompression_stats = ReadStats::new();
            let archive_bytes = StatsReader::new(archive_bytes, &mut decompression_stats);

            let restore_result =
                extract_archive(archive_bytes, location).context("Backup extraction failed")?;

            print!("\n");
            print_stats(
                &raw_read_stats,
                &decryption_stats,
                &decompression_stats,
                restore_result.restored_bytes_count,
            );

            return Ok(restore_result);
        }

        // FIXME: Do not look for hash if signature is mandatory.

        // Check hash if no signature exist.
        let todo = "Hash check";

        Ok(todo!())
    }
}
