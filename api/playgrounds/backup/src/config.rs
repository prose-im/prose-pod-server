// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

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
/// [integrity]
/// algorithm = "SHA-256"
///
/// # By default, backups are not signed as it requires an
/// # signing key to be configured. This is where it is done.
/// [signature]
/// # Default is `false` (opt-in). Also configure `signature.mode`
/// # and `signature.<mode>` when enabling signing.
/// enabled = true
/// # `true` makes it impossible to restore a non-signed backup.
/// # Default is `false` (opt-in).
/// mandatory = true
/// # How to sign backups. Default is `"gpg"`.
/// # Mostly there to allow non-breaking changes in the future.
/// mode = "gpg"
/// # Path to the key to use when signing new backups.
/// gpg.key = "/keys/prose-backup.asc"
/// # Optional. Use if you changed the primary keys instead of
/// # rotating subkeys. Those SHOULD NOT contain private key material.
/// gpg.additional_trusted_keys = ["/keys/prose-backup-old.pub.asc"]
///
/// # By default, backups are not encrypted as it requires an
/// # encryption key to be configured. This is where it is done.
/// [encryption]
/// # Default is `false` (opt-in). Also configure `encryption.mode`
/// # and `encryption.<mode>` when enabling encryption.
/// enabled = true
/// # `true` makes it impossible to restore a non-encrypted backup.
/// # Default is `false` (opt-in).
/// mandatory = true
/// # How to encrypt backups. Default is `"gpg"`.
/// # Mostly there to allow non-breaking changes in the future.
/// mode = "gpg"
/// # Path to the key/certificate to use when encrypting new backups. This cert
/// # MUST contain private key material suitable for storage encryption.
/// gpg.key = "/keys/prose-backup.asc"
/// # Optional. Use if you want to decrypt using private keys not present
/// # on the server (e.g. in a separate environment for forensic analysis).
/// # Those SHOULD NOT contain private key material.
/// gpg.additional_encryption_keys = ["/keys/other-system.pub.asc"]
/// # Optional. Use if you changed the primary keys instead of
/// # rotating subkeys. Those MUST contain private key material.
/// gpg.additional_decryption_keys = ["/keys/prose-backup-old.asc"]
/// ```
#[derive(Debug)]
#[derive(serde::Deserialize)]
pub struct BackupConfig {
    pub archiving: ArchivingConfig,
    pub compression: CompressionConfig,
    pub intergity: IntegrityConfig,
    pub signature: SignatureConfig,
    pub encryption: EncryptionConfig,
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

// MARK: Integrity

#[derive(Debug)]
#[derive(serde::Deserialize)]
pub struct IntegrityConfig {
    pub algorithm: IntegrityAlgorithm,
}

#[non_exhaustive]
#[derive(Debug)]
#[derive(serde::Deserialize)]
pub enum IntegrityAlgorithm {
    #[serde(rename = "SHA-256")]
    Sha256,
}

// MARK: Signature

#[derive(Debug)]
#[derive(serde::Deserialize)]
pub struct SignatureConfig {
    pub enabled: bool,
    pub mandatory: bool,
    pub mode: SignatureMode,
}

#[non_exhaustive]
#[derive(Debug)]
#[derive(serde::Deserialize)]
pub enum SignatureMode {
    #[serde(rename = "gpg")]
    Gpg,
}

// MARK: Encryption

#[derive(Debug)]
#[derive(serde::Deserialize)]
pub struct EncryptionConfig {
    pub enabled: bool,
    pub mandatory: bool,
    pub mode: EncryptionMode,
    pub gpg: Option<EncryptionGpgConfig>,
}

#[non_exhaustive]
#[derive(Debug)]
#[derive(serde::Deserialize)]
pub enum EncryptionMode {
    #[serde(rename = "gpg")]
    Gpg,
}

#[derive(Debug)]
#[derive(serde::Deserialize)]
pub struct EncryptionGpgConfig {
    pub key: std::path::PathBuf,

    #[serde(default)]
    pub additional_encryption_keys: Vec<std::path::PathBuf>,

    #[serde(default)]
    pub additional_decryption_keys: Vec<std::path::PathBuf>,
}
