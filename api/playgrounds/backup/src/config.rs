// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use figment::Figment;

/// Example full configuration (all keys have default values):
///
/// ```toml
/// [archiving]
/// # Default is `1`. No need to override this, it’s mostly there for
/// # integration testing or in case a breaking change is released by mistake.
/// version = 1
///
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
/// algorithm = "SHA-256"
///
/// # By default, backups are not signed as it requires an
/// # signing key to be configured. This is where it is done.
/// [signing]
/// # Default is `false` (opt-in). Also configure `signing.mode`
/// # and `signing.<mode>` when enabling signing.
/// enabled = true
/// # `true` makes it impossible to restore a non-signed backup.
/// # Default is `false` (opt-in).
/// mandatory = true
/// # Default is `true`. Use only if you need a global override
/// # (e.g. in tests).
/// pgp.enabled = true
/// # Path to the key to use when signing new backups.
/// pgp.key = "/keys/prose-backup.asc"
/// # Optional. Use if you changed the primary keys instead of
/// # rotating subkeys. Those SHOULD NOT contain private key material.
/// pgp.additional_trusted_keys = ["/keys/prose-backup-old.pub.asc"]
///
/// # By default, backups are not encrypted as it requires an
/// # encryption key to be configured. This is where it is done.
/// [encryption]
/// # Default is `false` (opt-in). Also configure `encryption.mode`
/// # and `encryption.<mode>` when enabling encryption.
/// enabled = true
/// # How to encrypt backups. Default is `"pgp"`.
/// # Mostly there to allow non-breaking changes in the future.
/// mode = "pgp"
/// # Path to the key/certificate to use when encrypting new backups. This cert
/// # MUST contain private key material suitable for storage encryption.
/// pgp.key = "/keys/prose-backup.asc"
/// # Optional. Use if you want to decrypt using private keys not present
/// # on the server (e.g. in a separate environment for forensic analysis).
/// # Those SHOULD NOT contain private key material.
/// pgp.additional_encryption_keys = ["/keys/other-system.pub.asc"]
/// # Optional. Use if you changed the primary keys instead of
/// # rotating subkeys. Those MUST contain private key material.
/// pgp.additional_decryption_keys = ["/keys/prose-backup-old.asc"]
/// ```
#[derive(Debug)]
#[derive(serde::Deserialize)]
pub struct BackupConfig {
    pub archiving: ArchivingConfig,

    pub compression: CompressionConfig,

    pub hashing: HashingConfig,

    pub signing: SigningConfig,

    pub encryption: EncryptionConfig,
}

// MARK: Parsing

pub fn default_config_static() -> Figment {
    use figment::providers::*;
    use toml::toml;

    let static_defaults = toml! {
        [archiving]
        version = 1

        [compression]
        zstd_compression_level = 3

        [hashing]
        algorithm = "SHA-256"

        [signing]
        enabled = false
        mandatory = false

        [encryption]
        enabled = false
        mode = "pgp"
    }
    .to_string();

    Figment::from(Toml::string(&static_defaults))
}

pub fn with_dynamic_defaults(mut figment: Figment) -> Result<Figment, figment::Error> {
    use figment::providers::*;

    let signing_enabled = figment.extract_inner::<bool>("signing.enabled")?;

    figment = figment.join(Serialized::default("signing.pgp.enabled", &signing_enabled));

    let signing_pgp_enabled = figment.extract_inner::<bool>("signing.pgp.enabled")?;
    if !signing_pgp_enabled {
        figment = figment.merge(Serialized::default("signing.pgp", json::Value::Null));
    }

    Ok(figment)
}

// MARK: Archiving

#[derive(Debug)]
#[derive(serde::Deserialize)]
pub struct ArchivingConfig {
    pub version: u8,
}

// MARK: Compression

#[derive(Debug)]
#[derive(serde::Deserialize)]
pub struct CompressionConfig {
    pub zstd_compression_level: i32,
}

// MARK: Hashing

#[derive(Debug)]
#[derive(serde::Deserialize)]
pub struct HashingConfig {
    pub algorithm: HashingAlgorithm,
}

#[derive(Debug)]
#[derive(serde::Deserialize)]
pub enum HashingAlgorithm {
    #[serde(rename = "SHA-256")]
    Sha256,
}

// MARK: Signing

#[derive(Debug)]
#[derive(serde::Deserialize)]
pub struct SigningConfig {
    pub mandatory: bool,

    #[serde(default, alias = "gpg")]
    pub pgp: Option<SigningPgpConfig>,
}

#[derive(Debug)]
#[derive(serde::Deserialize)]
pub struct SigningPgpConfig {
    pub key: std::path::PathBuf,

    #[serde(default)]
    pub additional_encryption_keys: Vec<std::path::PathBuf>,

    #[serde(default)]
    pub additional_decryption_keys: Vec<std::path::PathBuf>,
}

// MARK: Encryption

#[derive(Debug)]
#[derive(serde::Deserialize)]
pub struct EncryptionConfig {
    pub enabled: bool,

    pub mode: EncryptionMode,

    #[serde(default, alias = "gpg")]
    pub pgp: Option<EncryptionPgpConfig>,
}

#[derive(Debug)]
#[derive(serde::Deserialize)]
pub enum EncryptionMode {
    #[serde(rename = "pgp", alias = "gpg")]
    Pgp,
}

#[derive(Debug)]
#[derive(serde::Deserialize)]
pub struct EncryptionPgpConfig {
    pub key: std::path::PathBuf,

    #[serde(default)]
    pub additional_encryption_keys: Vec<std::path::PathBuf>,

    #[serde(default)]
    pub additional_decryption_keys: Vec<std::path::PathBuf>,
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
}
