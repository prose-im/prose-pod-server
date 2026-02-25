// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! When signing is enabled, we MUST still store a hash in addition to the
//! signature otherwise if signing is disabled then backups cannot be restored
//! anymore (no access to public key material to check the detached signature)!

use std::io::Write;

pub(crate) enum HashingVariant<Sha256> {
    Sha256(Sha256),
}

pub(crate) type DigestWriter<W> = HashingVariant<Sha256DigestWriter<W>>;

impl<W: Write> Write for DigestWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Sha256(writer) => writer.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Sha256(writer) => writer.flush(),
        }
    }
}

pub(crate) type Digest = HashingVariant<sha256::Output>;

impl<W: Write> DigestWriter<W> {
    pub fn finalize(self) -> Result<Digest, anyhow::Error> {
        match self {
            Self::Sha256(writer) => writer.finalize().map(Digest::Sha256),
        }
    }
}

impl AsRef<[u8]> for Digest {
    fn as_ref(&self) -> &[u8] {
        match self {
            Self::Sha256(digest) => digest.as_ref(),
        }
    }
}

// MARK: SHA-256

pub(crate) use self::sha256::Sha256DigestWriter;
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

    pub type Output = sha2::digest::Output<Sha256>;

    impl<W: Write> Sha256DigestWriter<W> {
        pub fn finalize(mut self) -> Result<self::Output, anyhow::Error> {
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
