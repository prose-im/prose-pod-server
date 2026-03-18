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
//!   let service = BackupService::from_config(&backup_config, blueprints)?;
//!
//!   let _backups = service.list_backups().await?;
//! }
//! ```

pub mod archiving;
mod compression;
pub mod config;
pub mod decryption;
pub mod encryption;
mod hashing;
mod pgp;
mod restoration;
pub mod signing;
pub mod stats;
pub mod stores;
mod util;
pub mod verification;

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Context as _;
pub use openpgp;
pub use tokio;
pub use toml;

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
    ///    ┌───┴────┐     ┌─────┴─────┐            │ PGP signing
    ///    │ Upload │     │   Hash    │            │ enabled?
    ///    │ backup │     │ (SHA 256) │            ◇───────┐
    ///    │  (S3)  │     └─────┬─────┘        Yes │       │ No
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
    pub async fn create_backup(
        &self,
        command: create::CreateBackupCommand<'_>,
    ) -> Result<create::CreateBackupSuccess, create::CreateBackupError> {
        crate::create::create_backup(self, command).await
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
        backup_file_name: &BackupFileName,
    ) -> Result<BackupDto<BackupMetadataFullDto>, anyhow::Error> {
        crate::read::get_details(self, backup_file_name).await
    }

    /// Get a short-lived URL to download a backup.
    #[inline]
    pub async fn get_download_url(
        &self,
        backup_name: &BackupFileName,
        ttl: std::time::Duration,
    ) -> Result<String, anyhow::Error> {
        crate::read::get_download_url(self, backup_name, ttl).await
    }

    #[inline]
    pub async fn restore_backup(
        &self,
        backup_name: &BackupFileName,
        blueprint: &archiving::ArchiveBlueprint,
    ) -> Result<ExtractAndRestoreSuccess, restoration::RestorationError> {
        crate::restore::restore_backup(self, backup_name, blueprint).await
    }

    #[inline]
    pub async fn extract_backup<'a>(
        &'a self,
        backup_name: &BackupFileName,
    ) -> Result<ExtractionSuccess<'a>, archiving::ExtractionError> {
        crate::restore::extract_backup(self, backup_name).await
    }

    #[inline]
    pub async fn restore_extracted_backup<'a>(
        &self,
        extraction_output: archiving::ExtractionOutput<'a>,
        blueprint: &archiving::ArchiveBlueprint,
    ) -> Result<restoration::RestorationOutput, restoration::RestorationError> {
        crate::restore::restore_extracted_backup(extraction_output, blueprint).await
    }

    #[inline]
    pub async fn delete_backup(&self, backup_name: &BackupFileName) -> Result<(), anyhow::Error> {
        crate::delete::delete_backup(self, backup_name).await
    }
}

impl BackupService {
    pub fn from_config(
        config: &BackupConfig,
        blueprints: HashMap<u8, archiving::ArchiveBlueprint>,
    ) -> Result<Self, anyhow::Error> {
        // NOTE: This gets inlined in release builds.
        Self::from_config_custom(
            config,
            archiving::Context { blueprints },
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
            });
        }

        let backup_store: Box<dyn ObjectStore> = match config.storage.backups {
            #[cfg(feature = "destination_s3")]
            config::StorageSubconfig::S3 { ref config } => Box::new(S3Store::from_config(config)),
            #[cfg(feature = "destination_fs")]
            config::StorageSubconfig::Fs { ref config } => {
                Box::new(FsStore::try_from_config(config, 0o600)?)
            }
        };
        let check_store: Box<dyn ObjectStore> = match config.storage.checks {
            #[cfg(feature = "destination_s3")]
            config::StorageSubconfig::S3 { ref config } => Box::new(S3Store::from_config(config)),
            #[cfg(feature = "destination_fs")]
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

    use crate::{BackupFileName, verification::PgpSignatureReport};

    #[derive(Debug)]
    pub struct BackupDto<Metadata> {
        /// Unique identifier (file name / object key) of the backup.
        ///
        /// E.g. `prose%2Dbackup-1772432392-Automatic%20backup.tar.zst.pgp`.
        pub id: BackupFileName,

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

        /// Fingerprint of the key used to sign the backup, if applicable.
        pub signing_keys: Vec<SigningKeyReportDto>,

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

    /// Information about a key used to sign
    #[derive(Debug)]
    pub struct SigningKeyReportDto {
        /// Unique fingerprint of the signing key.
        ///
        /// Note that for OpenPGP signatures, this is the certificate’s primary
        /// key fingerprint.
        pub fingerprint: String,

        /// Whether or not the backup signature was issued by a trusted entity.
        ///
        /// This doesn’t mean the signature is valid, which is indicated by
        /// [`is_valid`].
        ///
        /// [`is_valid`]: Self::is_valid
        pub is_trusted: bool,

        /// Whether or not the backup signature is valid.
        ///
        /// Note that this implies [`is_trusted`].
        ///
        /// [`is_trusted`]: Self::is_trusted
        pub is_valid: bool,
    }

    impl From<PgpSignatureReport> for SigningKeyReportDto {
        fn from(value: PgpSignatureReport) -> Self {
            Self {
                fingerprint: value.cert_fingerprint.to_spaced_hex(),
                is_trusted: value.is_trusted,
                is_valid: value.is_valid,
            }
        }
    }
}

mod create {
    use anyhow::Context as _;
    use composable_stream::*;

    use crate::archiving::{self, archive};
    use crate::compression::compress;
    use crate::dtos::{BackupDto, BackupMetadataPartialDto};
    use crate::encryption::{self, encrypt};
    use crate::hashing::digest;
    use crate::signing::pgp::pgp_sign;
    use crate::stats::{WriteStats, meter_writes};
    use crate::stores::ObjectStore as _;
    use crate::{BackupFileName, BackupName, BackupService};

    pub(crate) async fn create_backup(
        service: &BackupService,
        CreateBackupCommand {
            prefix,
            description,
            version,
            blueprint,
            additional_archive_data,
            #[cfg(feature = "test")]
            created_at,
        }: CreateBackupCommand<'_>,
    ) -> Result<CreateBackupSuccess, CreateBackupError> {
        use std::time::SystemTime;

        archiving::check_archiving_will_succeed(&blueprint)?;

        #[cfg(not(feature = "test"))]
        let created_at = SystemTime::now();

        let backup_name = BackupName::new(prefix, description, &created_at);

        let backup_file_name = match service.encryption_context {
            Some(encryption::EncryptionContext::Pgp { .. }) => {
                backup_name.with_extension("tar.zst.pgp")
            }
            None => backup_name.with_extension("tar.zst"),
        };

        // Try to open sink first, to abort early if something is wrong.
        let upload_backup = service
            .backup_store
            .writer(&backup_file_name)
            .await
            .map_err(CreateBackupError::CannotCreateSink)?;

        let start = SystemTime::now();

        let backup_writer = archive(&blueprint, version, &additional_archive_data)
            .then(compress(&service.compression_config))
            .then(eventually(service.encryption_context.as_ref(), |ctx| {
                encrypt(ctx, created_at)
            }))
            // Record stats so we can know the final size of the backup.
            .then(meter_writes(WriteStats::new()))
            .tee(digest(&service.hashing_config), Vec::<u8>::new())
            .opt_tee(
                service.signing_context.pgp.as_ref(),
                |ctx| pgp_sign(ctx, created_at),
                Vec::<u8>::new(),
            )
            .build(upload_backup)?;

        let ((backup_upload, digest), pgp_signature) = backup_writer.finalize();

        let (backup_upload, backup_stats) = backup_upload?;

        let mut digest_ids: Vec<BackupFileName> = Vec::new();
        let mut checks_upload_durations: Vec<(BackupFileName, std::time::Duration)> = Vec::new();

        // Upload SHA-256 digest.
        upload_integrity_check(
            digest?,
            backup_file_name.with_extension("sha256"),
            &service,
            &mut checks_upload_durations,
            &mut digest_ids,
        )
        .await?;

        let mut signature_ids: Vec<BackupFileName> = Vec::new();

        let is_signed = pgp_signature.is_some();

        // Upload OpenPGP signature.
        if let Some(pgp_signature) = pgp_signature {
            upload_integrity_check(
                pgp_signature?,
                // NOTE: OpenPGP will likely forever be the only signing protocol
                //   we support, but if we ever add one that also uses the `.sig`
                //   extension we can just use `.<protocol>.sig` for it.
                backup_file_name.with_extension("sig"),
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
        let backup_upload_duration = SystemTime::now().duration_since(start).unwrap_or_default();

        let size_bytes = backup_stats.bytes_written;
        tracing::info!("Created backup {backup_file_name:?} ({size_bytes}B).");

        // Construct the response.
        Ok(CreateBackupSuccess {
            backup: BackupDto {
                id: backup_file_name.clone(),
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
                backup_id: backup_file_name,
                digest_ids,
                signature_ids,
            },
            stats: CreateBackupStats {
                backup_upload_duration,
                checks_upload_durations,
            },
        })
    }

    async fn upload_integrity_check(
        data: Vec<u8>,
        key: BackupFileName,
        service: &BackupService,
        checks_upload_durations: &mut Vec<(BackupFileName, std::time::Duration)>,
        uploaded: &mut Vec<BackupFileName>,
    ) -> Result<(), CreateBackupError> {
        use std::time::SystemTime;

        let start = SystemTime::now();

        let mut uploader = service
            .check_store
            .writer(&key)
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

        checks_upload_durations.push((
            key.clone(),
            SystemTime::now().duration_since(start).unwrap(),
        ));
        uploaded.push(key);

        Ok(())
    }

    #[derive(Debug)]
    pub struct CreateBackupCommand<'a> {
        /// Desired backup prefix (e.g. “prose-backup”).
        pub prefix: &'a str,

        /// Desired backup description (e.g. “Automatic backup”).
        pub description: &'a str,

        pub version: u8,

        pub blueprint: &'a archiving::ArchiveBlueprint,

        /// Some more data to insert in the archive before it’s built.
        pub additional_archive_data: Vec<(String, bytes::Bytes)>,

        /// Timestamp which should be associated with the backup.
        ///
        /// This is only useful in tests, as we have no way to read data as it was
        /// at the previous date. It’s only metadata.
        #[cfg(feature = "test")]
        pub created_at: std::time::SystemTime,
    }

    impl<'a> CreateBackupCommand<'a> {
        #[inline]
        pub fn new(
            prefix: &'a str,
            description: &'a str,
            version: u8,
            blueprint: &'a archiving::ArchiveBlueprint,
        ) -> Self {
            Self {
                prefix,
                description,
                version,
                blueprint,
                additional_archive_data: Vec::with_capacity(0),
                #[cfg(feature = "test")]
                created_at: std::time::SystemTime::now(),
            }
        }

        #[cfg(feature = "test")]
        #[inline]
        pub fn created_at(mut self, created_at: std::time::SystemTime) -> Self {
            self.created_at = created_at;
            self
        }
    }

    #[derive(Debug)]
    pub struct CreateBackupOutput {
        /// Unique identifier (file name / object key) of the backup.
        ///
        /// E.g. `prose%2Dbackup-1772432392-Automatic%20backup.tar.zst.pgp`.
        pub backup_id: BackupFileName,

        /// Unique identifiers (file names / object keys) of backup digests
        /// (cryptographic checksums).
        ///
        /// E.g. `prose%2Dbackup-1772432392-Automatic%20backup.tar.zst.pgp.sha256`.
        pub digest_ids: Vec<BackupFileName>,

        /// Unique identifiers (file names / object keys) of backup signatures.
        ///
        /// E.g. `prose%2Dbackup-1772432392-Automatic%20backup.tar.zst.pgp.sig`.
        pub signature_ids: Vec<BackupFileName>,
    }

    #[derive(Debug)]
    pub struct CreateBackupStats {
        pub backup_upload_duration: std::time::Duration,
        pub checks_upload_durations: Vec<(BackupFileName, std::time::Duration)>,
    }

    #[derive(Debug)]
    pub struct CreateBackupSuccess {
        pub backup: BackupDto<BackupMetadataPartialDto>,
        pub output: CreateBackupOutput,
        pub stats: CreateBackupStats,
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
}

mod read {
    use crate::dtos::*;
    use crate::stores::ObjectStore as _;
    use crate::{BackupFileName, BackupFileNameComponents, BackupService};

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

        let backups = service.backup_store.list_all().await?;

        // NOTE: S3 results are sorted in alphabetically ascending order,
        //   and backup names use Unix timestamps which are alphabetically
        //   sortable. The first result is the oldest backup.
        let Some(oldest_backup) = backups.first() else {
            return Ok(vec![]);
        };

        let checks = service
            .check_store
            .list_all_after(&oldest_backup.file_name)
            .await?
            .into_iter()
            .map(|meta| meta.file_name)
            .collect::<Vec<_>>();

        let signing_is_mandatory = service.signing_context.is_signing_mandatory;

        let mut dtos: Vec<BackupDto<BackupMetadataPartialDto>> = Vec::with_capacity(backups.len());

        for backup in backups.into_iter().rev() {
            let backup_file_name = backup.file_name;

            let is_signed = checks.contains(&format!("{backup_file_name}.sig"));
            let is_encrypted = backup_file_name.ends_with(".pgp");

            let can_be_restored = (!signing_is_mandatory) || is_signed;

            let backup_file_name = match BackupFileName::from_str(&backup_file_name) {
                Ok(name) => name,
                Err(err) => {
                    tracing::warn!("Skipping `{backup_file_name}`: {err:?}");
                    continue;
                }
            };

            let BackupFileNameComponents {
                created_at,
                description,
                ..
            } = match BackupFileNameComponents::parse(&backup_file_name) {
                Ok(components) => components,
                Err(err) => {
                    tracing::warn!("Skipping `{backup_file_name}`: {err:?}");
                    continue;
                }
            };

            dtos.push(BackupDto {
                metadata: BackupMetadataPartialDto {
                    created_at,
                    size_bytes: backup.size_bytes,
                    is_signed,
                    is_encrypted,
                    can_be_restored,
                },
                description: description.into_owned(),
                id: backup_file_name,
            });
        }

        Ok(dtos)
    }

    pub(crate) async fn get_details(
        service: &BackupService,
        backup_file_name: &BackupFileName,
    ) -> Result<BackupDto<BackupMetadataFullDto>, anyhow::Error> {
        use crate::archiving::{ExtractionStats, extract};
        use crate::decryption::DecryptionReport;
        use crate::verification::VerificationReport;

        let parsed_backup_name = BackupFileNameComponents::parse(&backup_file_name)?;

        let mut verification_report = VerificationReport::default();
        let verification_result = service
            .download_backup_and_check_integrity(
                &backup_file_name,
                parsed_backup_name.created_at,
                &mut verification_report,
            )
            .await;

        let mut decryption_report = DecryptionReport::default();
        let mut extraction_stats = ExtractionStats::default();
        let mut is_encryption_valid: Option<bool> = None;
        let can_be_restored: bool;
        match verification_result {
            Ok(verification_output) => {
                let extraction_result = extract(
                    &verification_output,
                    &parsed_backup_name,
                    &service.archiving_context.blueprints,
                    &service.decryption_context,
                    &mut decryption_report,
                    &mut extraction_stats,
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

        let metadata = service.backup_store.metadata(&backup_file_name).await?;

        let is_signed = verification_report.is_signed;
        let is_intact = verification_report.is_intact;
        let is_encrypted: bool = backup_file_name.ends_with(".pgp");

        let dto = BackupDto {
            metadata: BackupMetadataFullDto {
                created_at: parsed_backup_name.created_at,
                size_bytes: metadata.size_bytes,
                is_signed,
                is_encrypted,
                can_be_restored,
                is_intact,
                signing_keys: verification_report
                    .signing_keys
                    .into_iter()
                    .map(Into::into)
                    .collect(),
                encryption_key: decryption_report
                    .used_cert_and_subkey
                    .map(|(cert_fingerprint, _)| cert_fingerprint.to_spaced_hex()),
                is_encryption_valid,
            },
            description: parsed_backup_name.description.into_owned(),
            id: backup_file_name.to_owned(),
        };

        Ok(dto)
    }

    pub(crate) async fn get_download_url(
        service: &BackupService,
        backup_name: &BackupFileName,
        ttl: std::time::Duration,
    ) -> Result<String, anyhow::Error> {
        // Apply max TTL from configuration.
        let ttl = ttl.clamp(
            std::time::Duration::ZERO,
            service.download_config.url_max_ttl,
        );

        service.backup_store.download_url(&backup_name, &ttl).await
    }
}

mod restore {
    use crate::archiving::*;
    use crate::decryption::*;
    use crate::restoration::*;
    use crate::verification::*;
    use crate::{BackupFileName, BackupFileNameComponents, BackupService};

    #[derive(Debug)]
    pub struct ExtractionSuccess<'a> {
        pub verification_report: VerificationReport,
        pub decryption_report: DecryptionReport,
        pub extraction_output: ExtractionOutput<'a>,
        pub extraction_stats: ExtractionStats,
    }

    pub struct ExtractAndRestoreSuccess {
        pub verification_report: VerificationReport,
        pub decryption_report: DecryptionReport,
        pub extraction_stats: ExtractionStats,
        pub restoration_output: RestorationOutput,
    }

    pub(crate) async fn restore_backup(
        service: &BackupService,
        backup_name: &BackupFileName,
        blueprint: &ArchiveBlueprint,
    ) -> Result<ExtractAndRestoreSuccess, RestorationError> {
        let ExtractionSuccess {
            verification_report,
            decryption_report,
            extraction_output,
            extraction_stats,
        } = service.extract_backup(backup_name).await?;

        let restoration_output = service
            .restore_extracted_backup(extraction_output, blueprint)
            .await?;

        Ok(ExtractAndRestoreSuccess {
            verification_report,
            decryption_report,
            extraction_stats,
            restoration_output,
        })
    }

    pub(crate) async fn extract_backup<'a>(
        service: &'a BackupService,
        backup_name: &BackupFileName,
    ) -> Result<ExtractionSuccess<'a>, ExtractionError> {
        // Parse backup name first.
        // Avoids unnecessary I/O if malformed.
        let parsed_backup_name @ BackupFileNameComponents { created_at, .. } =
            BackupFileNameComponents::parse(&backup_name)?;

        let mut verification_report = VerificationReport::default();
        let verification_output = service
            .download_backup_and_check_integrity(&backup_name, created_at, &mut verification_report)
            .await?;

        let mut decryption_report = DecryptionReport::default();
        let mut extraction_stats = ExtractionStats::default();
        let extraction_output = extract(
            &verification_output,
            &parsed_backup_name,
            &service.archiving_context.blueprints,
            &service.decryption_context,
            &mut decryption_report,
            &mut extraction_stats,
        )?;

        // TODO: Cache?
        drop(verification_output.tmp_dir);

        Ok(ExtractionSuccess {
            verification_report,
            decryption_report,
            extraction_output,
            extraction_stats,
        })
    }

    #[inline]
    pub(crate) async fn restore_extracted_backup<'a>(
        extraction_output: ExtractionOutput<'a>,
        blueprint: &ArchiveBlueprint,
    ) -> Result<RestorationOutput, RestorationError> {
        restore(extraction_output, blueprint)
    }
}

mod delete {
    use crate::stores::{BulkDeleteOutput, ObjectStore as _};
    use crate::{BackupFileName, BackupService};

    // NOTE: If using Object Lock, this method exits successfully and
    //   backups / integrity checks remain stored until locks are removed.
    pub(crate) async fn delete_backup(
        service: &BackupService,
        backup_name: &BackupFileName,
    ) -> Result<(), anyhow::Error> {
        // Delete the backup object.
        let deleted_state = service.backup_store.delete(&backup_name).await?;
        match deleted_state {
            crate::stores::DeletedState::Deleted => {}
            crate::stores::DeletedState::MarkedForDeletion => tracing::warn!(
                "Backup `{backup_name}` not deleted, but marked for deletion \
                once object locks are removed."
            ),
        }
        tracing::info!("Object `{backup_name}` deleted.");

        // Delete all associated integrity checks.
        {
            let BulkDeleteOutput {
                deleted,
                marked_for_deletion,
                errors,
            } = service.check_store.delete_all(&backup_name).await?;

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

// MARK: File name serialization and deserialization

#[derive(Debug)]
#[cfg_attr(feature = "test", derive(PartialEq, Eq))]
pub(crate) struct BackupFileNameComponents<'a> {
    #[allow(dead_code)]
    pub prefix: Cow<'a, str>,

    pub created_at: time::UtcDateTime,

    pub description: Cow<'a, str>,

    pub extensions: &'a str,
}

impl<'a> BackupFileNameComponents<'a> {
    fn parse(file_name: &'a str) -> Result<Self, anyhow::Error> {
        let Some((prefix, rest)) = file_name.split_once('-') else {
            anyhow::bail!("File `{file_name}` has no prefix.");
        };

        let Some((timestamp_str, rest)) = rest.split_once('-') else {
            anyhow::bail!("File `{file_name}` is missing the timestamp prefix.");
        };

        let secs: i64 = timestamp_str
            .parse()
            .with_context(|| format!("Could not read integer from `{timestamp_str}`"))?;

        let created_at = time::UtcDateTime::from_unix_timestamp(secs)
            .context("Could not parse timestamp from file name")?;

        let Some((description, extensions)) = rest.split_once('.') else {
            anyhow::bail!("File `{file_name}` has no extension.");
        };

        // “URL decode” components.
        let prefix = urlencoding::decode(prefix)
            .with_context(|| format!("Backup prefix `{prefix}` contains invalid UTF-8"))?;
        let description = urlencoding::decode(description).with_context(|| {
            format!("Backup description `{description}` contains invalid UTF-8")
        })?;

        Ok(BackupFileNameComponents {
            prefix,
            created_at,
            description,
            extensions,
        })
    }
}

/// Name of a backup (base name of the file).
///
/// E.g. `1772432392-Automatic%20backup`.
#[derive(Clone)]
#[repr(transparent)]
pub struct BackupName(String);

impl BackupName {
    pub fn new(prefix: &str, description: &str, created_at: &std::time::SystemTime) -> Self {
        use crate::util::SystemTimeExt as _;

        // Arbitrary safety limits.
        assert!(prefix.len() <= 256);
        assert!(description.len() <= 256);
        // NOTE: Provide default values instead of passing empty strings.
        assert!(!prefix.is_empty());
        assert!(!description.is_empty());

        // “URL encode” components to get rid of spaces, emojis, etc.
        let prefix = urlencode_file_name_component(prefix);
        let description = urlencode_file_name_component(description);

        // Unix timestamp with second precision as 10 chars covers 2001-09-09
        // to 2286-11-20 (<2001-09-09 needs 9 chars, >2286-11-20 needs 11).
        // For correctness, we’ll still format the number as 10 digits with
        // leading zeros (even if not necessary).
        let created_at = created_at.unix_timestamp();
        assert!(created_at <= 9_999_999_999);
        debug_assert!(created_at > 999_999_999);
        let backup_name = format!("{prefix}-{created_at:010}-{description}");

        Self(backup_name)
    }
}

#[allow(clippy::let_and_return)]
fn urlencode_file_name_component(str: &str) -> String {
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

impl BackupName {
    pub fn with_extension(&self, extension: &'static str) -> BackupFileName {
        debug_assert!(!extension.starts_with('.'));

        let suffix_start_idx = self.0.len();
        BackupFileName {
            value: format!("{self}.{extension}"),
            suffix_start_idx,
        }
    }
}

impl std::fmt::Debug for BackupName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.0, f)
    }
}

impl std::fmt::Display for BackupName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

/// Name of a backup file (base name with extensions).
///
/// E.g. `prose%2Dbackup-1772432392-Automatic%20backup.tar.zst.pgp`.
#[derive(Clone)]
pub struct BackupFileName {
    value: String,

    /// Index of the dot before the file extention.
    suffix_start_idx: usize,
}

impl std::fmt::Debug for BackupFileName {
    #[inline]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.value, f)
    }
}

impl std::fmt::Display for BackupFileName {
    #[inline]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.value, f)
    }
}

impl AsRef<std::path::Path> for BackupFileName {
    #[inline]
    fn as_ref(&self) -> &std::path::Path {
        self.value.as_ref()
    }
}

impl std::cmp::PartialEq for BackupFileName {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BackupFileNameError {
    #[error("Backup file name has no extension.")]
    NoExtension,
}

impl std::str::FromStr for BackupFileName {
    type Err = BackupFileNameError;

    fn from_str(file_name: &str) -> Result<Self, Self::Err> {
        match file_name.find('.') {
            Some(suffix_start_idx) => Ok(Self {
                value: file_name.to_owned(),
                suffix_start_idx,
            }),
            None => Err(BackupFileNameError::NoExtension),
        }
    }
}

impl BackupFileName {
    pub fn try_from(str: impl AsRef<str>) -> Result<Self, <Self as std::str::FromStr>::Err> {
        std::str::FromStr::from_str(str.as_ref())
    }

    /// Get the file base name.
    ///
    /// ```
    /// # use prose_backup::BackupFileName;
    /// # use std::str::FromStr as _;
    /// let file_name = BackupFileName::from_str("test.foo.bar").unwrap();
    /// assert_eq!(file_name.basename(), "test");
    /// ```
    pub fn basename(&self) -> &str {
        &self.value[..self.suffix_start_idx]
    }

    /// Get the extensions of the file name (no leading `.`).
    ///
    /// ```
    /// # use prose_backup::BackupFileName;
    /// # use std::str::FromStr as _;
    /// let file_name = BackupFileName::from_str("test.foo.bar").unwrap();
    /// assert_eq!(file_name.extension(), "foo.bar");
    /// ```
    pub fn extension(&self) -> &str {
        &self.value[(self.suffix_start_idx + 1)..]
    }

    /// Push a new extension to the file name (keeps existing ones).
    ///
    /// ```
    /// # use prose_backup::BackupFileName;
    /// # use std::str::FromStr as _;
    /// let file_name = BackupFileName::from_str("test.foo.bar").unwrap();
    /// let other_file_name = file_name.with_extension("baz");
    /// assert_eq!(other_file_name.extension(), "foo.bar.baz");
    /// ```
    pub fn with_extension(&self, extension: &'static str) -> Self {
        debug_assert!(!extension.starts_with('.'));
        assert!(!extension.ends_with('.'));

        Self {
            value: format!("{self}.{extension}", self = self.value),
            suffix_start_idx: self.suffix_start_idx,
        }
    }
}

impl std::ops::Deref for BackupFileName {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.value.as_str()
    }
}

impl AsRef<String> for BackupFileName {
    fn as_ref(&self) -> &String {
        &self.value
    }
}

impl AsRef<str> for BackupFileName {
    fn as_ref(&self) -> &str {
        self.value.as_str()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_backup_file_name_components_parsing() -> Result<(), anyhow::Error> {
        use crate::BackupFileNameComponents;
        use std::borrow::Cow;

        let components = BackupFileNameComponents::parse(
            "prose%2Dbackup-1772432392-Automatic%20backup.tar.zst.pgp",
        )?;
        assert_eq!(
            components,
            BackupFileNameComponents {
                prefix: Cow::Borrowed("prose-backup"),
                created_at: time::UtcDateTime::UNIX_EPOCH + time::Duration::seconds(1772432392),
                description: Cow::Borrowed("Automatic backup"),
                extensions: "tar.zst.pgp",
            }
        );

        Ok(())
    }
}
