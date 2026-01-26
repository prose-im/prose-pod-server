// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! When signing is enabled, we MUST still store a hash in addition to the
//! signature otherwise if signing is disabled then backups cannot be restored
//! anymore (no access to public key material to check the detached signature)!

use std::io::{self, Write};

use anyhow::{Context, anyhow};
use sha2::{Digest as _, Sha256};

use crate::{CreateBackupError, ObjectStore, ProseBackupService, writer_chain::WriterChainBuilder};

pub(crate) enum IntegrityWriterBuilder {
    Sha256,
}

impl<M, F> WriterChainBuilder<M, F> {
    pub(crate) fn digest<'w, W, OuterWriter>(
        self,
        config: &IntegrityWriterBuilder,
    ) -> WriterChainBuilder<
        impl FnOnce(W) -> Result<OuterWriter, CreateBackupError>,
        impl FnOnce(OuterWriter) -> Result<(), CreateBackupError>,
    >
    where
        W: Write + Send + Sync + 'w,
        M: FnOnce(IntegrityWriter<'w, W>) -> Result<OuterWriter, CreateBackupError>,
        F: FnOnce(OuterWriter) -> Result<IntegrityWriter<'w, W>, CreateBackupError>,
    {
        let Self { make, finalize, .. } = self;

        WriterChainBuilder {
            make: move |writer| {
                let writer = match config {
                    IntegrityWriterBuilder::Sha256 => IntegrityWriter::Sha256 {
                        hasher: Sha256::new(),
                        writer,
                    },
                };

                make(writer)
            },

            finalize: |writer: OuterWriter| {
                let writer: IntegrityWriter<_> = finalize(writer)?;

                let res = writer
                    .finalize()
                    .map_err(CreateBackupError::IntegrityCheckGenerationFailed);

                res
            },
        }
    }
}

// MARK: Integrity

#[non_exhaustive]
pub enum IntegrityWriter<W: Write> {
    Sha256 { hasher: Sha256, writer: W },
}

impl<W: Write> IntegrityWriter<W> {
    pub fn finalize(self) -> Result<(), anyhow::Error> {
        match self {
            Self::Sha256 { hasher, mut writer } => {
                let hash = hasher.finalize();
                writer.write_all(&hash).context("Could not write hash")
            }
        }
    }
}

impl<W: Write> Write for IntegrityWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Self::Sha256 { hasher, .. } => hasher.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Sha256 { hasher, .. } => hasher.flush(),
        }
    }
}
