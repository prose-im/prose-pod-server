// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! Create a [`BackupService`], then use it to create, read and delete backups.
//!
//! ```no_run
//! use prose_backup::{BackupService, BackupConfig};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), anyhow::Error> {
//!   let toml: toml::Table = unimplemented!();
//!   let backup_config = BackupConfig::try_from(toml)?;
//!
//!   let blueprints = unimplemented!();
//!   let migrations = unimplemented!();
//!   let service = BackupService::from_config(&backup_config, blueprints, migrations)?;
//!
//!   let _backups = service.list_backups().await?;
//! }
//! ```

#[cfg(all(not(feature = "hashing-blake3"), not(feature = "hashing-sha2")))]
compile_error!("One of feature “hashing-blake3” or “hashing-sha2” must be enabled.");

pub mod archiving;
mod compression;
pub mod config;
pub mod decryption;
pub mod encryption;
pub mod event_handlers;
mod hashing;
mod pgp;
pub mod restoration;
pub mod signing;
pub mod stats;
pub mod stores;
mod util;
pub mod verification;

use std::collections::HashMap;
use std::sync::Arc;

pub use openpgp;
pub use tokio;
pub use toml;

pub use self::backup_id::*;
pub use self::config::BackupConfig;
pub use self::create::*;
pub use self::restore::*;

// MARK: Service

/// Backup service. Central component of the library.
pub struct BackupService {
    pub archiving_context: archiving::Context,
    pub compression_config: config::CompressionConfig,
    pub hashing_config: config::HashingConfig,
    pub encryption_context: Option<encryption::Context>,
    pub signing_context: signing::Context,
    pub verification_context: verification::Context,
    pub decryption_context: decryption::Context,
    pub restoration_context: restoration::Context,
    pub download_config: config::DownloadConfig,

    pub backup_store: stores::CachedStore<Box<dyn stores::ObjectStore>>,
    pub check_store: Box<dyn stores::ObjectStore>,
}
crate::util::assert_impl!(BackupService: Send);
crate::util::assert_impl!(BackupService: Sync);

impl BackupService {
    /// Create a new backup and upload it to the store.
    ///
    /// Everything is done in a single stream of the following shape:
    ///
    /// ```text
    ///                         ┌─/var/lib/prosody
    ///                         ├─/etc/prosody
    ///                         ├─/etc/prose
    ///                         ├─/var/lib/prose-pod-server
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
    ///                         │ PGP encryption enabled?
    ///                         ◇──────┐
    ///                     Yes │      │ No
    ///                    ┌────┴────┐ │
    ///                    │ Encrypt │ │
    ///                    │  (PGP)  │ │
    ///                    └────┬────┘ │
    ///                         ◇──────┘
    ///      ╺━┯━━━━━━━━━━━━━━━━┿━━━━━━━━━━━━━━━━━━┯━╸
    ///    ┌───┴────┐      ┌────┴─────┐            │ PGP signing
    ///    │ Upload │      │   Hash   │            │ enabled?
    ///    │ backup │      │ (BLAKE3) │            ◇───────┐
    ///    │  (S3)  │      └────┬─────┘        Yes │       │ No
    ///    └───┬────┘  ┌────────┴─────────┐    ┌───┴───┐   ◯
    ///        ◯       │ Upload integrity │    │ Sign  │
    ///                │    check (S3)    │    │ (PGP) │
    ///                └────────┬─────────┘    └───┬───┘
    ///                         ◯            ┌─────┴─────┐
    ///                                      │  Upload   │
    ///                                      │ signature │
    ///                                      │   (S3)    │
    ///                                      └─────┬─────┘
    ///                                            ◯
    /// ```
    #[inline]
    pub async fn create_backup<D: archiving::AdditionalData>(
        &self,
        command: create::CreateBackupCommand<'_, D>,
        event_handler: &mut impl CreateBackupEventHandler,
    ) -> Result<create::CreateBackupSuccess, create::CreateBackupError> {
        crate::create::create_backup(self, command, event_handler).await
    }

    /// List all backups, in alphabetically descending order.
    #[inline]
    pub async fn list_backups(
        &self,
    ) -> Result<Vec<BackupDto<BackupMetadataPartialDto>>, anyhow::Error> {
        crate::read::list_backups(self).await
    }

    #[inline]
    pub async fn get_details(
        &self,
        backup_id: &BackupId,
    ) -> Result<BackupDto<BackupMetadataFullDto>, anyhow::Error> {
        crate::read::get_details(self, backup_id).await
    }

    /// Get a short-lived URL to download a backup.
    #[inline]
    pub async fn get_download_url(
        &self,
        backup_id: &BackupId,
        ttl: std::time::Duration,
    ) -> Result<String, anyhow::Error> {
        crate::read::get_download_url(self, backup_id, ttl).await
    }

    #[inline]
    pub async fn restore_backup<EventHandler>(
        &self,
        backup_id: &BackupId,
        blueprint: &archiving::ArchiveBlueprint,
        event_handler: &mut EventHandler,
    ) -> Result<ExtractAndRestoreSuccess, restoration::RestorationError>
    where
        EventHandler: ExtractBackupEventHandler + RestoreBackupEventHandler,
    {
        crate::restore::restore_backup(self, backup_id, blueprint, event_handler).await
    }

    #[inline]
    pub async fn extract_backup<'a>(
        &'a self,
        backup_id: &BackupId,
        event_handler: &mut impl ExtractBackupEventHandler,
    ) -> Result<ExtractionSuccess<'a>, archiving::ExtractionError> {
        crate::restore::extract_backup(self, backup_id, event_handler).await
    }

    #[inline]
    pub async fn restore_extracted_backup<'a>(
        &self,
        backup_id: &BackupId,
        extraction_output: archiving::ExtractionOutput<'a>,
        blueprint: &archiving::ArchiveBlueprint,
        event_handler: &mut impl RestoreBackupEventHandler,
    ) -> Result<restoration::RestorationOutput, restoration::RestorationError> {
        crate::restore::restore_extracted_backup(
            backup_id,
            extraction_output,
            blueprint,
            &self.restoration_context,
            event_handler,
        )
        .await
    }

    #[inline]
    pub async fn delete_backup(&self, backup_id: &BackupId) -> Result<(), anyhow::Error> {
        crate::delete::delete_backup(self, backup_id).await
    }
}

impl BackupService {
    pub fn from_config(
        config: &BackupConfig,
        blueprints: HashMap<u8, archiving::ArchiveBlueprint>,
        migrations: Vec<restoration::ArchiveMigration>,
    ) -> Result<Self, anyhow::Error> {
        // NOTE: This gets inlined in release builds.
        Self::from_config_custom(
            config,
            archiving::Context { blueprints },
            restoration::Context { migrations },
            |path| {
                use openpgp::parse::Parse as _;
                openpgp::Cert::from_file(path)
            },
            openpgp::policy::StandardPolicy::new,
        )
    }

    /// This is what [`ProseBackupService::from_config`] calls internally.
    ///
    /// You should not have to use it. It’s only made public because it’s used
    /// in integration tests.
    #[doc(hidden)]
    #[cfg_attr(not(feature = "test"), inline(always))]
    pub fn from_config_custom<P>(
        config: &BackupConfig,
        archiving_context: archiving::Context,
        restoration_context: restoration::Context,
        get_pgp_cert: impl Fn(&std::path::PathBuf) -> Result<openpgp::Cert, anyhow::Error>,
        pgp_policy: impl Fn() -> P,
    ) -> Result<Self, anyhow::Error>
    where
        P: openpgp::policy::Policy + 'static,
    {
        use crate::decryption::PgpDecryptionContext;
        use crate::signing::PgpSigningContext;
        use crate::stores::*;
        use crate::verification::PgpVerificationContext;

        let encryption_context = match &config.encryption {
            config::EncryptionConfig::Off => None,
            config::EncryptionConfig::Pgp { config: pgp } => {
                let mut recipients = Vec::with_capacity(pgp.additional_recipients.len() + 1);

                recipients.push(get_pgp_cert(&pgp.tsk)?);

                for path in pgp.additional_recipients.iter() {
                    recipients.push(get_pgp_cert(path)?);
                }

                Some(encryption::Context::Pgp {
                    recipients,
                    policy: Box::new(pgp_policy()),
                })
            }
        };

        let pgp_signing_context = match config.signing.pgp.as_ref() {
            Some(pgp) => {
                let pgp_cert = get_pgp_cert(&pgp.tsk)?;
                Some(PgpSigningContext {
                    tsk: pgp_cert,
                    policy: Box::new(pgp_policy()),
                    passphrases: pgp.passphrases.clone(),
                })
            }
            None => None,
        };
        let signing_context = signing::Context {
            is_signing_mandatory: config.signing.mandatory,
            pgp: pgp_signing_context,
        };

        let pgp_verification_context = match config.signing.pgp.as_ref() {
            Some(pgp) => {
                let pgp_cert = get_pgp_cert(&pgp.tsk)?;
                Some(PgpVerificationContext {
                    certs: Arc::new(vec![pgp_cert]),
                    policy: Box::new(pgp_policy()),
                })
            }
            None => None,
        };
        let verification_context = verification::Context {
            pgp: pgp_verification_context,
        };

        let mut decryption_context = decryption::Context::default();
        if let config::EncryptionConfig::Pgp { config: pgp } = &config.encryption {
            let pgp_cert = get_pgp_cert(&pgp.tsk)?;
            decryption_context.pgp = Some(PgpDecryptionContext {
                tsks: vec![pgp_cert],
                policy: Box::new(pgp_policy()),
                passphrases: pgp.passphrases.clone(),
            });
        }

        let backup_store: Box<dyn ObjectStore> = match config.storage.backups {
            #[cfg(feature = "storage-s3")]
            config::StorageSubconfig::S3 { ref config } => Box::new(S3Store::from_config(config)),
            #[cfg(feature = "storage-fs")]
            config::StorageSubconfig::Fs { ref config } => {
                Box::new(FsStore::try_from_config(config, 0o600)?)
            }
        };
        let check_store: Box<dyn ObjectStore> = match config.storage.checks {
            #[cfg(feature = "storage-s3")]
            config::StorageSubconfig::S3 { ref config } => Box::new(S3Store::from_config(config)),
            #[cfg(feature = "storage-fs")]
            config::StorageSubconfig::Fs { ref config } => {
                Box::new(FsStore::try_from_config(config, 0o600)?)
            }
        };

        Ok(Self {
            archiving_context,
            compression_config: config.compression.to_owned(),
            hashing_config: config.hashing.to_owned(),
            encryption_context,
            signing_context,
            verification_context,
            decryption_context,
            restoration_context,
            backup_store: stores::CachedStore::new(backup_store, Arc::default(), &config.caching),
            check_store,
            download_config: config.download.to_owned(),
        })
    }
}

// MARK: DTOs

use self::dtos::*;
pub mod dtos {
    //! [Data Transfer Objects].
    //!
    //! [Data Transfer Objects]: https://en.wikipedia.org/wiki/Data_transfer_object "“Data transfer object” on Wikipedia"

    use crate::{BackupId, verification::PgpSignatureReport};

    #[derive(Debug)]
    pub struct BackupDto<Metadata> {
        /// Unique identifier (file name / object key) of the backup.
        ///
        /// E.g. `prose%2Dbackup-1772432392-Automatic%20backup.tar.zst.pgp`.
        pub id: BackupId,

        /// Description of the backup.
        ///
        /// E.g. “Automatic backup”.
        pub description: String,

        /// Metadata associated with the backup.
        pub metadata: Metadata,
    }

    #[derive(Debug)]
    pub struct BackupMetadataPartialDto {
        /// UTC timestamp at which the backup was created.
        pub created_at: time::UtcDateTime,

        /// Size of the backup, in bytes.
        pub size_bytes: u64,

        /// Whether or not the backup was signed.
        ///
        /// This doesn’t mean anything regarding whether or not the signature
        /// was issued by a trusted entity nor if it’s valid. Such information
        /// is only present in [`BackupMetadataFullDto`].
        pub is_signed: bool,

        /// Whether or not the backup is encrypted.
        pub is_encrypted: bool,

        /// An indicator potentially indicating a backup cannot be restored
        /// (e.g. it’s not signed but signing is mandatory).
        ///
        /// ⚠️ Beware that [`can_be_restored`] is a one-sided indicator. `true`
        /// doesn’t mean a restore will succeed for sure. More computation is
        /// required to know if the backup is intact and using trusted keys for
        /// example. `false`, however, means restoring this backup is impossible
        /// and the option shouldn’t be presented to a user.
        ///
        /// [`can_be_restored`]: BackupMetadataPartialDto::can_be_restored
        pub can_be_restored: bool,
    }

    /// [`BackupMetadataPartialDto`] with additional data that requires
    /// expensive computation.
    #[derive(Debug)]
    pub struct BackupMetadataFullDto {
        /// UTC timestamp at which the backup was created.
        pub created_at: time::UtcDateTime,

        /// Size of the backup, in bytes.
        pub size_bytes: u64,

        /// Whether or not the backup is intact (not corrupted).
        ///
        /// If it’s not intact, you cannot restore it.
        pub is_intact: bool,

        /// Whether or not the backup was signed.
        ///
        /// This doesn’t mean anything regarding whether or not the signature
        /// was issued by a trusted entity nor if it’s valid. Such information
        /// is in [`signing_keys`], then [`is_trusted`] and [`is_valid`].
        ///
        /// [`signing_keys`]: Self::signing_keys
        /// [`is_trusted`]: SigningKeyReportDto::is_trusted
        /// [`is_valid`]: SigningKeyReportDto::is_valid
        pub is_signed: bool,

        /// Fingerprint of the key(s) used to sign the backup, if applicable.
        ///
        /// Note that this list might be empty while [`is_signed`] is `true`.
        /// It means the backup was signed but with an unknown key (lost or
        /// malicious). In that case, [`can_be_restored`] might still be `true`
        /// (e.g. if the integrity of the backup was verified successfully).
        ///
        /// [`is_signed`]: Self::signing_keys
        /// [`can_be_restored`]: Self::can_be_restored
        pub known_signing_keys: Vec<SigningKeyReportDto>,

        /// Whether or not the backup is encrypted.
        pub is_encrypted: bool,

        /// Fingerprint of the key used to encrypt the backup, if applicable.
        pub encryption_key: Option<String>,

        /// Whether or not the backup can be successfully decrypted
        /// (i.e. encrypted with an known private key).
        pub is_encryption_valid: Option<bool>,

        /// Whether or not the backup can be restored (e.g. `false` if its
        /// signature is invalid).
        ///
        /// ⚠️ Beware that [`BackupMetadataFullDto::can_be_restored`] might
        /// differ from [`BackupMetadataPartialDto::can_be_restored`] as the
        /// latter doesn’t know if the backup is intact or using trusted keys.
        pub can_be_restored: bool,
    }

    /// Information about a key used to sign a backup.
    #[derive(Debug)]
    pub struct SigningKeyReportDto {
        /// Unique fingerprint of the signing key.
        ///
        /// Note that for OpenPGP signatures, this is the certificate’s primary
        /// key fingerprint.
        pub fingerprint: String,

        /// Whether or not the backup signature is valid.
        pub is_valid: bool,
    }

    impl From<PgpSignatureReport> for SigningKeyReportDto {
        fn from(value: PgpSignatureReport) -> Self {
            Self {
                fingerprint: value.cert_fingerprint.to_spaced_hex(),
                is_valid: value.is_valid,
            }
        }
    }
}

mod create {
    use anyhow::Context as _;
    use composable_stream::*;

    use crate::BackupService;
    use crate::archiving::{self, *};
    use crate::backup_id::*;
    use crate::compression::*;
    use crate::config::CompressionConfig;
    use crate::dtos::*;
    use crate::encryption::*;
    use crate::hashing::*;
    use crate::signing::pgp::*;
    use crate::stats::*;
    use crate::stores::*;

    pub(crate) async fn create_backup<D: archiving::AdditionalData>(
        service: &BackupService,
        CreateBackupCommand {
            prefix,
            description,
            blueprint,
            additional_archive_data,
            #[cfg(feature = "test")]
            created_at,
        }: CreateBackupCommand<'_, D>,
        event_handler: &mut impl CreateBackupEventHandler,
    ) -> Result<CreateBackupSuccess, CreateBackupError> {
        let expected_archive_size =
            check_archiving_will_succeed(&blueprint, &additional_archive_data)?;

        #[cfg(not(feature = "test"))]
        let created_at = std::time::SystemTime::now();

        let mut extensions: Vec<Box<str>> = vec![Box::from("tar")];
        match &service.compression_config {
            #[cfg(feature = "compression-zstd")]
            CompressionConfig::Zstd { .. } => extensions.push(Box::from("zst")),
            CompressionConfig::Off => {}
        }
        match &service.encryption_context {
            Some(EncryptionContext::Pgp { .. }) => extensions.push(Box::from("pgp")),
            None => {}
        };

        let backup_id = BackupId {
            prefix: Box::from(prefix),
            created_at: created_at.into(),
            description: Box::from(description),
            extensions,
        };
        let raw_backup_id = ObjectId::from(&backup_id);

        event_handler.on_archive_start(&backup_id, expected_archive_size);

        // Try to open sink first, to abort early if something is wrong.
        let upload_backup = service
            .backup_store
            .writer(&raw_backup_id)
            .await
            .map_err(CreateBackupError::CannotCreateSink)?;

        let start = std::time::Instant::now();

        let archive_writer = archive(&blueprint, additional_archive_data)
            .then(meter_writes(BackupStatsReader {
                backup_id: &backup_id,
                event_handler,
            }))
            .then(compress(&service.compression_config))
            .then(eventually(service.encryption_context.as_ref(), |ctx| {
                encrypt(ctx, created_at)
            }))
            // Record stats so we can know the final size of the backup.
            .then(meter_writes(WriteStats::new()))
            .tee_into(digest(&service.hashing_config))
            .opt_tee(
                service.signing_context.pgp.as_ref(),
                |ctx| pgp_sign(ctx, created_at),
                Vec::<u8>::new(),
            )
            .build(upload_backup)?;

        let delete_guard = BackupAutoDeleteGuard::new(service, &backup_id);

        let compression_writer = archive_writer
            // NOTE: Flushes the stream if needed.
            .into_inner()
            .context("Could not init archive")
            .map_err(CreateBackupError::ArchivingFailed)?;

        let compression_writer = compression_writer.into_inner();

        let encryption_writer_opt = compression_writer
            .finalize()
            .map_err(CreateBackupError::CompressionFailed)?;

        let (Tee(Tee(backup_upload, pgp_signing_writer_opt), digest_writer), backup_stats) =
            match encryption_writer_opt {
                Either::A(encryption_writer) => encryption_writer
                    .into_inner()
                    .map_err(CreateBackupError::EncryptionFailed)?,
                Either::B(writer) => writer,
            }
            .into_parts();

        let digest = digest_writer.finalize();

        let mut digest_ids: Vec<ObjectId> = Vec::new();
        let mut checks_upload_durations: Vec<(ObjectId, std::time::Duration)> = Vec::new();

        // Upload digest.
        let digest_id = match service.hashing_config.algorithm {
            #[cfg(feature = "hashing-blake3")]
            crate::config::HashingAlgorithm::Blake3 => raw_backup_id.with_extension("blake3"),
            #[cfg(feature = "hashing-sha2")]
            crate::config::HashingAlgorithm::Sha256 => raw_backup_id.with_extension("sha256"),
        };
        upload_integrity_check(
            digest,
            digest_id,
            &service,
            &mut checks_upload_durations,
            &mut digest_ids,
        )
        .await?;

        let mut signature_ids: Vec<ObjectId> = Vec::new();

        let is_signed = pgp_signing_writer_opt.is_some();

        // Upload OpenPGP signature.
        if let OptionalStream::Some(writer) = pgp_signing_writer_opt {
            let pgp_signature = writer
                .finalize()
                .map_err(CreateBackupError::SigningFailed)?;

            upload_integrity_check(
                pgp_signature,
                // NOTE: OpenPGP will likely forever be the only signing protocol
                //   we support, but if we ever add one that also uses the `.sig`
                //   extension we can just use `.<protocol>.sig` for it.
                raw_backup_id.with_extension("sig"),
                &service,
                &mut checks_upload_durations,
                &mut signature_ids,
            )
            .await?;
        }

        // Finish uploading backup.
        () = backup_upload
            .finalize()
            .map_err(CreateBackupError::UploadFailed)?;
        let size_bytes = backup_stats.bytes_written;
        let elapsed = start.elapsed();
        tracing::info!("Created backup {backup_id:?} ({size_bytes}B) in {elapsed:?}.");
        event_handler.on_backup_uploaded(&backup_id, size_bytes, elapsed);

        delete_guard.defuse();

        // Construct the response.
        Ok(CreateBackupSuccess {
            backup: BackupDto {
                id: backup_id.clone(),
                description: description.to_owned(),
                metadata: BackupMetadataPartialDto {
                    created_at: created_at.into(),
                    size_bytes,
                    is_signed,
                    is_encrypted: service.encryption_context.is_some(),
                    can_be_restored: true,
                },
            },
            output: CreateBackupOutput {
                backup_id,
                digest_ids,
                signature_ids,
            },
        })
    }

    async fn upload_integrity_check(
        data: Vec<u8>,
        check_id: ObjectId,
        service: &BackupService,
        checks_upload_durations: &mut Vec<(ObjectId, std::time::Duration)>,
        uploaded: &mut Vec<ObjectId>,
    ) -> Result<(), CreateBackupError> {
        let start = std::time::Instant::now();

        let mut uploader = service
            .check_store
            .writer(&check_id)
            .await
            .map_err(CreateBackupError::CannotCreateSink)?;

        let mut cursor = std::io::Cursor::new(data);
        std::io::copy(&mut cursor, &mut uploader)
            .context("`std::io::copy` failed")
            .map_err(CreateBackupError::IntegrityCheckUploadFailed)?;

        uploader
            .finalize()
            .context("`finalize` failed")
            .map_err(CreateBackupError::IntegrityCheckUploadFailed)?;

        checks_upload_durations.push((check_id.clone(), start.elapsed()));
        uploaded.push(check_id);

        Ok(())
    }

    pub struct CreateBackupCommand<'a, D: archiving::AdditionalData = ()> {
        /// Desired backup prefix (e.g. “prose-backup”).
        pub prefix: &'a str,

        /// Desired backup description (e.g. “Automatic backup”).
        pub description: &'a str,

        pub blueprint: &'a ArchiveBlueprint,

        /// Some more data to insert in the archive before it’s built.
        pub additional_archive_data: Option<D>,

        /// Timestamp which should be associated with the backup.
        ///
        /// This is only useful in tests, as we have no way to read data as it was
        /// at the previous date. It’s only metadata.
        #[cfg(feature = "test")]
        pub created_at: std::time::SystemTime,
    }

    #[allow(unused_variables)]
    pub trait CreateBackupEventHandler: Send + Sync {
        #[inline]
        fn on_archive_start(&mut self, backup_id: &BackupId, expected_archive_size: u64) {}

        #[inline]
        fn on_archive_progress(&mut self, backup_id: &BackupId, archived_bytes: usize) {}

        #[inline]
        fn on_upload_progress(&mut self, object_id: &ObjectId, uploaded_bytes: usize) {}

        #[inline]
        fn on_backup_uploaded(
            &mut self,
            backup_id: &BackupId,
            size_bytes: u64,
            duration: std::time::Duration,
        ) {
        }

        #[inline]
        fn on_digest_uploaded(&mut self, object_id: &ObjectId, duration: std::time::Duration) {}

        #[inline]
        fn on_signature_uploaded(&mut self, object_id: &ObjectId, duration: std::time::Duration) {}
    }

    struct BackupStatsReader<'a, H: CreateBackupEventHandler> {
        backup_id: &'a BackupId,
        event_handler: &'a mut H,
    }

    impl<'a, H: CreateBackupEventHandler> StreamStats for BackupStatsReader<'a, H> {
        fn record_chunk(&mut self, len: usize) {
            self.event_handler.on_archive_progress(self.backup_id, len);
        }
    }

    impl<'a, H: CreateBackupEventHandler> WriterStats for BackupStatsReader<'a, H> {}

    #[derive(Debug)]
    pub struct CreateBackupOutput {
        /// Unique identifier (file name / object key) of the backup.
        ///
        /// E.g. `prose%2Dbackup-1772432392-Automatic%20backup.tar.zst.pgp`.
        pub backup_id: BackupId,

        /// Unique identifiers (file names / object keys) of backup digests
        /// (cryptographic checksums).
        ///
        /// E.g. `prose%2Dbackup-1772432392-Automatic%20backup.tar.zst.pgp.sha256`.
        pub digest_ids: Vec<ObjectId>,

        /// Unique identifiers (file names / object keys) of backup signatures.
        ///
        /// E.g. `prose%2Dbackup-1772432392-Automatic%20backup.tar.zst.pgp.sig`.
        pub signature_ids: Vec<ObjectId>,
    }

    #[derive(Debug)]
    pub struct CreateBackupSuccess {
        pub backup: BackupDto<BackupMetadataPartialDto>,
        pub output: CreateBackupOutput,
    }

    #[derive(Debug, thiserror::Error)]
    pub enum CreateBackupError {
        #[error("Cannot create backup sink")]
        CannotCreateSink(#[source] anyhow::Error),

        #[error("Cannot archive")]
        CannotArchive(#[from] archiving::errors::CannotArchive),

        #[error("Archiving failed")]
        ArchivingFailed(#[source] anyhow::Error),

        #[error("Cannot compress")]
        CannotCompress(#[source] anyhow::Error),

        #[error("Compression failed")]
        CompressionFailed(#[source] anyhow::Error),

        #[error("Cannot encrypt")]
        CannotEncrypt(#[source] anyhow::Error),

        #[error("Encryption failed")]
        EncryptionFailed(#[source] anyhow::Error),

        #[error("Backup hashing failed")]
        HashingFailed(#[source] anyhow::Error),

        #[error("Cannot sign")]
        CannotSign(#[source] anyhow::Error),

        #[error("Signing failed")]
        SigningFailed(#[source] anyhow::Error),

        #[error("Failed uploading backup")]
        UploadFailed(#[source] anyhow::Error),

        #[error("Failed uploading backup integrity check")]
        IntegrityCheckUploadFailed(#[source] anyhow::Error),

        #[error(transparent)]
        Other(anyhow::Error),
    }

    struct BackupAutoDeleteGuard<'a> {
        service: &'a BackupService,
        // NOTE: It’d be nice to take ownership to force defusing the guard to
        //   get back ownership and create the `CreateBackupOutput` but:
        //   1. Without using a separate module, one could still access this
        //      field without using the defuser;
        //   2. It would be a bit annoying to handle lifetimes in the current
        //      implementation of `create_backup`
        //   3. The compiler warns if the guard is unused, ensuring we don’t
        //      forget about defusing it.
        //   Since this code is internal, let’s just not care about it.
        backup_id: Option<&'a BackupId>,
    }

    impl<'a> BackupAutoDeleteGuard<'a> {
        fn new(service: &'a BackupService, backup_id: &'a BackupId) -> Self {
            Self {
                service,
                backup_id: Some(backup_id),
            }
        }

        fn defuse(mut self) {
            std::mem::take(&mut self.backup_id);
        }
    }

    impl<'a> Drop for BackupAutoDeleteGuard<'a> {
        fn drop(&mut self) {
            let Some(backup_id) = std::mem::take(&mut self.backup_id) else {
                return;
            };

            tokio::task::block_in_place(move || {
                tokio::runtime::Handle::current().block_on(async move {
                    match self.service.delete_backup(backup_id).await {
                        Ok(_) => tracing::info!("Cleaned up backup `{backup_id}`."),
                        Err(err) => {
                            tracing::error!("Failed cleaning up backup `{backup_id}`: {err:#}")
                        }
                    }
                })
            })
        }
    }
}

mod read {
    use crate::BackupService;
    use crate::backup_id::*;
    use crate::decryption::*;
    use crate::dtos::*;
    use crate::stores::*;

    pub(crate) async fn list_backups(
        service: &BackupService,
    ) -> Result<Vec<BackupDto<BackupMetadataPartialDto>>, anyhow::Error> {
        // NOTE: S3 lists objects in alphabetically ascending order and has
        //   no way to list in reverse order or list by “last modified” date
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

        use std::str::FromStr as _;

        /// Determines whether an object is a backup based on its name.
        /// It’s not bulletproof and might break if we make changes to
        /// compression or encryption but it’s good enough for now.
        fn is_backup(metadata: &ObjectMetadata) -> bool {
            match metadata.file_name.rsplit(".").next() {
                Some(file_ext) => {
                    for ext in [
                        #[cfg(feature = "hashing-sha2")]
                        "sha256",
                        #[cfg(feature = "hashing-blake3")]
                        "blake3",
                        "sig",
                    ] {
                        if file_ext == ext {
                            return false;
                        }
                    }
                    true
                }
                None => false,
            }
        }

        // NOTE: If using the same bucket and prefix for both the backups and
        //   integrity checks, `service.backup_store.list_all` will also return
        //   integrity checks. We need to filter it.
        let objects = service.backup_store.list_all().await?;

        let backups = objects.into_iter().filter(is_backup).collect::<Vec<_>>();

        // NOTE: S3 results are sorted in alphabetically ascending order,
        //   and backup names use Unix timestamps which are alphabetically
        //   sortable. The first result is the oldest backup.
        let Some(oldest_backup) = backups.first() else {
            return Ok(vec![]);
        };

        let checks = {
            // NOTE: If using the same bucket and prefix for both the backups and
            //   integrity checks, `service.backup_store.list_all` will also return
            //   integrity checks. We need to filter it.
            let objects = service
                .check_store
                .list_all_after(&oldest_backup.file_name)
                .await?
                .into_iter();

            objects
                .filter(|metadata| !is_backup(metadata))
                .map(|meta| meta.file_name)
                .collect::<Vec<_>>()
        };

        let signing_is_mandatory = service.signing_context.is_signing_mandatory;

        let mut dtos: Vec<BackupDto<BackupMetadataPartialDto>> = Vec::with_capacity(backups.len());

        for backup in backups.into_iter().rev() {
            let backup_file_name = backup.file_name;

            let is_signed = checks.contains(&format!("{backup_file_name}.sig"));
            let is_encrypted = backup_file_name.ends_with(".pgp");

            let can_be_restored = (!signing_is_mandatory) || is_signed;

            let backup_id = match BackupId::from_str(&backup_file_name) {
                Ok(name) => name,
                Err(err) => {
                    tracing::warn!("Skipping `{backup_file_name}`: {err:?}");
                    continue;
                }
            };

            dtos.push(BackupDto {
                metadata: BackupMetadataPartialDto {
                    created_at: backup_id.created_at,
                    size_bytes: backup.size_bytes,
                    is_signed,
                    is_encrypted,
                    can_be_restored,
                },
                description: backup_id.description.to_string(),
                id: backup_id,
            });
        }

        Ok(dtos)
    }

    pub(crate) async fn get_details(
        service: &BackupService,
        backup_id: &BackupId,
    ) -> Result<BackupDto<BackupMetadataFullDto>, anyhow::Error> {
        use crate::archiving::extract;
        use crate::verification::VerificationReport;

        let mut verification_report = VerificationReport::default();
        let verification_result = service
            .download_backup_and_check_integrity(
                &backup_id,
                backup_id.created_at,
                &mut verification_report,
            )
            .await;

        let mut decryption_report = DecryptionReport::default();
        let mut is_encryption_valid: Option<bool> = None;
        let can_be_restored: bool;
        match verification_result {
            Ok(verification_output) => {
                let extraction_result = extract(
                    &verification_output,
                    &backup_id,
                    &service.archiving_context.blueprints,
                    &service.decryption_context,
                    &mut decryption_report,
                );
                match extraction_result {
                    Ok(_) => {
                        is_encryption_valid = Some(true);
                        can_be_restored = true;
                    }
                    Err(err) => {
                        tracing::debug!("{err:#}");
                        is_encryption_valid = Some(false);
                        can_be_restored = false;
                    }
                }
            }
            Err(err) => {
                tracing::debug!("{err:#}");
                can_be_restored = false;
            }
        }

        let metadata = service
            .backup_store
            .metadata(&ObjectId::from(backup_id))
            .await?;

        let is_signed = verification_report.is_signed;
        let is_intact = verification_report.is_intact;
        let is_encrypted: bool = backup_id.extensions.contains(&Box::from("pgp"));

        let dto = BackupDto {
            metadata: BackupMetadataFullDto {
                created_at: backup_id.created_at,
                size_bytes: metadata.size_bytes,
                is_signed,
                is_encrypted,
                can_be_restored,
                is_intact,
                known_signing_keys: verification_report
                    .known_signing_keys
                    .into_iter()
                    .map(Into::into)
                    .collect(),
                encryption_key: decryption_report
                    .used_cert_and_subkey
                    .map(|(cert_fingerprint, _)| cert_fingerprint.to_spaced_hex()),
                is_encryption_valid,
            },
            description: backup_id.description.to_string(),
            id: backup_id.to_owned(),
        };

        Ok(dto)
    }

    pub(crate) async fn get_download_url(
        service: &BackupService,
        backup_id: &BackupId,
        ttl: std::time::Duration,
    ) -> Result<String, anyhow::Error> {
        // Apply max TTL from configuration.
        let ttl = ttl.clamp(
            std::time::Duration::ZERO,
            service.download_config.url_max_ttl,
        );

        service
            .backup_store
            .download_url(&ObjectId::from(backup_id), &ttl)
            .await
    }
}

mod restore {
    use crate::BackupService;
    use crate::archiving::*;
    use crate::backup_id::*;
    use crate::decryption::*;
    use crate::restoration;
    use crate::restoration::*;
    use crate::stats::*;
    use crate::verification::*;

    #[derive(Debug)]
    pub struct ExtractionSuccess<'a> {
        pub verification_report: VerificationReport,
        pub extraction_output: ExtractionOutput<'a>,
    }

    pub struct ExtractAndRestoreSuccess {
        pub verification_report: VerificationReport,
        pub restoration_output: RestorationOutput,
    }

    #[allow(unused_variables)]
    pub trait RestoreBackupEventHandler: Send + Sync {
        #[inline]
        fn on_path_restored(&mut self, backup_id: &BackupId, path: &std::path::Path) {}
    }

    pub(crate) async fn restore_backup<EventHandler>(
        service: &BackupService,
        backup_id: &BackupId,
        blueprint: &ArchiveBlueprint,
        event_handler: &mut EventHandler,
    ) -> Result<ExtractAndRestoreSuccess, RestorationError>
    where
        EventHandler: ExtractBackupEventHandler + RestoreBackupEventHandler,
    {
        let ExtractionSuccess {
            verification_report,
            extraction_output,
        } = service.extract_backup(backup_id, event_handler).await?;

        let restoration_output = service
            .restore_extracted_backup(backup_id, extraction_output, blueprint, event_handler)
            .await?;

        Ok(ExtractAndRestoreSuccess {
            verification_report,
            restoration_output,
        })
    }

    #[allow(unused_variables)]
    pub trait ExtractBackupEventHandler: Send + Sync {
        #[inline]
        fn on_restoration_start(&mut self, backup_id: &BackupId, backup_size: u64) {}

        /// WARN: Note that `on_raw_read` will be called one last time with
        ///   `len = 0`. This is on purpose, to keep the library completely
        ///   transparent, so make sure to skip this case if needed.
        #[inline]
        fn on_raw_read(&mut self, backup_id: &BackupId, len: usize) {}

        #[inline]
        fn on_decryption_finished(
            &mut self,
            backup_id: &BackupId,
            stats: ReadStats,
            report: DecryptionReport,
        ) {
        }

        #[inline]
        fn on_decompression_finished(&mut self, backup_id: &BackupId, stats: ReadStats) {}

        #[inline]
        fn on_extraction_finished(&mut self, backup_id: &BackupId, report: ExtractionReport) {}
    }

    pub(crate) async fn extract_backup<'a>(
        service: &'a BackupService,
        backup_id: &BackupId,
        event_handler: &mut impl ExtractBackupEventHandler,
    ) -> Result<ExtractionSuccess<'a>, ExtractionError> {
        let mut verification_report = VerificationReport::default();
        let verification_output = service
            .download_backup_and_check_integrity(
                &backup_id,
                backup_id.created_at,
                &mut verification_report,
            )
            .await?;

        let extraction_output = extract(
            &verification_output,
            &backup_id,
            &service.archiving_context.blueprints,
            &service.decryption_context,
            event_handler,
        )?;

        Ok(ExtractionSuccess {
            verification_report,
            extraction_output,
        })
    }

    #[inline]
    pub(crate) async fn restore_extracted_backup<'a>(
        backup_id: &BackupId,
        extraction_output: ExtractionOutput<'a>,
        blueprint: &ArchiveBlueprint,
        context: &restoration::Context,
        event_handler: &mut impl RestoreBackupEventHandler,
    ) -> Result<RestorationOutput, RestorationError> {
        restore(
            backup_id,
            extraction_output,
            blueprint,
            context,
            event_handler,
        )
    }
}

mod delete {
    use crate::BackupService;
    use crate::backup_id::*;
    use crate::stores::*;

    // NOTE: If using Object Lock, this method exits successfully and
    //   backups / integrity checks remain stored until locks are removed.
    pub(crate) async fn delete_backup(
        service: &BackupService,
        backup_id: &BackupId,
    ) -> Result<(), anyhow::Error> {
        let backup_id = ObjectId::from(backup_id);

        // Delete the backup object.
        let deleted_state = service.backup_store.delete(&backup_id).await?;
        match deleted_state {
            crate::stores::DeletedState::Deleted => {}
            crate::stores::DeletedState::MarkedForDeletion => tracing::warn!(
                "Backup `{backup_id}` not deleted, but marked for deletion \
                once object locks are removed."
            ),
        }
        tracing::info!("Object `{backup_id}` deleted.");

        // Delete all associated integrity checks.
        {
            let BulkDeleteOutput {
                deleted,
                marked_for_deletion,
                errors,
            } = service.check_store.delete_all(&backup_id).await?;

            // Log successes.
            for key in deleted {
                tracing::info!("Object `{key}` deleted.");
            }

            // Warn if a deletion only yielded a marker.
            for key in marked_for_deletion {
                tracing::warn!(
                    "Object `{key}` not deleted, but marked for deletion \
                    once object locks are removed."
                );
            }

            // Log errors.
            for error in errors {
                tracing::warn!("{error:#}");
            }
        }

        Ok(())
    }
}

mod backup_id {
    //! Backup ID serialization and deserialization.

    use anyhow::Context as _;

    /// Unique identifier of the backup.
    ///
    /// E.g. `prose%2Dbackup-1772432392-Automatic%20backup.tar.zst.pgp`.
    #[derive(Clone)]
    pub struct BackupId {
        pub prefix: Box<str>,

        pub created_at: time::UtcDateTime,

        pub description: Box<str>,

        pub extensions: Vec<Box<str>>,
    }

    impl BackupId {
        fn parse(str: &str) -> Result<Self, anyhow::Error> {
            let Some((prefix, rest)) = str.split_once('-') else {
                anyhow::bail!("File `{str}` has no prefix.");
            };

            let Some((timestamp_str, rest)) = rest.split_once('-') else {
                anyhow::bail!("File `{str}` is missing the timestamp prefix.");
            };

            let secs: i64 = timestamp_str
                .parse()
                .with_context(|| format!("Could not read integer from `{timestamp_str}`"))?;

            let created_at = time::UtcDateTime::from_unix_timestamp(secs)
                .context("Could not parse timestamp from file name")?;

            let Some((description, extensions)) = rest.split_once('.') else {
                anyhow::bail!("File `{str}` has no extension.");
            };

            // “URL decode” components.
            let prefix = urlencoding::decode(prefix)
                .with_context(|| format!("Backup prefix `{prefix}` contains invalid UTF-8"))?;
            let description = urlencoding::decode(description).with_context(|| {
                format!("Backup description `{description}` contains invalid UTF-8")
            })?;

            Ok(BackupId {
                prefix: Box::from(prefix),
                created_at,
                description: Box::from(description),
                extensions: extensions.split(".").map(Box::from).collect(),
            })
        }
    }

    impl std::fmt::Display for BackupId {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            let Self {
                prefix,
                created_at,
                description,
                extensions,
            } = self;

            // Arbitrary safety limits.
            assert!(prefix.len() <= 256);
            assert!(description.len() <= 256);
            // NOTE: Provide default values instead of passing empty strings.
            assert!(!prefix.is_empty());
            assert!(!description.is_empty());

            // “URL encode” components to get rid of spaces, emojis, etc.
            let prefix = urlencode_component(&prefix);
            let description = urlencode_component(&description);

            // Unix timestamp with second precision as 10 chars covers 2001-09-09
            // to 2286-11-20 (<2001-09-09 needs 9 chars, >2286-11-20 needs 11).
            // For correctness, we’ll still format the number as 10 digits with
            // leading zeros (even if not necessary).
            let created_at = created_at.unix_timestamp();
            assert!(created_at <= 9_999_999_999);
            debug_assert!(created_at > 999_999_999);

            let extensions = extensions.join(".");

            write!(f, "{prefix}-{created_at:010}-{description}.{extensions}")
        }
    }

    impl std::fmt::Debug for BackupId {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            std::fmt::Debug::fmt(&self.to_string(), f)
        }
    }

    #[cfg(feature = "test")]
    impl PartialEq for BackupId {
        fn eq(&self, other: &Self) -> bool {
            let Self {
                prefix,
                created_at,
                description,
                extensions,
            } = self;

            created_at.unix_timestamp() == other.created_at.unix_timestamp()
                && *description == other.description
                && *extensions == other.extensions
                && *prefix == other.prefix
        }
    }

    #[allow(clippy::let_and_return)]
    fn urlencode_component(str: &str) -> String {
        #[cfg(feature = "test")]
        use std::borrow::Cow;

        let res = urlencoding::encode(str);

        // Also percent-encode `-` to prevent incorrect splitting.
        #[cfg(feature = "test")]
        assert_eq!(
            urlencoding::decode("test%2Dext"),
            Ok(Cow::Borrowed("test-ext"))
        );
        let res = res.replace("-", "%2D");

        // Also percent-encode `.` to prevent incorrect file extension parsing.
        // WARN: Also prevents path traversal attacks. We should not be subject
        //   to it given that backup IDs are object storage keys but better be safe
        //   than sorry.
        #[cfg(feature = "test")]
        assert_eq!(
            urlencoding::decode("test%2Eext"),
            Ok(Cow::Borrowed("test.ext"))
        );
        let res = res.replace(".", "%2E");

        // Also percent-encode `/` to prevent incorrect parsing of HTTP
        // requests when a backup ID is used in the path.
        #[cfg(feature = "test")]
        debug_assert_eq!(
            urlencoding::decode("test%2Ffoo"),
            Ok(Cow::Borrowed("test/foo"))
        );
        let res = res.replace("/", "%2F");

        res
    }

    impl std::str::FromStr for BackupId {
        type Err = anyhow::Error;

        #[inline]
        fn from_str(str: &str) -> Result<Self, Self::Err> {
            Self::parse(str)
        }
    }

    #[cfg(test)]
    mod tests {
        #[test]
        #[cfg(feature = "test")]
        fn test_backup_id_components_parsing() -> Result<(), anyhow::Error> {
            use crate::BackupId;

            let components =
                BackupId::parse("prose%2Dbackup-1772432392-Automatic%20backup.tar.zst.pgp")?;
            assert_eq!(
                components,
                BackupId {
                    prefix: Box::from("prose-backup"),
                    created_at: time::UtcDateTime::UNIX_EPOCH + time::Duration::seconds(1772432392),
                    description: Box::from("Automatic backup"),
                    extensions: vec![
                        Box::from("tar"),
                        Box::from("zst"),
                        Box::from("pgp")
                    ],
                }
            );

            Ok(())
        }
    }
}
