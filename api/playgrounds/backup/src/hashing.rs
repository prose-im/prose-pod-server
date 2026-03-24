// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! Hashing/checksum logic.
//!
//! When signing is enabled, we MUST still store a hash in addition to the
//! signature otherwise if signing is disabled then backups cannot be restored
//! anymore (no access to public key material to check the detached signature)!

use composable_stream::ComposableStreamBuilder;
use std::io::Write;

use crate::CreateBackupError;
use crate::config::{self, HashingConfig};

pub(crate) enum DigestWriter<W> {
    Sha256(Sha256DigestWriter<W>),
}

pub(crate) fn digest<W>(
    hashing_config: &HashingConfig,
) -> ComposableStreamBuilder<impl FnOnce(W) -> Result<DigestWriter<W>, CreateBackupError>>
where
    W: Write + Send + Sync,
{
    ComposableStreamBuilder {
        // NOTE: We create only one writer in the form of an enum because:
        //   1. It does not make much sense to create multiple digests
        //   2. We ensure there is always at least one
        make: move |writer: W| match hashing_config.algorithm {
            config::HashingAlgorithm::Sha256 => {
                Ok(DigestWriter::Sha256(Sha256DigestWriter::new(writer)))
            }
        },
    }
}

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

impl<W: Write> DigestWriter<W> {
    #[cfg_attr(not(coverage), allow(unused_mut))]
    pub fn finalize(mut self) -> Result<W, anyhow::Error> {
        #[cfg(coverage)]
        self.flush()?;

        match self {
            Self::Sha256(writer) => writer.finalize(),
        }
    }
}

// MARK: SHA-256

use self::sha256::*;
mod sha256 {
    use std::io::{self, Write};

    use anyhow::Context as _;
    use sha2::{Digest as _, Sha256};

    pub(crate) struct Sha256DigestWriter<W> {
        hasher: Sha256,
        writer: W,
    }

    impl<W> Sha256DigestWriter<W> {
        pub fn new(writer: W) -> Self {
            Self {
                hasher: Sha256::new(),
                writer,
            }
        }
    }

    impl<W: Write> Sha256DigestWriter<W> {
        pub fn finalize(mut self) -> Result<W, anyhow::Error> {
            self.flush()?;

            let hash = self.hasher.finalize();
            self.writer
                .write_all(&hash)
                .context("Could not write hash")?;
            Ok(self.writer)
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
