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
/// ```toml
/// [compression]
/// # Zstd compression level (see https://raw.githack.com/facebook/zstd/v1.5.7/doc/zstd_manual.html).
/// # This value is transparently passed to the `zstd` library for forward
/// # compatibility, meaning any negative or positive value can be used
/// # although `zstd` only supports `<= 22` at the moment.
/// # The special value `0` means `zstd`’s default (currently `3`).
/// # Default is `3`.
/// zstd_compression_level = 3
///
/// [hashing]
/// # The algorithm to use when computing backup checksums.
/// # Note that only SHA-256 is supported at the moment, and we don’t plan on
/// # supporting more algorithms. This configuration key is mostly there for
/// # future-proofing.
/// algorithm = "SHA-256"
///
/// # By default, backups are not signed as it requires a secret signing key
/// # to be configured and accessible. This is where it is done.
/// [signing]
/// # `true` makes it impossible to restore a non-signed backup.
/// # Default is `true` (opt-out) as soon as you enabled a signing method.
/// mandatory = true
/// # Default is `false` (opt-in).
/// pgp.enabled = true
/// # Path to the Transferable Secret Key to use when signing new backups.
/// # This TSK MUST contain private key material suitable for signing.
/// pgp.tsk = "/path/to/prose-backup.asc"
/// # Optional. Use if you changed the primary key instead of rotating subkeys.
/// # Those SHOULD NOT contain private key material.
/// pgp.additional_trusted_issuers = ["/path/to/prose-backup-old.pub.asc"]
///
/// # By default, backups are not encrypted as it requires a secret
/// # encryption key to be configured. This is where it is done.
/// [encryption]
/// # Encryption mode. Allowed values: `"off"` (default), `"pgp"`.
/// # Also configure `encryption.<mode>` when you enable encryption.
/// mode = "pgp"
/// # Path to the Transferable Secret Key to use when encrypting new backups.
/// # This TSK MUST contain private key material suitable for storage
/// # encryption (as it’s the one used when decrypting).
/// pgp.tsk = "/path/to/prose-backup.asc"
/// # Optional. Use if you changed the primary key instead of rotating subkeys.
/// # Those MUST contain private key material.
/// pgp.additional_decryption_keys = ["/path/to/prose-backup-old.asc"]
/// # Optional. Use if you want to decrypt using private keys not present
/// # on the server (e.g. in a separate environment for forensic analysis).
/// # Those SHOULD NOT contain private key material.
/// pgp.additional_recipients = ["/path/to/other-system.pub.asc"]
/// ```
#[derive(Debug)]
#[derive(serde::Deserialize)]
pub struct BackupConfig {
    pub compression: CompressionConfig,

    pub hashing: HashingConfig,

    pub signing: SigningConfig,

    pub encryption: EncryptionConfig,
}

// MARK: Parsing

fn default_config_static() -> Figment {
    use figment::providers::*;
    use toml::toml;

    let static_defaults = toml! {
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
    }
    .to_string();

    Figment::from(Toml::string(&static_defaults))
}

fn with_dynamic_defaults(mut figment: Figment) -> Result<Figment, figment::Error> {
    use figment::providers::*;

    let signing_enabled_opt = figment.extract_inner::<bool>("signing.enabled").ok();
    if signing_enabled_opt == Some(false) {
        figment = figment.merge(Serialized::default("signing", json::Value::Null));
    } else {
        let signing_pgp_enabled = figment.extract_inner::<bool>("signing.pgp.enabled")?;

        let signing_should_be_mandatory =
            signing_enabled_opt.unwrap_or(false) || signing_pgp_enabled;

        figment = figment.join(Serialized::default(
            "signing.mandatory",
            &signing_should_be_mandatory,
        ));

        if !signing_pgp_enabled {
            figment = figment.merge(Serialized::default("signing.pgp", json::Value::Null));
        }
    }

    let encryption_enabled = figment.extract_inner::<bool>("encryption.enabled").ok();
    if encryption_enabled == Some(false) {
        figment = figment.merge(Serialized::default(
            "encryption.mode",
            json::Value::String("off".to_owned()),
        ));
    }

    Ok(figment)
}

// MARK: Compression

#[derive(Debug, Clone)]
#[derive(serde::Deserialize)]
pub struct CompressionConfig {
    pub zstd_compression_level: i32,
}

// MARK: Hashing

#[derive(Debug, Clone)]
#[derive(serde::Deserialize)]
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
pub struct SigningConfig {
    pub mandatory: bool,

    #[serde(default, alias = "gpg")]
    pub pgp: Option<SigningPgpConfig>,
}

#[derive(Debug, Clone)]
#[derive(serde::Deserialize)]
pub struct SigningPgpConfig {
    pub tsk: std::path::PathBuf,

    #[serde(default)]
    pub additional_trusted_issuers: Vec<std::path::PathBuf>,
}

// MARK: Encryption

#[derive(Debug, Clone)]
#[derive(serde::Deserialize)]
pub struct EncryptionConfig {
    pub mode: EncryptionMode,

    #[serde(default, alias = "gpg")]
    pub pgp: Option<EncryptionPgpConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[derive(serde::Deserialize)]
pub enum EncryptionMode {
    #[serde(rename = "off")]
    Off,
    #[serde(rename = "pgp", alias = "gpg")]
    Pgp,
}

#[derive(Debug, Clone)]
#[derive(serde::Deserialize)]
pub struct EncryptionPgpConfig {
    pub tsk: std::path::PathBuf,

    #[serde(default)]
    pub additional_decryption_keys: Vec<std::path::PathBuf>,

    #[serde(default)]
    pub additional_recipients: Vec<std::path::PathBuf>,
}

// MARK: Constructors

impl BackupConfig {
    #[inline(always)]
    pub fn default_figment() -> Figment {
        default_config_static()
    }
}

impl Default for BackupConfig {
    #[inline(always)]
    fn default() -> Self {
        Self::try_from(Self::default_figment())
            .expect("Default figment should always be valid (enforced by tests)")
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

        let figment = Self::default_figment().merge(Toml::string(toml.to_string().as_str()));

        Self::try_from(figment)
    }
}

// MARK: Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_contructor() {
        // NOTE: Ensures this doesn’t panic.
        _ = BackupConfig::default();
    }

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
}
