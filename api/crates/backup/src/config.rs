// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! Backups configuration.
//!
//! See [`BackupConfig`].

use std::collections::HashMap;

use figment::Figment;

pub use crate::util::BytesAmount;

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
/// // The algorithm to use when compressing backups.
/// // Possible values: `"zstd"` (default), `"off"`.
/// // Note that using `"off"` is highly discouraged as it would result in
/// // larger backups.
/// algorithm = "zstd"
/// // Zstandard compression level (see <https://raw.githack.com/facebook/zstd/v1.5.7/doc/zstd_manual.html>).
/// // This value is transparently passed to the `zstd` library for forward
/// // compatibility, meaning any negative or positive value can be used
/// // although `zstd` only supports `<= 22` at the moment.
/// // The special value `0` means `zstd`’s default (currently `3`).
/// // Default is `3`.
/// zstd.compression_level = 3
///
/// [hashing]
/// // The algorithm to use when computing backup checksums.
/// // Possible values: `"BLAKE3"` (default), `"SHA-256"`.
/// algorithm = "BLAKE3"
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
/// // Configure passphrases via environment variables.
/// // You can use the certificate’s fingerprint or the subkey’s.
/// // pgp.passphrases.<fingerprint_1> = "example"
/// // pgp.passphrases.<fingerprint_2> = "example"
/// // Optional. Use if you changed the primary key instead of rotating subkeys.
/// // Those SHOULD NOT contain private key material.
/// pgp.additional_trusted_issuers = ["/path/to/prose-backup-old.pub.asc"]
///
/// // By default, backups are not encrypted as it requires a secret
/// // encryption key to be configured. This is where it is done.
/// [encryption]
/// // Encryption mode. Possible values: `"off"` (default), `"pgp"`.
/// // Also configure `encryption.<mode>` when you enable encryption.
/// mode = "pgp"
/// // Path to the Transferable Secret Key to use when encrypting new backups.
/// // This TSK MUST contain private key material suitable for storage
/// // encryption (as it’s the one used when decrypting).
/// pgp.tsk = "/path/to/prose-backup.asc"
/// // Configure passphrases via environment variables.
/// // You can use the certificate’s fingerprint or the subkey’s.
/// // pgp.passphrases.<fingerprint_1> = "example"
/// // pgp.passphrases.<fingerprint_2> = "example"
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
/// provider = "s3"
/// s3.bucket_name = "backups"
/// s3.prefix = "prose/"
///
/// // Where to store backup integrity checks.
/// [storage.checks]
/// provider = "s3"
/// // This bucket SHOULD have Object Lock enabled (see <https://docs.aws.amazon.com/AmazonS3/latest/userguide/object-lock.html>).
/// // If using Object Lock, you should also enable some bucket-level
/// // configuration to automatically cleanup objects and delete markers
/// // once the retention period ends (this library won’t do it). Because of
/// // mandatory versioning, this library can only mark objects as deleted but
/// // the underlying data will still exist (and be billed by your provider!).
/// s3.bucket_name = "checks"
/// s3.prefix = "prose/"
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

    pub caching: CachingConfig,

    /// Don’t mind this, it’s just there to make `deny_unknown_fields` happy
    /// (we can’t remove keys in `figment`).
    #[doc(hidden)]
    pub pgp: AlwaysNone,

    /// Don’t mind this, it’s just there to make `deny_unknown_fields` happy
    /// (we can’t remove keys in `figment`).
    #[cfg(feature = "provider_s3")]
    #[doc(hidden)]
    pub s3: AlwaysNone,

    /// Don’t mind this, it’s just there to make `deny_unknown_fields` happy
    /// (we can’t remove keys in `figment`).
    #[cfg(feature = "provider_fs")]
    #[doc(hidden)]
    pub fs: AlwaysNone,
}

// MARK: Parsing

pub fn default_config_static() -> toml::Table {
    use toml::toml;

    #[cfg(feature = "blake3")]
    let default_hashing_algorithm = "BLAKE3";
    #[cfg(all(not(feature = "blake3"), feature = "sha2"))]
    let default_hashing_algorithm = "SHA-256";

    let cache_dir = tempfile::env::temp_dir().display().to_string();

    #[allow(unused_mut)]
    let mut static_defaults = toml! {
        [compression]
        // This isn’t the default in most cases, it’s just a fallback.
        algorithm = "off"

        [hashing]
        algorithm = default_hashing_algorithm

        [signing]
        pgp.enabled = false

        [encryption]
        // Note that, for consistency with `signing`, `enabled = false`
        // overrides `mode` to `"off"`. It can be useful in tests or
        // when overriding configuration with environment variables.
        mode = "off"

        [download]
        url_max_ttl = "PT5M"

        [caching]
        cache_dir = cache_dir
    };

    #[cfg(feature = "zstd")]
    static_defaults.extend(toml! {
        [compression]
        algorithm = "zstd"
        zstd.compression_level = 3
    });

    #[cfg(feature = "provider_fs")]
    static_defaults.extend(toml! {
        [storage.backups]
        fs.overwrite = false
        fs.mode = 0o600

        [storage.checks]
        fs.overwrite = false
        fs.mode = 0o600
    });

    static_defaults
}

/// NOTE: `figment::Error` is at least 208 bytes. clippy suggested boxing.
pub fn with_dynamic_defaults(mut figment: Figment) -> Result<Figment, Box<figment::Error>> {
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

    for provider in ["s3", "fs"] {
        // Move e.g. `s3` to `storage.s3`.
        if let Ok(default) = figment.extract_inner::<figment::value::Value>(provider) {
            figment = figment
                .merge(Serialized::default(
                    &format!("storage.{provider}"),
                    default.clone(),
                ))
                .remove(provider);
        }

        // Move e.g. `storage.s3` to `storage.(backups|checks).s3`.
        if let Ok(default) =
            figment.extract_inner::<figment::value::Value>(&format!("storage.{provider}"))
        {
            figment = figment
                .merge(Serialized::default(
                    &format!("storage.backups.{provider}"),
                    default.clone(),
                ))
                .merge(Serialized::default(
                    &format!("storage.checks.{provider}"),
                    default,
                ))
                .remove(&format!("storage.{provider}"));
        }
    }

    // Move `storage.provider` to `storage.(backups|checks).provider`.
    if let Ok(default) = figment.extract_inner::<figment::value::Value>("storage.provider") {
        figment = figment
            .merge(Serialized::default(
                "storage.backups.provider",
                default.clone(),
            ))
            .merge(Serialized::default("storage.checks.provider", default))
            .remove("storage.provider");
    }

    // Move `pgp` to `(encryption|signing).pgp`.
    if let Ok(default) = figment.extract_inner::<figment::value::Value>("pgp") {
        figment = figment
            .merge(Serialized::default("encryption.pgp", default.clone()))
            .merge(Serialized::default("signing.pgp", default))
            .remove("pgp");
    }

    Ok(figment)
}

// MARK: Compression

#[derive(Debug, Clone)]
#[derive(serde::Deserialize)]
#[serde(tag = "algorithm")]
pub enum CompressionConfig {
    #[cfg(feature = "zstd")]
    #[serde(rename = "zstd", alias = "Zstandard")]
    Zstd {
        #[serde(rename = "zstd")]
        config: CompressionZstdConfig,
    },

    #[serde(rename = "off", alias = "none")]
    Off,
}

#[derive(Debug, Clone)]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompressionZstdConfig {
    pub compression_level: i32,
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
    #[cfg(feature = "blake3")]
    #[serde(rename = "BLAKE3")]
    Blake3,

    #[cfg(feature = "sha2")]
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
    #[serde(with = "crate::util::serde::pgp::passphrases")]
    pub passphrases: HashMap<openpgp::Fingerprint, openpgp::crypto::Password>,

    #[serde(default)]
    pub additional_trusted_issuers: Vec<std::path::PathBuf>,

    /// Don’t mind this, it’s just there to make `deny_unknown_fields` happy
    /// (we can’t remove keys in `figment`).
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

    #[serde(rename = "pgp", alias = "OpenPGP", alias = "gpg")]
    Pgp {
        #[serde(rename = "pgp", alias = "gpg")]
        config: EncryptionPgpConfig,
    },
}

#[derive(Debug, Clone)]
#[serde_with::serde_as]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EncryptionPgpConfig {
    pub tsk: std::path::PathBuf,

    #[serde(default)]
    #[serde(with = "crate::util::serde::pgp::passphrases")]
    pub passphrases: HashMap<openpgp::Fingerprint, openpgp::crypto::Password>,

    #[serde(default)]
    pub additional_decryption_keys: Vec<std::path::PathBuf>,

    #[serde(default)]
    pub additional_recipients: Vec<std::path::PathBuf>,

    /// Don’t mind this, it’s just there to make `deny_unknown_fields` happy
    /// (we can’t remove keys in `figment`).
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

    /// Don’t mind this, it’s just there to make `deny_unknown_fields` happy
    /// (we can’t remove keys in `figment`).
    #[doc(hidden)]
    pub provider: AlwaysNone,

    /// Don’t mind this, it’s just there to make `deny_unknown_fields` happy
    /// (we can’t remove keys in `figment`).
    #[cfg(feature = "provider_s3")]
    #[doc(hidden)]
    pub s3: AlwaysNone,

    /// Don’t mind this, it’s just there to make `deny_unknown_fields` happy
    /// (we can’t remove keys in `figment`).
    #[cfg(feature = "provider_fs")]
    #[doc(hidden)]
    pub fs: AlwaysNone,
}

#[derive(Debug, Clone)]
#[derive(serde::Deserialize)]
#[serde(tag = "provider")]
pub enum StorageSubconfig {
    #[cfg(feature = "provider_s3")]
    #[serde(rename = "s3", alias = "S3")]
    S3 {
        #[serde(rename = "s3")]
        config: StorageS3Config,
    },

    #[cfg(feature = "provider_fs")]
    #[serde(rename = "fs")]
    Fs {
        #[serde(rename = "fs")]
        config: StorageFsConfig,
    },
}

#[cfg(feature = "provider_s3")]
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
    pub prefix: Option<String>,

    #[serde(default)]
    pub force_path_style: Option<bool>,

    #[serde(default, flatten)]
    pub object_lock: Option<S3ObjectLockConfig>,

    #[serde(default)]
    #[serde(with = "crate::util::serde::s3::object_lock_legal_hold_status::option")]
    pub object_lock_legal_hold_status: Option<s3::types::ObjectLockLegalHoldStatus>,
}

#[cfg(feature = "provider_s3")]
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

#[cfg(feature = "provider_fs")]
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

// MARK: Caching

#[derive(Debug, Clone)]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CachingConfig {
    pub cache_dir: std::path::PathBuf,

    #[serde(default)]
    pub max_backup_cache_size: Option<BytesAmount>,
}

// MARK: Constructors

impl BackupConfig {
    #[inline]
    pub fn default_figment() -> Figment {
        use figment::providers::Serialized;

        Figment::from(Serialized::defaults(default_config_static()))
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
    fn remove(self, key: &str) -> Self;
}

impl FigmentExt for Figment {
    fn remove(self, key: &str) -> Self {
        use figment::providers::Serialized;

        self.merge(Serialized::default(key, json::Value::Null))
    }
}

#[derive(Clone, Copy, Default)]
#[repr(transparent)]
pub struct AlwaysNone(());

impl std::fmt::Debug for AlwaysNone {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("/* irrelevant */")
    }
}

impl<'de> serde::Deserialize<'de> for AlwaysNone {
    fn deserialize<D>(_deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::de::Deserializer<'de>,
    {
        Ok(Self(()))
    }
}

// MARK: Tests

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::*;

    #[test]
    #[cfg(feature = "test")]
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
            ($res:expr, $expected:expr) => {
                assert_eq!(
                    $res.err().as_ref().map(anyhow::Error::to_string),
                    Some($expected.to_owned())
                )
            };
            (toml: $toml:tt, $expected:expr) => {{
                let res = backup_config!($toml);
                assert_error!(res, $expected)
            }};
        }

        // NOTE: Error message not relevant here,
        //   there is a default value for `storage`.
        let res = BackupConfig::try_from(toml::Table::new());
        assert!(res.is_err());

        // NOTE: Error message not relevant here,
        //   there is a default value for `storage.backups`.
        let res = backup_config!({ [storage] });
        assert!(res.is_err());

        assert_error!(
            toml: {
                [storage.backups]
            },
            "missing field `provider` for key \"default.storage.backups\" in TOML source string"
        );

        #[cfg(feature = "provider_s3")]
        let (provider, missing_field, supported) = ("s3", "s3", "one of `S3`, `s3`, `fs`");
        #[cfg(not(feature = "provider_s3"))]
        let (provider, missing_field, supported) = ("fs", "directory", "`fs`");

        assert_error!(
            toml: {
                [storage.backups]
                provider = provider
            },
            format!("missing field `{missing_field}` for key \"default.storage.backups\" in TOML source string")
        );

        assert_error!(
            toml: {
                [storage.backups]
                provider = "foo"
            },
            format!("unknown variant: found `foo`, expected `{supported}` for key \"default.storage.backups.provider\" in TOML source string")
        );
    }
}
