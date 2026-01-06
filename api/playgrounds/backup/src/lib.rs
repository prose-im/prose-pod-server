// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

extern crate aws_sdk_s3 as s3;
extern crate sequoia_openpgp as openpgp;

mod archiving;
mod backup_repository;
mod compression;
mod encryption;
mod gpg;
mod integrity;
pub mod sink;
pub mod source;
mod util;

use crate::archiving::check_archiving_will_succeed;
use crate::util::tee_writer::TeeWriter;

use self::sink::{BackupSink, S3Sink};
use self::source::{BackupSource, S3Source};
pub use self::{
    archiving::ArchivingConfig, backup_repository::BackupRepository,
    compression::CompressionConfig, encryption::EncryptionConfig, integrity::IntegrityConfig,
};

// MARK: Service

pub type BackupService<Sink = S3Sink, Source = S3Source> = ProseBackupService<Sink, Source>;

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
pub struct ProseBackupService<Sink: BackupSink, Source: BackupSource> {
    pub archiving_config: ArchivingConfig,
    pub compression_config: CompressionConfig,
    pub encryption_config: Option<EncryptionConfig>,
    pub integrity_config: Option<IntegrityConfig>,
    pub repository: BackupRepository<Source, Sink>,
}

impl<Sink: BackupSink, Source: BackupSource> BackupService<Sink, Source> {
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
        let integrity_check_file_name = if self.integrity_config.is_some() {
            format!("{backup_file_name}.sig")
        } else {
            format!("{backup_file_name}.sha256")
        };

        let upload_backup = self
            .repository
            .backup_sink
            .backup_writer(&backup_file_name)
            .map_err(CreateBackupError::CannotCreateSink)?;

        let upload_integrity_check = self
            .repository
            .backup_sink
            .integrity_check_writer(&integrity_check_file_name)
            .map_err(CreateBackupError::CannotCreateSink)?;

        let (mut gen_integrity_check, finalize2) = builder()
            .integrity_check(self.integrity_config.as_ref())
            .build(upload_integrity_check)?;

        let (writer, finalize) = builder()
            .archive(archive, &self.archiving_config)
            .compress(&self.compression_config)
            .encrypt_if_possible(self.encryption_config.as_ref())
            .tee(&mut gen_integrity_check, finalize2)
            .build(upload_backup)?;

        let ((), finalize2) = finalize(writer)?;
        () = finalize2(gen_integrity_check)?;

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
}

// MARK: Writer

struct ProseBackupWriterBuilder<Make, Finalize> {
    make: Make,
    finalize: Finalize,
}

fn builder<W, E>() -> ProseBackupWriterBuilder<
    // NOTE: We need `W -> W` here as this layer will be
    //   the outer-most layer when building the final writer.
    impl FnOnce(W) -> Result<W, E>,
    impl FnOnce(W) -> Result<W, E>,
> {
    ProseBackupWriterBuilder {
        make: move |writer: W| Ok(writer),
        finalize: move |writer: W| Ok(writer),
    }
}

impl<M, F> ProseBackupWriterBuilder<M, F> {
    /// NOTE: Accepts a mutable reference to leave ownership to the called and
    ///   allow it to finalize the other writer manually.
    fn tee<'a, InnerWriter, InnerWriter2, OuterWriter, Out1, F2, Out2, E>(
        self,
        other_writer: &'a mut InnerWriter2,
        finalize2: F2,
    ) -> ProseBackupWriterBuilder<
        impl FnOnce(InnerWriter) -> Result<OuterWriter, E>,
        impl FnOnce(OuterWriter) -> Result<(Out1, F2), E>,
    >
    where
        M: FnOnce(TeeWriter<InnerWriter, &'a mut InnerWriter2>) -> Result<OuterWriter, E>,
        F: FnOnce(OuterWriter) -> Result<Out1, E>,
        F2: FnOnce(InnerWriter2) -> Out2,
    {
        let Self { make, finalize, .. } = self;

        ProseBackupWriterBuilder {
            make: move |writer| make(TeeWriter::new(writer, other_writer)),

            finalize: move |writer: OuterWriter| {
                let writer = finalize(writer)?;

                Ok((writer, finalize2))
            },
        }
    }

    #[must_use]
    fn build<InnerWriter, OuterWriter, Out>(
        self,
        writer: InnerWriter,
    ) -> Result<(OuterWriter, F), CreateBackupError>
    where
        InnerWriter: std::io::Write,
        M: FnOnce(InnerWriter) -> Result<OuterWriter, CreateBackupError>,
        F: FnOnce(OuterWriter) -> Out,
    {
        let Self { make, finalize, .. } = self;

        make(writer).map(move |w| (w, finalize))
    }
}
