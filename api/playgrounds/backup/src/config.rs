// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! Backups configuration.
//!
//! See [`BackupConfig`].

use figment::Figment;

/// Backup configuration.
///
/// Example full configuration (all keys have default values):
///
/// ```
/// # use prose_backup::BackupConfig;
/// # use toml::toml;
/// #
/// # let toml = toml! {
/// [compression]
/// // Zstd compression level (see <https://raw.githack.com/facebook/zstd/v1.5.7/doc/zstd_manual.html>).
/// // This value is transparently passed to the `zstd` library for forward
/// // compatibility, meaning any negative or positive value can be used
/// // although `zstd` only supports `<= 22` at the moment.
/// // The special value `0` means `zstd`’s default (currently `3`).
/// // Default is `3`.
/// zstd_compression_level = 3
///
/// [hashing]
/// // The algorithm to use when computing backup checksums.
/// // Note that only SHA-256 is supported at the moment, and we don’t plan on
/// // supporting more algorithms. This configuration key is mostly there for
/// // future-proofing.
/// algorithm = "SHA-256"
///
/// // By default, backups are not signed as it requires a secret signing key
/// // to be configured and accessible. This is where it is done.
/// [signing]
/// // `true` makes it impossible to restore a non-signed backup.
/// // Default is `true` (opt-out) as soon as you enabled a signing method.
/// mandatory = true
/// // Default is `false` (opt-in).
/// pgp.enabled = true
/// // Path to the Transferable Secret Key to use when signing new backups.
/// // This TSK MUST contain private key material suitable for signing.
/// pgp.tsk = "/path/to/prose-backup.asc"
/// // Optional. Use if you changed the primary key instead of rotating subkeys.
/// // Those SHOULD NOT contain private key material.
/// pgp.additional_trusted_issuers = ["/path/to/prose-backup-old.pub.asc"]
///
/// // By default, backups are not encrypted as it requires a secret
/// // encryption key to be configured. This is where it is done.
/// [encryption]
/// // Encryption mode. Allowed values: `"off"` (default), `"pgp"`.
/// // Also configure `encryption.<mode>` when you enable encryption.
/// mode = "pgp"
/// // Path to the Transferable Secret Key to use when encrypting new backups.
/// // This TSK MUST contain private key material suitable for storage
/// // encryption (as it’s the one used when decrypting).
/// pgp.tsk = "/path/to/prose-backup.asc"
/// // Optional. Use if you changed the primary key instead of rotating subkeys.
/// // Those MUST contain private key material.
/// pgp.additional_decryption_keys = ["/path/to/prose-backup-old.asc"]
/// // Optional. Use if you want to decrypt using private keys not present
/// // on the server (e.g. in a separate environment for forensic analysis).
/// // Those SHOULD NOT contain private key material.
/// pgp.additional_recipients = ["/path/to/other-system.pub.asc"]
///
/// // Where to store backups.
/// [storage.backups]
/// mode = "s3"
/// s3.bucket_name = "prose-backups"
///
/// // Where to store backup integrity checks.
/// [storage.checks]
/// mode = "s3"
/// // This bucket SHOULD have Object Lock enabled (see <https://docs.aws.amazon.com/AmazonS3/latest/userguide/object-lock.html>).
/// // If using Object Lock, you should also enable some bucket-level
/// // configuration to automatically cleanup objects and delete markers
/// // once the retention period ends (this library won’t do it). Because of
/// // mandatory versioning, this library can only mark objects as deleted but
/// // the underlying data will still exist (and be billed by your provider!).
/// s3.bucket_name = "prose-checks"
///
/// // Global S3 configuration.
/// // `storage.backups.s3` and `storage.checks.s3` will fallback to it.
/// // It is recommended to pass those keys via environment variables.
/// [s3]
/// region = "nbg1"
/// endpoint_url = "https://nbg1.your-objectstorage.com"
/// access_key = "574LAYIP1TR7PGYPCNV7"
/// // Pass the secret key via an environment variable.
/// # secret_key = "example"
///
/// [download]
/// // Longest allowed validity for a backup download URL. Default is 5 minutes.
/// // Uses the [ISO 8601 Duration format](https://en.wikipedia.org/wiki/ISO_8601#Durations).
/// url_max_ttl = "PT5M"
/// # };
/// #
/// # let _backup_config = BackupConfig::try_from(toml)?;
/// #
/// # Ok::<(), anyhow::Error>(())
/// ```
#[derive(Debug)]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupConfig {
    pub compression: CompressionConfig,

    pub hashing: HashingConfig,

    pub signing: SigningConfig,

    pub encryption: EncryptionConfig,

    pub storage: StorageConfig,

    pub download: DownloadConfig,

    /// Don’t mind this, it’s just there to make `deny_unknown_fields` happy
    /// (we can’t remove keys in figment).
    #[cfg(feature = "destination_s3")]
    #[doc(hidden)]
    pub s3: AlwaysNone,

    /// Don’t mind this, it’s just there to make `deny_unknown_fields` happy
    /// (we can’t remove keys in figment).
    #[cfg(feature = "destination_fs")]
    #[doc(hidden)]
    pub fs: AlwaysNone,
}

// MARK: Parsing

fn default_config_static() -> Figment {
    use figment::providers::*;
    use toml::toml;

    let mut static_defaults = toml! {
        [compression]
        zstd_compression_level = 3

        [hashing]
        algorithm = "SHA-256"

        [signing]
        pgp.enabled = false

        [encryption]
        // Note that, for consistency with `signing`, `enabled = false`
        // overrides `mode` to `"off"`. It can be useful in tests or
        // when overriding configuration with environment variables.
        mode = "off"

        [download]
        url_max_ttl = "PT5M"
    };

    #[cfg(feature = "destination_fs")]
    static_defaults.extend(toml! {
        [storage.backups]
        fs.overwrite = false
        fs.mode = 0o600

        [storage.checks]
        fs.overwrite = false
        fs.mode = 0o600
    });

    Figment::from(Serialized::defaults(static_defaults))
}

fn with_dynamic_defaults(mut figment: Figment) -> Result<Figment, figment::Error> {
    use figment::providers::*;

    let signing_enabled_opt = figment.extract_inner::<bool>("signing.enabled").ok();
    if signing_enabled_opt == Some(false) {
        figment = figment.remove("signing");
    } else {
        let signing_pgp_enabled = figment.extract_inner::<bool>("signing.pgp.enabled")?;

        if !signing_pgp_enabled {
            figment = figment.remove("signing.pgp");
        }

        let signing_should_be_mandatory =
            signing_enabled_opt.unwrap_or(false) || signing_pgp_enabled;

        figment = figment.join(Serialized::default(
            "signing.mandatory",
            &signing_should_be_mandatory,
        ));
    }

    let encryption_enabled = figment.extract_inner::<bool>("encryption.enabled").ok();
    if encryption_enabled == Some(false) {
        figment = figment.merge(Serialized::default(
            "encryption.mode",
            json::Value::String("off".to_owned()),
        ));
    }

    for mode in ["s3", "fs"] {
        if let Ok(default) = figment.extract_inner::<figment::value::Value>(mode) {
            figment = figment
                .merge(Serialized::default(
                    &format!("storage.backups.{mode}"),
                    default.clone(),
                ))
                .merge(Serialized::default(
                    &format!("storage.checks.{mode}"),
                    default,
                ))
                .remove(mode);
        }
    }

    Ok(figment)
}

// MARK: Compression

#[derive(Debug, Clone)]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompressionConfig {
    pub zstd_compression_level: i32,
}

// MARK: Hashing

#[derive(Debug, Clone)]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HashingConfig {
    pub algorithm: HashingAlgorithm,
}

#[derive(Debug, Clone, Copy)]
#[derive(serde::Deserialize)]
pub enum HashingAlgorithm {
    #[serde(rename = "SHA-256")]
    Sha256,
}

// MARK: Signing

#[derive(Debug, Clone)]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SigningConfig {
    pub mandatory: bool,

    #[serde(default, alias = "gpg")]
    pub pgp: Option<SigningPgpConfig>,
}

#[derive(Debug, Clone)]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SigningPgpConfig {
    pub tsk: std::path::PathBuf,

    #[serde(default)]
    pub additional_trusted_issuers: Vec<std::path::PathBuf>,

    /// Don’t mind this, it’s just there to make `deny_unknown_fields` happy
    /// (we can’t remove keys in figment).
    #[doc(hidden)]
    pub enabled: AlwaysNone,
}

// MARK: Encryption

#[derive(Debug, Clone)]
#[derive(serde::Deserialize)]
#[serde(tag = "mode")]
pub enum EncryptionConfig {
    #[serde(rename = "off")]
    Off,

    #[serde(rename = "pgp", alias = "gpg")]
    Pgp {
        #[serde(rename = "pgp", alias = "gpg")]
        config: EncryptionPgpConfig,
    },
}

#[derive(Debug, Clone)]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EncryptionPgpConfig {
    pub tsk: std::path::PathBuf,

    #[serde(default)]
    pub additional_decryption_keys: Vec<std::path::PathBuf>,

    #[serde(default)]
    pub additional_recipients: Vec<std::path::PathBuf>,

    /// Don’t mind this, it’s just there to make `deny_unknown_fields` happy
    /// (we can’t remove keys in figment).
    #[doc(hidden)]
    pub enabled: AlwaysNone,
}

// MARK: Storage

#[derive(Debug, Clone)]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageConfig {
    pub backups: StorageSubconfig,

    pub checks: StorageSubconfig,
}

#[derive(Debug, Clone)]
#[derive(serde::Deserialize)]
#[serde(tag = "mode")]
pub enum StorageSubconfig {
    #[cfg(feature = "destination_s3")]
    #[serde(rename = "s3")]
    S3 {
        #[serde(rename = "s3")]
        config: StorageS3Config,
    },

    #[cfg(feature = "destination_fs")]
    #[serde(rename = "fs")]
    Fs {
        #[serde(rename = "fs")]
        config: StorageFsConfig,
    },
}

#[cfg(feature = "destination_s3")]
#[derive(Debug, Clone)]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageS3Config {
    pub bucket_name: String,

    pub region: String,

    pub endpoint_url: String,

    pub access_key: String,

    pub secret_key: secrecy::SecretString,

    #[serde(default)]
    pub session_token: Option<secrecy::SecretString>,

    #[serde(default)]
    pub force_path_style: Option<bool>,

    #[serde(default, flatten)]
    pub object_lock: Option<S3ObjectLockConfig>,

    #[serde(default)]
    #[serde(with = "crate::util::serde::s3::object_lock_legal_hold_status::option")]
    pub object_lock_legal_hold_status: Option<s3::types::ObjectLockLegalHoldStatus>,
}

#[cfg(feature = "destination_s3")]
#[derive(Debug, Clone)]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3ObjectLockConfig {
    #[serde(rename = "object_lock_mode")]
    #[serde(with = "crate::util::serde::s3::object_lock_retention_mode")]
    pub mode: s3::types::ObjectLockRetentionMode,

    #[serde(rename = "object_lock_duration")]
    #[serde(with = "crate::util::serde::iso8601_duration")]
    pub duration: std::time::Duration,
}

#[cfg(feature = "destination_fs")]
#[derive(Debug, Clone)]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageFsConfig {
    pub directory: std::path::PathBuf,

    pub overwrite: bool,

    /// WARN: This must be an octal number between `0o000` and `0o777`.
    pub mode: crate::util::Octal<3>,
}

// MARK: Download

#[derive(Debug, Clone)]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DownloadConfig {
    #[serde(with = "crate::util::serde::iso8601_duration")]
    pub url_max_ttl: std::time::Duration,
}

// MARK: Constructors

impl BackupConfig {
    #[inline(always)]
    pub fn default_figment() -> Figment {
        default_config_static()
    }
}

impl TryFrom<Figment> for BackupConfig {
    type Error = anyhow::Error;

    #[inline]
    fn try_from(figment: Figment) -> Result<Self, Self::Error> {
        with_dynamic_defaults(figment)?
            .extract::<Self>()
            .map_err(anyhow::Error::new)
    }
}

#[cfg(feature = "test")]
impl TryFrom<toml::Table> for BackupConfig {
    type Error = anyhow::Error;

    fn try_from(toml: toml::Table) -> Result<Self, Self::Error> {
        use figment::providers::*;

        let figment = Self::default_figment().merge(Serialized::defaults(toml));

        Self::try_from(figment)
    }
}

#[cfg(feature = "test")]
impl TryFrom<figment::providers::Data<figment::providers::Toml>> for BackupConfig {
    type Error = anyhow::Error;

    fn try_from(
        provider: figment::providers::Data<figment::providers::Toml>,
    ) -> Result<Self, Self::Error> {
        let figment = Self::default_figment().merge(provider);

        Self::try_from(figment)
    }
}

// MARK: Helpers

trait FigmentExt {
    fn remove(self, key: &'static str) -> Self;
}

impl FigmentExt for Figment {
    fn remove(self, key: &'static str) -> Self {
        use figment::providers::Serialized;

        self.merge(Serialized::default(key, json::Value::Null))
    }
}

#[derive(Debug, Clone, Copy, Default)]
#[repr(transparent)]
pub struct AlwaysNone(Option<Impossible>);

#[derive(Debug, Clone, Copy)]
enum Impossible {}

impl<'de> serde::Deserialize<'de> for AlwaysNone {
    fn deserialize<D>(_deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::de::Deserializer<'de>,
    {
        Ok(Self(None))
    }
}

// MARK: Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_signing_defaults() {
        // NOTE(RemiBardon): I guess I have to explain this. Basically the way
        //   configuration defaults are implemented we might forget to apply a
        //   “enabled”. Here I’m leveraging Rust’s type system to ensure we
        //   update the test everytime we change the configuration schema and,
        //   consequently, keep the defaults up-to-date.
        let SigningConfig {
            mandatory: _mandatory,
            pgp,
        } = SigningConfig {
            mandatory: false,
            pgp: None,
        };

        default_config_static();

        let json = json::Map::new();

        macro_rules! assert_none {
            ($key:ident) => {
                assert_eq!(
                    json.get(stringify!($key)),
                    $key.map_or(None, |_| unreachable!())
                );
            };
        }

        assert_none!(pgp);
    }

    #[test]
    fn test_storage_errors() {
        use figment::providers::*;
        use toml::toml;

        // NOTE: In tests we need to serialize then re-parse the TOML otherwise
        //   we get errors with unpredictable output like
        //   “in playgrounds/backup/src/config.rs:417:53 toml::map::Map<alloc::string::String, toml::value::Value>”
        //   instead of “in TOML source string”. Users should never see those
        //   anyway, as configuration will come from a TOML file (not a value).
        macro_rules! backup_config {
            ($toml:tt) => {
                BackupConfig::try_from(Toml::string(&toml! $toml.to_string()))
            };
        }
        macro_rules! assert_error {
            ($res:expr, $msg:literal) => {
                assert_eq!(
                    $res.err().as_ref().map(anyhow::Error::to_string),
                    Some($msg.to_owned())
                )
            };
            (toml: $toml:tt, $msg:literal) => {
                let res = backup_config!($toml);
                assert_error!(res, $msg)
            };
        }

        // NOTE: Error message not relevant here,
        //   there is a default value for `storage`.
        let res = BackupConfig::try_from(toml::Table::new());
        assert!(matches!(res, Err(_)));

        // NOTE: Error message not relevant here,
        //   there is a default value for `storage.backups`.
        let res = backup_config!({ [storage] });
        assert!(matches!(res, Err(_)));

        assert_error!(
            toml: {
                [storage.backups]
            },
            "missing field `mode` for key \"default.storage.backups\" in TOML source string"
        );

        assert_error!(
            toml: {
                [storage.backups]
                mode = "s3"
            },
            "missing field `s3` for key \"default.storage.backups\" in TOML source string"
        );

        assert_error!(
            toml: {
                [storage.backups]
                mode = "foo"
            },
            "unknown variant: found `foo`, expected ``s3` or `fs`` for key \"default.storage.backups.mode\" in TOML source string"
        );
    }
}
