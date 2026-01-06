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
mod writer_chain;

use crate::{
    archiving::check_archiving_will_succeed,
    stores::{ObjectStore, S3Store},
};

pub use self::{
    archiving::ArchivingConfig, compression::CompressionConfig, encryption::EncryptionConfig,
    integrity::IntegrityConfig,
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
    ///            └───┬────┘        │ Sign  | │   Hash    |
    ///                │             | (GPG) │ | (SHA 256) │
    ///                │             └───┬───┘ └─────┬─────┘
    ///                │                 ◇───────────┘
    ///                │        ┌────────┴─────────┐
    ///                │        | Upload integrity |
    ///                │        |    check (S3)    |
    ///                │        └────────┬─────────┘
    ///              ╺━┷━━━━━━━━┯━━━━━━━━┷━╸
    ///                         ◉
    /// ```
    pub fn create_backup(
        &self,
        backup_name: &str,
        archive: tar::Archive<std::io::Cursor<bytes::Bytes>>,
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
            .map_err(CreateBackupError::CannotCreateSink)?;

        let mut integrity_check: Vec<u8> = Vec::new();

        let (mut gen_integrity_check, finalize2) = writer_chain::builder()
            .integrity_check(self.integrity_config.as_ref())
            .build(&mut integrity_check)?;

        let (writer, finalize) = writer_chain::builder()
            .archive(archive, &self.archiving_config)
            .compress(&self.compression_config)
            .encrypt_if_possible(self.encryption_config.as_ref())
            .tee(&mut gen_integrity_check)
            .build(upload_backup)?;

        () = finalize(writer)?;
        () = finalize2(gen_integrity_check)?;

        let integrity_check_file_name = if self.integrity_config.is_some() {
            format!("{backup_file_name}.sig")
        } else {
            format!("{backup_file_name}.sha256")
        };

        let mut upload_integrity_check = self
            .integrity_check_store
            .writer(&integrity_check_file_name)
            .map_err(CreateBackupError::CannotCreateSink)?;

        let mut cursor = std::io::Cursor::new(integrity_check);
        std::io::copy(&mut cursor, &mut upload_integrity_check)
            .map_err(CreateBackupError::IntegrityCheckUploadFailed)?;

        Ok((backup_file_name, integrity_check_file_name))
    }
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
}
