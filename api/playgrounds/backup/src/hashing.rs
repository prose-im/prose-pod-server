// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! When signing is enabled, we MUST still store a hash in addition to the
//! signature otherwise if signing is disabled then backups cannot be restored
//! anymore (no access to public key material to check the detached signature)!

// MARK: SHA-256

pub use self::sha256::Sha256DigestWriter;
mod sha256 {
    use std::io::{self, Write};

    use anyhow::Context as _;
    use sha2::{Digest as _, Sha256};

    pub struct Sha256DigestWriter<W> {
        hasher: Sha256,
        writer: W,
    }

    impl Sha256DigestWriter<Vec<u8>> {
        pub fn new() -> Self {
            Self {
                hasher: Sha256::new(),
                writer: Vec::new(),
            }
        }
    }

    impl<W: Write> Sha256DigestWriter<W> {
        pub fn finalize(mut self) -> Result<sha2::digest::Output<Sha256>, anyhow::Error> {
            let hash = self.hasher.finalize();
            self.writer
                .write_all(&hash)
                .context("Could not write hash")?;
            Ok(hash)
        }
    }

    impl<W: Write> Write for Sha256DigestWriter<W> {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.hasher.write(buf)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.hasher.flush()
        }
    }
}
