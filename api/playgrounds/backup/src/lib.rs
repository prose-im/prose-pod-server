// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

extern crate aws_sdk_s3 as s3;
pub extern crate sequoia_openpgp as openpgp;
extern crate serde_json as json;

mod archiving;
mod compression;
pub mod config;
#[cfg(false)]
mod decryption;
mod encryption;
mod gpg;
mod hashing;
mod signing;
mod stats;
pub mod stores;
mod util;
#[cfg(false)]
mod verification;
mod writer_chain;

use std::borrow::Cow;

use anyhow::Context;
use bytes::Bytes;

use crate::{
    archiving::{ArchivingBlueprint, check_archiving_will_succeed},
    config::SigningConfig,
    hashing::{DigestWriter, Sha256DigestWriter},
    stores::{ObjectStore, S3Store},
};

pub use self::{
    archiving::CURRENT_BACKUP_VERSION as CURRENT_VERSION,
    config::{ArchivingConfig, BackupConfig, CompressionConfig, EncryptionConfig, HashingConfig},
    encryption::EncryptionContext,
    signing::PgpSigningContext,
};

// MARK: Service

pub type BackupService<'service, BackupStore = S3Store, CheckStore = S3Store> =
    ProseBackupService<'service, BackupStore, CheckStore>;

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
pub struct ProseBackupService<'service, BackupStore, CheckStore> {
    pub fs_root: std::path::PathBuf,

    pub archiving_config: ArchivingConfig,
    pub compression_config: CompressionConfig,
    pub hashing_config: HashingConfig,
    pub signing_config: Option<SigningConfig>,

    pub encryption_context: Option<EncryptionContext<'service>>,
    pub pgp_signing_context: Option<PgpSigningContext<'service>>,
    #[cfg(false)]
    pub decryption_helper: DecryptionHelper,
    #[cfg(false)]
    pub verification_helper: VerificationHelper<'service>,

    pub backup_store: BackupStore,
    pub check_store: CheckStore,
}

// MARK: Create backup

#[derive(Debug)]
pub struct CreateBackupOutput {
    pub backup_id: String,
    pub digest_ids: Vec<String>,
    pub signature_ids: Vec<String>,
}

impl<'service, S1: ObjectStore, S2: ObjectStore> ProseBackupService<'service, S1, S2> {
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
    ///                         │ GPG encryption enabled?
    ///                         ◇──────┐
    ///                     Yes │      │ No
    ///                    ┌────┴────┐ │
    ///                    │ Encrypt | │
    ///                    |  (GPG)  │ │
    ///                    └────┬────┘ │
    ///                         ◇──────┘
    ///      ╺━┯━━━━━━━━━━━━━━━━┿━━━━━━━━━━━━━━━━━━━┯━╸
    ///    ┌───┴────┐     ┌─────┴─────┐             │ GPG signing
    ///    | Upload |     │   Hash    |             │ enabled?
    ///    | backup |     | (SHA 256) │             ◇───────┐
    ///    |  (S3)  |     └─────┬─────┘         Yes │       │ No
    ///    └───┬────┘  ┌────────┴─────────┐     ┌───┴───┐   ◯
    ///        ◯       | Upload integrity |     │ Sign  |
    ///                |    check (S3)    |     | (GPG) │
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
        description: &str,
        prose_pod_api_data: Bytes,
    ) -> Result<CreateBackupOutput, CreateBackupError> {
        // ///
        // use openpgp::parse::Parse as _;

        // let cert = openpgp::Cert::from_file(&config.key)
        //     .context("Cannot read OpenPGP cert")
        //     .map_err(CreateBackupError::CannotEncrypt)?;
        // let helper = GpgHelper::new(cert);
        // ///

        let archiving_blueprint =
            ArchivingBlueprint::new(self.archiving_config.version, &self.fs_root)
                .context("Invalid archiving version in configuration")
                .map_err(CreateBackupError::Other)?;
        check_archiving_will_succeed(&archiving_blueprint)?;

        let created_at = std::time::SystemTime::now();

        let backup_name = BackupName::from(description, &created_at);

        let backup_file_name = match self.encryption_context {
            Some(EncryptionContext::Gpg { .. }) => backup_name.with_extension("tar.zst.gpg"),
            None => backup_name.with_extension("tar.zst"),
        };

        let upload_backup = self
            .backup_store
            .writer(&backup_file_name)
            .await
            .map_err(CreateBackupError::CannotCreateSink)?;

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
        let mut pgp_signature_writer = match self.pgp_signing_context.as_ref() {
            Some(context) => {
                let writer = context
                    .new_writer(&mut pgp_signature, created_at)
                    .map_err(CreateBackupError::CannotSign)?;
                Some(writer)
            }
            None => None,
        };

        let (writer, finalize_backup) = writer_chain::builder()
            .archive(prose_pod_api_data, &archiving_blueprint)
            .compress(&self.compression_config)
            .encrypt_if_possible(self.encryption_context.as_ref(), created_at)
            .tee(&mut digest_writer)
            .opt_tee(pgp_signature_writer.as_mut())
            .build(upload_backup)?;

        () = finalize_backup(writer)?;

        let digest = digest_writer
            .finalize()
            .map_err(CreateBackupError::HashingFailed)?;
        todo!("Upload");

        if let Some(sig_writer) = pgp_signature_writer {
            () = sig_writer
                .finalize()
                .map_err(CreateBackupError::IntegrityCheckGenerationFailed)?;
            let fixme = "Error case";

            // NOTE: OpenPGP will likely forever be the only signing protocol
            //   we support, but if we ever add one that also uses the `.sig`
            //   extension we can just use `.<protocol>.sig` for it.
            let file_name = backup_file_name.with_extension("sig");
            todo!("Upload signature");
        }

        // Upload SHA-256 digest.
        let sha_256_digest_id = {
            let file_name = backup_file_name.with_extension("sha256");

            let mut uploader = self
                .check_store
                .writer(&file_name)
                .await
                .map_err(CreateBackupError::CannotCreateSink)?;

            let mut cursor = std::io::Cursor::new(digest);
            std::io::copy(&mut cursor, &mut uploader)
                .map_err(CreateBackupError::IntegrityCheckUploadFailed)?;

            file_name.to_string()
        };

        Ok(CreateBackupOutput {
            backup_id: backup_file_name.to_string(),
            digest_ids: vec![sha_256_digest_id],
            signature_ids: todo!(),
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

    #[error("Failed computing backup integrity check: {0:?}")]
    IntegrityCheckGenerationFailed(anyhow::Error),

    #[error("Failed uploading backup integrity check: {0:?}")]
    IntegrityCheckUploadFailed(std::io::Error),

    #[error("{0:?}")]
    Other(anyhow::Error),
}

// MARK: List

#[derive(Debug)]
pub struct BackupDto {
    pub id: Box<str>,
    pub metadata: BackupMetadataPartialDto,
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
    pub description: String,
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
    pub description: String,
    pub created_at: time::UtcDateTime,
    pub is_intact: bool,
    pub is_signed: bool,
    pub signing_key: Option<openpgp::Fingerprint>,
    pub is_signature_trusted: Option<bool>,
    pub is_signature_valid: Option<bool>,
    pub is_encrypted: bool,
    pub encryption_key: Option<openpgp::Fingerprint>,
    pub is_encryption_valid: Option<bool>,
    pub is_trusted: bool,
    pub can_be_restored: bool,
}

impl<'service, S1: ObjectStore, S2: ObjectStore> ProseBackupService<'service, S1, S2> {
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

        let checks = self.check_store.list_all_after(oldest_backup).await?;

        let todo = "Make this dynamic via configuration.";
        let signing_is_mandatory = false;
        let encryption_is_mandatory = false;

        let mut dtos: Vec<BackupDto> = Vec::with_capacity(backups.len());

        for backup_file_name in backups.into_iter() {
            let is_signed = checks.contains(&format!("{backup_file_name}.sig"));
            let is_encrypted = backup_file_name.ends_with(".gpg");

            let can_be_restored = true
                && (!signing_is_mandatory || is_signed)
                && (!encryption_is_mandatory || is_encrypted);

            let BackupNameComponents {
                created_at,
                description,
                ..
            } = match parse_backup_file_name(&backup_file_name) {
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
                    is_signed,
                    is_encrypted,
                    can_be_restored,
                },
            });
        }

        Ok(dtos)
    }
}

struct BackupNameComponents<'a> {
    created_at: time::UtcDateTime,
    description: Cow<'a, str>,
    extensions: Vec<&'a str>,
}

#[derive(Clone)]
#[repr(transparent)]
pub struct BackupName(String);

impl BackupName {
    pub fn from(description: &str, created_at: &std::time::SystemTime) -> Self {
        use crate::util::SystemTimeExt as _;

        // Arbitrary safety limits.
        assert!(description.len() <= 256);
        // NOTE: Provide a default value instead of passing an empty string.
        assert!(description.len() > 0);

        // “URL encode” the backup name to get rid of spaces, emojis, etc.
        let backup_name = urlencoding::encode(description);
        // Also percent-encode `.` to prevent incorrect file extension parsing.
        debug_assert_eq!(
            urlencoding::decode("test%2Eext"),
            Ok(Cow::Borrowed("test.ext"))
        );
        let backup_name = backup_name.replace(".", "%2E");
        // Also percent-encode `/` to prevent incorrect parsing of HTTP
        // requests when a backup ID is used in the path.
        debug_assert_eq!(
            urlencoding::decode("test%2Ffoo"),
            Ok(Cow::Borrowed("test/foo"))
        );
        let backup_name = backup_name.replace("/", "%2F");

        // Unix timestamp with second precision as 10 chars covers 2001-09-09
        // to 2286-11-20 (<2001-09-09 needs 9 chars, >2286-11-20 needs 11).
        // For correctness, we’ll still format the number as 10 digits with
        // leading zeros (even if not necessary).
        let created_at = created_at.unix_timestamp();
        assert!(created_at < 99_999_999_999);
        debug_assert!(created_at > 999_999_999);
        let backup_name = format!("{created_at:010}-{backup_name}");

        Self(backup_name)
    }
}

impl BackupName {
    fn with_extension(&self, extension: &'static str) -> BackupFileName {
        debug_assert!(!extension.starts_with('.'));

        let suffix_start_idx = self.0.len();
        BackupFileName {
            value: format!("{self}.{extension}"),
            suffix_start_idx,
        }
    }
}

impl std::fmt::Display for BackupName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

impl std::fmt::Debug for BackupName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.0, f)
    }
}

#[derive(Clone)]
pub struct BackupFileName {
    value: String,

    /// Index of the dot before the file extention.
    suffix_start_idx: usize,
}

impl BackupFileName {
    /// ```rs
    /// let file_name = BackupName::from("test").with_extension("foo.bar");
    /// assert_eq!(file_name.basename(), "test");
    /// ```
    fn basename(&self) -> &str {
        &self.value[..self.suffix_start_idx]
    }

    /// ```rs
    /// let file_name = BackupName::from("test").with_extension("foo.bar");
    /// assert_eq!(file_name.extension(), "foo.bar");
    /// ```
    fn extension(&self) -> &str {
        &self.value[(self.suffix_start_idx + 1)..]
    }

    /// ```rs
    /// let file_name = BackupName::from("test").with_extension("foo.bar");
    /// let other_file_name = file_name.with_extension("baz");
    /// assert_eq!(file_name.extension(), "foo.bar.baz");
    /// ```
    fn with_extension(&self, extension: &'static str) -> Self {
        debug_assert!(!extension.starts_with('.'));

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

impl AsRef<str> for BackupFileName {
    fn as_ref(&self) -> &str {
        self.value.as_str()
    }
}

fn parse_backup_file_name<'a>(
    file_name: &'a str,
) -> Result<BackupNameComponents<'a>, anyhow::Error> {
    let Some((prefix, suffix)) = file_name.split_once('-') else {
        anyhow::bail!("File `{file_name}` is missing the timestamp prefix.");
    };

    let secs: i64 = prefix
        .parse()
        .with_context(|| format!("Could not read integer from `{prefix}`"))?;

    let created_at = match time::UtcDateTime::from_unix_timestamp(secs) {
        Ok(timestamp) => timestamp,
        Err(err) => {
            return Err(
                anyhow::Error::from(err).context("Could not parse timestamp from file name")
            );
        }
    };

    let Some((description, extensions)) = suffix.split_once('.') else {
        todo!();
    };

    let description = urlencoding::decode(description)
        .with_context(|| format!("Backup description `{description}` contains invalid UTF-8"))?;

    let extensions = extensions.split(".").collect::<Vec<_>>();

    Ok(BackupNameComponents {
        created_at,
        description,
        extensions,
    })
}

// MARK: Restore

#[cfg(false)]
mod restore {
    use std::path::Path;

    use anyhow::anyhow;

    use crate::{
        BackupNameComponents, ProseBackupService,
        archiving::{ExtractionSuccess, extract_archive},
        parse_backup_file_name,
        stats::{StatsReader, print_stats},
        stores::ObjectStore,
        verification::{IntegrityCheck, pre_validate_integrity_checks},
    };

    impl<'service, S1: ObjectStore, S2: ObjectStore> ProseBackupService<'service, S1, S2> {
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

            let BackupNameComponents {
                created_at,
                description,
                extensions,
            } = parse_backup_file_name(backup_name)?;

            // TODO: List integrity checks.
            // TODO: Read integrity checks in `Vec<u8>`s first. Avoids unnecessary
            //   read of the whole backup file if something is wrong (i.e. fetch
            //   fails, corrupted signature, no supported check…). Integrity checks
            //   are quite small so loading all in memory is better than saving to
            //   temporary files (less I/O).
            // TODO: Check signature for corruption (just in case).
            // TODO: Read backup to temporary file. It will have to be downloaded
            //   at some point anyway, and doing it this early allows us not to
            //   fetch it twice. It also allows us to easily performing integrity
            //   checks in parallel by opening multiple file descriptors (instead
            //   of writing a lot of overly complicated reading logic to reuse the
            //   same in-memory reader).
            // TODO: Run integrity checks.
            // TODO: Restore backup.

            let integrity_check_names = (self.check_store)
                .find(backup_name)
                .await
                .context("Failed listing integrity checks")?;

            let mut integrity_checks = Vec::with_capacity(integrity_check_names.len());
            for key in integrity_check_names {
                let mut reader = (self.check_store)
                    .reader(&key)
                    .await
                    .context(format!("Could not open integrity check reader for '{key}'"))?;
                let mut buf: Vec<u8> = Vec::new();
                reader.read_to_end(&mut buf);

                integrity_checks.push(IntegrityCheck {
                    name: key,
                    value: buf,
                });
            }

            pre_validate_integrity_checks(&integrity_checks)?;

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

                // FIXME: https://docs.rs/sequoia-openpgp/2.1.0/sequoia_openpgp/parse/stream/struct.Decryptor.html
                //   > Signature verification and detection of ciphertext tampering requires processing the whole message first. Therefore, OpenPGP implementations supporting streaming operations necessarily must output unverified data. This has been a source of problems in the past. To alleviate this, we buffer the message first (up to 25 megabytes of net message data by default, see DEFAULT_BUFFER_SIZE), and verify the signatures if the message fits into our buffer. Nevertheless it is important to treat the data as unverified and untrustworthy until you have seen a positive verification. See Decryptor::message_processed for more information.
                let mut decryption_stats = ReadStats::new();
                let compressed_archive_reader = if backup_name.ends_with(".gpg") {
                    if let Some(config) = self.encryption_config.gpg.as_ref() {
                        let decryptor = DecryptorBuilder::from_reader(backup_reader)
                            .context("Failed creating decryptor builder")?
                            .with_policy(config.policy.as_ref(), Some(created_at.into()), config)
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
}
