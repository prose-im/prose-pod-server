// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::io::{self, Write};

use anyhow::{Context, anyhow};
use openpgp::parse::{Parse as _, stream::*};
use sha2::{Digest as _, Sha256};

use crate::{
    BackupService, BackupSink, BackupSource, CreateBackupError, gpg::GpgConfig,
    writer_chain::WriterChainBuilder,
};

impl<Sink: BackupSink, Source: BackupSource> BackupService<Sink, Source> {
    pub fn check_backup_integrity(
        &self,
        backup_file_name: &str,
        integrity_check_file_name: &str,
    ) -> Result<(), anyhow::Error> {
        use std::io::Read as _;

        let mut backup_reader = self
            .repository
            .backup_source
            .backup_reader(backup_file_name)
            .context("Could not open backup reader")?;

        let mut verifier = BackupVerifier::new(self.integrity_config.as_ref());
        std::io::copy(&mut backup_reader, &mut verifier.writer).context("Could not read backup")?;

        let mut integrity_check_reader = self
            .repository
            .backup_source
            .integrity_check_reader(integrity_check_file_name)
            .context("Could not open integrity check reader")?;
        let mut integrity_check: Vec<u8> = Vec::new();
        integrity_check_reader
            .read_to_end(&mut integrity_check)
            .context("Could not read integrity check")?;

        verifier.verify(&integrity_check)
    }
}

// MARK: Verifier

pub type BackupVerifier<'a> = ProseBackupVerifier<'a>;

pub(crate) struct ProseBackupVerifier<'a> {
    pub(crate) writer: IntegrityChecker<'a>,
}

impl<'a> BackupVerifier<'a> {
    pub(crate) fn new(integrity_config: Option<&'a IntegrityConfig>) -> Self {
        Self {
            writer: IntegrityChecker::new(integrity_config),
        }
    }

    pub(crate) fn verify(self, integrity_check: &Vec<u8>) -> Result<(), anyhow::Error> {
        self.writer.verify(integrity_check)
    }
}

// MARK: Integrity

impl<M, F> WriterChainBuilder<M, F> {
    pub(crate) fn integrity_check<'a, W, OuterWriter>(
        self,
        config: Option<&IntegrityConfig>,
    ) -> WriterChainBuilder<
        impl FnOnce(W) -> Result<OuterWriter, CreateBackupError>,
        impl FnOnce(OuterWriter) -> Result<(), CreateBackupError>,
    >
    where
        W: Write + Send + Sync + 'a,
        M: FnOnce(IntegrityWriter<'a, W>) -> Result<OuterWriter, CreateBackupError>,
        F: FnOnce(OuterWriter) -> Result<IntegrityWriter<'a, W>, CreateBackupError>,
    {
        let Self { make, finalize, .. } = self;

        WriterChainBuilder {
            make: move |writer| {
                let writer = if let Some(integrity_config) = config {
                    IntegrityWriter::Signature(
                        PgpSignatureWriter::new(writer, &integrity_config.cert)
                            .map_err(CreateBackupError::CannotComputeIntegrityCheck)?,
                    )
                } else {
                    IntegrityWriter::Hash {
                        hasher: Sha256::new(),
                        writer,
                    }
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

pub type IntegrityConfig = GpgConfig;

/// OpenPGP signature writer.
pub struct PgpSignatureWriter<'a> {
    signer: openpgp::serialize::stream::Signer<'a>,
}

impl<'a> PgpSignatureWriter<'a> {
    fn new<W>(writer: W, cert: &openpgp::Cert) -> Result<Self, anyhow::Error>
    where
        W: Write + Send + Sync + 'a,
    {
        use openpgp::serialize::stream::{Message, Signer};

        let keypair = cert
            .keys()
            .with_policy(&openpgp::policy::StandardPolicy::new(), None)
            .secret()
            .for_signing()
            .next()
            .context("No signing key")?
            .key()
            .clone()
            .into_keypair()?;

        let message = Message::new(writer);
        let signer = Signer::new(message, keypair)?.detached();

        Ok(Self { signer })
    }
}

impl<'a> PgpSignatureWriter<'a> {
    fn finalize(self) -> Result<(), anyhow::Error> {
        self.signer.build()?.finalize()
    }
}

impl<'a> Write for PgpSignatureWriter<'a> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.signer.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.signer.flush()
    }
}

pub enum IntegrityWriter<'a, W: Write> {
    /// Integrity only.
    Hash { hasher: Sha256, writer: W },
    /// Integrity and authenticity using an OpenPGP key.
    Signature(PgpSignatureWriter<'a>),
}

impl<'a, W: Write> IntegrityWriter<'a, W> {
    pub fn finalize(self) -> Result<(), anyhow::Error> {
        match self {
            Self::Hash { hasher, mut writer } => {
                let hash = hasher.finalize();
                writer.write_all(&hash).context("Could not write hash")
            }
            Self::Signature(signer) => signer.finalize(),
        }
    }
}

impl<'a, W: Write> Write for IntegrityWriter<'a, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            IntegrityWriter::Hash { hasher, .. } => hasher.write(buf),
            IntegrityWriter::Signature(writer) => writer.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            IntegrityWriter::Hash { hasher, .. } => hasher.flush(),
            IntegrityWriter::Signature(writer) => writer.flush(),
        }
    }
}

pub enum IntegrityChecker<'a> {
    /// Integrity only.
    Hash { hasher: Sha256 },
    /// Integrity and authenticity using an OpenPGP key.
    Signature {
        helper: &'a IntegrityConfig,
        buffer: std::io::Cursor<Vec<u8>>,
    },
}

impl<'a> IntegrityChecker<'a> {
    fn new(helper: Option<&'a IntegrityConfig>) -> Self {
        match helper {
            Some(helper) => Self::Signature {
                helper,
                buffer: std::io::Cursor::new(Vec::new()),
            },
            None => Self::Hash {
                hasher: Sha256::new(),
            },
        }
    }
}

impl<'a> Write for IntegrityChecker<'a> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Self::Hash { hasher, .. } => hasher.write(buf),
            Self::Signature { buffer, .. } => buffer.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Hash { hasher, .. } => hasher.flush(),
            Self::Signature { buffer, .. } => buffer.flush(),
        }
    }
}

impl<'a> IntegrityChecker<'a> {
    fn verify(self, integrity_check: &[u8]) -> Result<(), anyhow::Error> {
        match self {
            Self::Hash { hasher } => {
                let hash = hasher.finalize();
                if integrity_check == hash.to_vec().as_slice() {
                    Ok(())
                } else {
                    Err(anyhow!("Invalid hash."))
                }
            }

            Self::Signature { helper, mut buffer } => {
                let mut verifier = DetachedVerifierBuilder::from_bytes(&integrity_check)?
                    .with_policy(helper.policy.as_ref(), None, helper)
                    .context("Could not build detached signature verifier")?;

                verifier
                    .verify_reader(&mut buffer)
                    .context("Signature verification failed")?;

                Ok(())
            }
        }
    }
}
