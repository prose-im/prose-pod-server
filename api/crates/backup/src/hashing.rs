// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! Hashing/checksum logic.
//!
//! When signing is enabled, we MUST still store a hash in addition to the
//! signature otherwise if signing is disabled then backups cannot be restored
//! anymore (no access to public key material to check the detached signature)!

use std::io::Write;

use crate::config::{self, HashingConfig};

pub(crate) enum DigestWriter {
    #[cfg(feature = "hashing-blake3")]
    Blake3(blake3::Hasher),

    #[cfg(feature = "hashing-sha2")]
    Sha256(sha2::Sha256),
}

pub(crate) fn digest(hashing_config: &HashingConfig) -> DigestWriter {
    // NOTE: We create only one writer in the form of an enum because:
    //   1. It does not make much sense to create multiple digests
    //   2. We ensure there is always at least one
    match hashing_config.algorithm {
        #[cfg(feature = "hashing-blake3")]
        config::HashingAlgorithm::Blake3 => DigestWriter::Blake3(blake3::Hasher::new()),
        #[cfg(feature = "hashing-sha2")]
        config::HashingAlgorithm::Sha256 => DigestWriter::Sha256(sha2::Sha256::default()),
    }
}

impl Write for DigestWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            #[cfg(feature = "hashing-blake3")]
            Self::Blake3(writer) => writer.write(buf),
            #[cfg(feature = "hashing-sha2")]
            Self::Sha256(writer) => writer.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            #[cfg(feature = "hashing-blake3")]
            Self::Blake3(writer) => writer.flush(),
            #[cfg(feature = "hashing-sha2")]
            Self::Sha256(writer) => writer.flush(),
        }
    }
}

impl DigestWriter {
    pub fn finalize(self) -> Vec<u8> {
        match self {
            #[cfg(feature = "hashing-blake3")]
            Self::Blake3(hasher) => hasher.finalize().as_bytes().to_vec(),
            #[cfg(feature = "hashing-sha2")]
            Self::Sha256(hasher) => sha2::Digest::finalize(hasher).to_vec(),
        }
    }
}
