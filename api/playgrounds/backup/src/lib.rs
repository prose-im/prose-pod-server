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
//!   let backup_store: prose_backup::stores::S3 = todo!();
//!   let check_store: prose_backup::stores::S3 = todo!();
//!
//!   let service = BackupService::from_config(
//!     BackupConfig::default(),
//!     backup_store,
//!     check_store,
//!   )?;
//!
//!   let _backups = service.list_backups().await?;
//! }
//! ```

mod archiving;
mod compression;
pub mod config;
pub mod decryption;
pub mod encryption;
mod hashing;
mod pgp;
mod restoration;
pub mod signing;
mod stats;
pub mod stores;
mod util;
pub mod verification;
mod writer_chain;

use std::borrow::Cow;

use anyhow::Context as _;
pub use openpgp;
pub use tokio;
pub use toml;

use crate::stores::{ObjectStore, S3Store};

pub use self::archiving::ArchiveBlueprint;
pub use self::config::BackupConfig;
pub use self::restoration::RestorationSuccess;

// MARK: Service

/// Convenient alias to [`ProseBackupService`] with sensible defaults.
pub type BackupService<BackupStore = S3Store, CheckStore = S3Store> =
    ProseBackupService<BackupStore, CheckStore>;

/// Backup service. Central component of the library.
pub struct ProseBackupService<BackupStore, CheckStore> {
    pub compression_config: config::CompressionConfig,
    pub hashing_config: config::HashingConfig,
    pub encryption_context: Option<encryption::Context>,
    pub signing_context: signing::Context,
    pub verification_context: verification::Context,
    pub decryption_context: decryption::Context,
    pub download_config: config::DownloadConfig,

    pub backup_store: BackupStore,
    pub check_store: CheckStore,
}

impl<BackupStore, CheckStore> ProseBackupService<BackupStore, CheckStore> {
    pub fn from_config(
        config: BackupConfig,
        backup_store: BackupStore,
        check_store: CheckStore,
    ) -> Result<Self, anyhow::Error> {
        // NOTE: This gets inlined in release builds.
        Self::from_config_custom(
            config,
            backup_store,
            check_store,
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
        config: BackupConfig,
        backup_store: BackupStore,
        check_store: CheckStore,
        get_pgp_cert: impl Fn(&std::path::PathBuf) -> Result<openpgp::Cert, anyhow::Error>,
        pgp_policy: impl Fn() -> P,
    ) -> Result<Self, anyhow::Error>
    where
        P: openpgp::policy::Policy + 'static,
    {
        use decryption::PgpDecryptionContext;
        use signing::PgpSigningContext;
        use verification::{PgpVerificationContext, PgpVerificationHelper};

        let encryption_context = match config.encryption.mode {
            config::EncryptionMode::Off => None,
            config::EncryptionMode::Pgp => {
                let Some(pgp) = config.encryption.pgp.as_ref() else {
                    return Err(anyhow::Error::msg(
                        "`encryption.mode` is `\"pgp\"` but `encryption.pgp` is missing.",
                    ));
                };

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
                    helper: PgpVerificationHelper {
                        certs: vec![pgp_cert],
                    },
                    policy: Box::new(pgp_policy()),
                })
            }
            None => None,
        };
        let verification_context = verification::Context {
            pgp: pgp_verification_context,
        };

        let mut decryption_context = decryption::Context::default();
        if let Some(pgp) = config.encryption.pgp.as_ref() {
            let pgp_cert = get_pgp_cert(&pgp.tsk)?;
            decryption_context.pgp = Some(PgpDecryptionContext {
                tsks: vec![pgp_cert],
                policy: Box::new(pgp_policy()),
            });
        }

        Ok(Self {
            compression_config: config.compression,
            hashing_config: config.hashing,
            encryption_context,
            signing_context,
            verification_context,
            decryption_context,
            backup_store,
            check_store,
            download_config: config.download,
        })
    }
}

// MARK: DTOs

use self::dtos::*;
pub mod dtos {
    //! [Data Transfer Objects].
    //!
    //! [Data Transfer Objects]: https://en.wikipedia.org/wiki/Data_transfer_object "“Data transfer object” on Wikipedia"

    #[derive(Debug)]
    pub struct BackupDto {
        /// Unique identifier (file name / object key) of the backup.
        ///
        /// E.g. `prose%2Dbackup-1772432392-Automatic%20backup.tar.zst.pgp`.
        pub id: Box<str>,

        /// Metadata associated with the backup.
        pub metadata: BackupMetadataPartialDto,
    }

    #[derive(Debug)]
    pub struct BackupMetadataPartialDto {
        /// Description of the backup.
        ///
        /// E.g. “Automatic backup”.
        pub description: String,

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
        /// Description of the backup.
        ///
        /// E.g. “Automatic backup”.
        pub description: String,

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
        /// is in [`is_signature_trusted`] and [`is_signature_valid`].
        ///
        /// [`is_signature_trusted`]: Self::is_signature_trusted
        /// [`is_signature_valid`]: Self::is_signature_valid
        pub is_signed: bool,

        /// Fingerprint of the key used to sign the backup, if applicable.
        pub signing_key: Option<openpgp::Fingerprint>,

        /// Whether or not the backup signature was issued by a trusted entity.
        ///
        /// This doesn’t mean the signature is valid, which is indicated by
        /// [`is_signature_valid`].
        ///
        /// [`is_signature_valid`]: Self::is_signature_valid
        pub is_signature_trusted: Option<bool>,

        /// Whether or not the backup was signed by a trusted issuer.
        pub is_signature_valid: Option<bool>,

        /// Whether or not the backup is encrypted.
        pub is_encrypted: bool,

        /// Fingerprint of the key used to encrypt the backup, if applicable.
        pub encryption_key: Option<openpgp::Fingerprint>,

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
}

// MARK: Create backup

#[derive(Debug)]
pub struct CreateBackupCommand<'a> {
    /// Desired backup prefix (e.g. “prose-backup”).
    pub prefix: &'a str,

    /// Desired backup description (e.g. “Automatic backup”).
    pub description: &'a str,

    /// Timestamp which should be associated with the backup.
    ///
    /// This is only useful in tests, as we have no way to read data as it was
    /// at the previous date. It’s only metadata.
    #[cfg(feature = "test")]
    pub created_at: std::time::SystemTime,
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

impl<S1: ObjectStore, S2: ObjectStore> ProseBackupService<S1, S2> {
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
    ///                    │ Encrypt | │
    ///                    |  (PGP)  │ │
    ///                    └────┬────┘ │
    ///                         ◇──────┘
    ///      ╺━┯━━━━━━━━━━━━━━━━┿━━━━━━━━━━━━━━━━━━━┯━╸
    ///    ┌───┴────┐     ┌─────┴─────┐             │ PGP signing
    ///    | Upload |     │   Hash    |             │ enabled?
    ///    | backup |     | (SHA 256) │             ◇───────┐
    ///    |  (S3)  |     └─────┬─────┘         Yes │       │ No
    ///    └───┬────┘  ┌────────┴─────────┐     ┌───┴───┐   ◯
    ///        ◯       | Upload integrity |     │ Sign  |
    ///                |    check (S3)    |     | (PGP) │
    ///                └────────┬─────────┘     └───┬───┘
    ///                         ◯             ┌─────┴─────┐
    ///                                       |  Upload   |
    ///                                       | signature |
    ///                                       |   (S3)    |
    ///                                       └─────┬─────┘
    ///                                             ◯
    /// ```
    pub async fn create_backup(
        &self,
        CreateBackupCommand {
            prefix,
            description,
            #[cfg(feature = "test")]
            created_at,
        }: CreateBackupCommand<'_>,
        archiving_blueprint: &ArchiveBlueprint,
    ) -> Result<CreateBackupOutput, CreateBackupError> {
        use crate::hashing::{DigestWriter, Sha256DigestWriter};

        archiving::check_archiving_will_succeed(&archiving_blueprint)?;

        #[cfg(not(feature = "test"))]
        let created_at = std::time::SystemTime::now();

        let backup_name = BackupName::new(prefix, description, &created_at);

        let backup_file_name = match self.encryption_context {
            Some(encryption::EncryptionContext::Pgp { .. }) => {
                backup_name.with_extension("tar.zst.pgp")
            }
            None => backup_name.with_extension("tar.zst"),
        };

        let upload_backup = self
            .backup_store
            .writer(&backup_file_name)
            .await
            .map_err(CreateBackupError::CannotCreateSink)?;

        // NOTE: We create only one writer in the form of an enum because:
        //   1. It does not make much sense to create multiple digests
        //   2. We ensure there is always at least one
        let mut digest_writer = match self.hashing_config.algorithm {
            config::HashingAlgorithm::Sha256 => DigestWriter::Sha256(Sha256DigestWriter::new()),
        };

        // NOTE: While it would be tempting to try to factor this so we can
        //   handle _n_ writers and not forget to call `finalize` on it,
        //   Rust’s borrow checker makes it very complicated. It would require
        //   quite a lot of new types, making the code more complicated to read
        //   but also compile as it would involve a lot of generics. Let’s make
        //   it easier for both humans and `rustc` to figure out what’s going
        //   on, by keeping it explicit.
        let mut pgp_signature: Vec<u8> = Vec::new();
        let mut pgp_signature_writer = match self.signing_context.pgp.as_ref() {
            Some(context) => {
                let writer = context
                    .new_writer(&mut pgp_signature, created_at)
                    .map_err(CreateBackupError::CannotSign)?;
                Some(writer)
            }
            None => None,
        };

        let (writer, finalize_backup) = writer_chain::builder()
            .archive(&archiving_blueprint)
            .compress(&self.compression_config)
            .encrypt_if_possible(self.encryption_context.as_ref(), created_at)
            .tee(&mut digest_writer)
            .opt_tee(pgp_signature_writer.as_mut())
            .build(upload_backup)?;

        () = finalize_backup(writer)?;

        let mut digest_ids: Vec<BackupFileName> = Vec::new();

        // SHA-256 digest.
        {
            let digest = digest_writer
                .finalize()
                .map_err(CreateBackupError::HashingFailed)?;

            let file_name = backup_file_name.with_extension("sha256");

            let mut uploader = self
                .check_store
                .writer(&file_name)
                .await
                .map_err(CreateBackupError::CannotCreateSink)?;

            let mut cursor = std::io::Cursor::new(digest);
            std::io::copy(&mut cursor, &mut uploader)
                .map_err(CreateBackupError::IntegrityCheckUploadFailed)?;

            digest_ids.push(file_name);
        }

        let mut signature_ids: Vec<BackupFileName> = Vec::new();

        // OpenPGP signature.
        if pgp_signature_writer.is_some() {
            // NOTE(RemiBardon): Don’t ask me why, but the borrow checker
            //   doesn’t accept opening the optional with a `match` statement.
            //   It makes no sense to me why, but I guess we’ll unwrap then…
            let sig_writer = pgp_signature_writer.unwrap();

            () = sig_writer
                .finalize()
                .map_err(CreateBackupError::SigningFailed)?;

            // NOTE: OpenPGP will likely forever be the only signing protocol
            //   we support, but if we ever add one that also uses the `.sig`
            //   extension we can just use `.<protocol>.sig` for it.
            let file_name = backup_file_name.with_extension("sig");

            let mut uploader = self
                .check_store
                .writer(&file_name)
                .await
                .map_err(CreateBackupError::CannotCreateSink)?;

            let mut cursor = std::io::Cursor::new(pgp_signature);
            std::io::copy(&mut cursor, &mut uploader)
                .map_err(CreateBackupError::IntegrityCheckUploadFailed)?;

            signature_ids.push(file_name);
        }

        Ok(CreateBackupOutput {
            backup_id: backup_file_name,
            digest_ids,
            signature_ids,
        })
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

    #[error("Backup hashing failed: {0:?}")]
    HashingFailed(anyhow::Error),

    #[error("Cannot sign backup: {0:?}")]
    CannotSign(anyhow::Error),

    #[error("Backup signing failed: {0:?}")]
    SigningFailed(anyhow::Error),

    #[error("Failed uploading backup integrity check: {0:?}")]
    IntegrityCheckUploadFailed(std::io::Error),

    #[error("{0:?}")]
    Other(anyhow::Error),
}

// MARK: Read

impl<S1: ObjectStore, S2: ObjectStore> ProseBackupService<S1, S2> {
    /// List all backups, in alphabetically descending order.
    pub async fn list_backups(&self) -> Result<Vec<BackupDto>, anyhow::Error> {
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

        let backups = self.backup_store.list_all().await?;

        // NOTE: S3 results are sorted in alphabetically ascending order,
        //   and backup names use Unix timestamps which are alphabetically
        //   sortable. The first result is the oldest backup.
        let Some(oldest_backup) = backups.first() else {
            return Ok(vec![]);
        };

        let checks = self
            .check_store
            .list_all_after(&oldest_backup.file_name)
            .await?
            .into_iter()
            .map(|meta| meta.file_name)
            .collect::<Vec<_>>();

        let signing_is_mandatory = self.signing_context.is_signing_mandatory;

        let mut dtos: Vec<BackupDto> = Vec::with_capacity(backups.len());

        for backup in backups.into_iter().rev() {
            let backup_file_name = backup.file_name;

            let is_signed = checks.contains(&format!("{backup_file_name}.sig"));
            let is_encrypted = backup_file_name.ends_with(".pgp");

            let can_be_restored = (!signing_is_mandatory) || is_signed;

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

            let backup_name = match urlencoding::decode(&backup_file_name) {
                Ok(backup_name) => backup_name,
                Err(err) => {
                    tracing::warn!("Skipping `{backup_file_name}`: {err:?}");
                    continue;
                }
            };
            dtos.push(BackupDto {
                id: Box::from(backup_name),
                metadata: BackupMetadataPartialDto {
                    description: description.into_owned(),
                    created_at,
                    size_bytes: backup.size_bytes,
                    is_signed,
                    is_encrypted,
                    can_be_restored,
                },
            });
        }

        Ok(dtos)
    }

    /// Get a short-lived URL to download a backup.
    pub async fn get_download_url(
        &self,
        backup_name: &BackupFileName,
        ttl: std::time::Duration,
    ) -> Result<String, anyhow::Error> {
        // Apply max TTL from configuration.
        let ttl = ttl.clamp(std::time::Duration::ZERO, self.download_config.url_max_ttl);

        self.backup_store.download_url(&backup_name, &ttl).await
    }
}

// MARK: Restore

mod restore {
    use std::collections::HashMap;

    use anyhow::Context as _;

    use crate::{
        BackupFileName, BackupFileNameComponents, ProseBackupService,
        archiving::{ArchiveBlueprint, ExtractionSuccess, extract_archive},
        decryption,
        restoration::{RestorationSuccess, restore},
        stats::{ReadStats, StatsReader, print_stats},
        stores::ObjectStore,
        util::debug_panic,
    };

    impl<S1: ObjectStore, S2: ObjectStore> ProseBackupService<S1, S2> {
        pub async fn extract_backup<'a>(
            &self,
            backup_name: &BackupFileName,
            blueprints: &'a HashMap<u8, ArchiveBlueprint>,
        ) -> Result<ExtractionSuccess<'a>, anyhow::Error> {
            // Parse backup name first.
            // Avoids unnecessary I/O if malformed.
            let parsed_backup_name @ BackupFileNameComponents { created_at, .. } =
                BackupFileNameComponents::parse(&backup_name)?;

            let (tmp, backup_path) = self
                .download_backup_and_check_integrity(&backup_name, created_at.clone())
                .await?;

            let backup_file = std::fs::File::open(backup_path)
                .context("Could not open backup file")
                .inspect_err(debug_panic)?;

            let mut raw_read_stats = ReadStats::new();
            let backup_reader = StatsReader::new(backup_file, &mut raw_read_stats);

            // FIXME: https://docs.rs/sequoia-openpgp/2.1.0/sequoia_openpgp/parse/stream/struct.Decryptor.html
            //   > Signature verification and detection of ciphertext tampering requires processing the whole message first. Therefore, OpenPGP implementations supporting streaming operations necessarily must output unverified data. This has been a source of problems in the past. To alleviate this, we buffer the message first (up to 25 megabytes of net message data by default, see DEFAULT_BUFFER_SIZE), and verify the signatures if the message fits into our buffer. Nevertheless it is important to treat the data as unverified and untrustworthy until you have seen a positive verification. See Decryptor::message_processed for more information.
            let mut decryption_stats = ReadStats::new();
            let compressed_archive_reader = decryption::reader(
                backup_reader,
                &self.decryption_context,
                &parsed_backup_name,
                &mut decryption_stats,
            )?;

            let archive_bytes =
                zstd::Decoder::new(compressed_archive_reader).context("Cannot decompress")?;

            let mut decompression_stats = ReadStats::new();
            let archive_bytes = StatsReader::new(archive_bytes, &mut decompression_stats);

            let extraction_result =
                extract_archive(archive_bytes, blueprints).context("Backup extraction failed")?;
            drop(tmp);

            print!("\n");
            print_stats(
                &raw_read_stats,
                &decryption_stats,
                &decompression_stats,
                extraction_result.extracted_bytes_count,
            );

            Ok(extraction_result)
        }

        pub async fn restore_backup<'a>(
            &self,
            ExtractionSuccess {
                tmp_dir, blueprint, ..
            }: ExtractionSuccess<'a>,
        ) -> Result<RestorationSuccess, anyhow::Error> {
            let restore_result = restore(tmp_dir, blueprint)?;

            Ok(restore_result)
        }
    }
}

// MARK: File name serialization and deserialization

#[derive(Debug)]
#[cfg_attr(feature = "test", derive(PartialEq, Eq))]
pub(crate) struct BackupFileNameComponents<'a> {
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
        assert!(prefix.len() > 0);
        assert!(description.len() > 0);

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

#[derive(Debug, Clone, Copy)]
pub enum BackupFileNameError {
    NoExtension,
}

impl std::fmt::Display for BackupFileNameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoExtension => write!(f, "Backup file name has no extension."),
        }
    }
}

impl std::error::Error for BackupFileNameError {}

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
