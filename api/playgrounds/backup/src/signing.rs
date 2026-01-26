// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! When signing is enabled, we MUST still store a hash in addition to the
//! signature otherwise if signing is disabled then backups cannot be restored
//! anymore (no access to public key material to check the detached signature)!

use std::io::{self, Write};

use anyhow::Context as _;
use openpgp::parse::stream::*;

use crate::{
    CreateBackupError, ObjectStore, ProseBackupService,
    signing::gpg::{PgpSignatureWriter, PgpVerificationHelper},
    writer_chain::WriterChainBuilder,
};

pub(crate) enum SignatureWriterBuilder<'a> {
    Gpg(&'a PgpVerificationHelper),
}

impl<'s, S1, S2> ProseBackupService<'s, S1, S2>
where
    S1: ObjectStore,
    S2: ObjectStore,
{
    pub async fn check_signature(
        &self,
        backup_file_name: &str,
        signature_file_name: &str,
    ) -> Result<(), anyhow::Error> {
        use std::io::Read as _;

        let mut backup_reader = self
            .backup_store
            .reader(backup_file_name)
            .await
            .context("Could not open backup reader")?;

        let mut verifier = BackupVerifier::new(self.integrity_config.as_ref());
        std::io::copy(&mut backup_reader, &mut verifier.writer).context("Could not read backup")?;

        let mut integrity_check_reader = self
            .check_store
            .reader(signature_file_name)
            .await
            .context("Could not open integrity check reader")?;
        let mut integrity_check: Vec<u8> = Vec::new();
        integrity_check_reader
            .read_to_end(&mut integrity_check)
            .context("Could not read integrity check")?;

        verifier.verify(&integrity_check)?;

        tracing::debug!("Integrity check passed ({signature_file_name}).");

        Ok(())
    }
}

impl<M, F> WriterChainBuilder<M, F> {
    pub(crate) fn sign<'w, W, OuterWriter>(
        self,
        config: &SignatureWriterBuilder,
    ) -> WriterChainBuilder<
        impl FnOnce(W) -> Result<OuterWriter, CreateBackupError>,
        impl FnOnce(OuterWriter) -> Result<(), CreateBackupError>,
    >
    where
        W: Write + Send + Sync + 'w,
        M: FnOnce(SignatureWriter<'w>) -> Result<OuterWriter, CreateBackupError>,
        F: FnOnce(OuterWriter) -> Result<SignatureWriter<'w>, CreateBackupError>,
    {
        let Self { make, finalize, .. } = self;

        WriterChainBuilder {
            make: move |writer| {
                let writer = match config {
                    SignatureWriterBuilder::Gpg(cert) => SignatureWriter::Pgp(
                        PgpSignatureWriter::new(writer, cert)
                            .map_err(CreateBackupError::CannotComputeIntegrityCheck)?,
                    ),
                };

                make(writer)
            },

            finalize: |writer: OuterWriter| {
                let writer: SignatureWriter = finalize(writer)?;

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
pub enum SignatureWriter<'a> {
    Pgp(PgpSignatureWriter<'a>),
}

impl<'a> SignatureWriter<'a> {
    pub fn finalize(self) -> Result<(), anyhow::Error> {
        match self {
            Self::Pgp(signer) => signer.finalize(),
        }
    }
}

impl<'a> Write for SignatureWriter<'a> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            SignatureWriter::Pgp(writer) => writer.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            SignatureWriter::Pgp(writer) => writer.flush(),
        }
    }
}

enum Signer<'a> {
    /// Integrity and authenticity using an OpenPGP key.
    Gpg {
        helper: &'a PgpVerificationHelper,
        buffer: std::io::Cursor<Vec<u8>>,
    },
}

impl<'a> Signer<'a> {
    fn new(integrity_config: &'a SignatureWriterBuilder) -> Self {
        match integrity_config {
            SignatureWriterBuilder::Gpg(helper) => Self::Gpg {
                helper,
                buffer: std::io::Cursor::new(Vec::new()),
            },
        }
    }
}

impl<'a> Write for Signer<'a> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Self::Gpg { buffer, .. } => buffer.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Gpg { buffer, .. } => buffer.flush(),
        }
    }
}

// impl<'a> Signer<'a> {
//     fn verify(self, integrity_check: &[u8]) -> Result<(), anyhow::Error> {
//         match self {
//             Self::Gpg { helper, mut buffer } => {
//                 let mut verifier = DetachedVerifierBuilder::from_bytes(&integrity_check)?
//                     .with_policy(helper.policy.as_ref(), None, helper)
//                     .context("Could not build detached signature verifier")?;

//                 verifier
//                     .verify_reader(&mut buffer)
//                     .context("Signature verification failed")?;

//                 Ok(())
//             }
//         }
//     }
// }

mod gpg {
    use std::{
        io::{self, Write},
        sync::Arc,
    };

    use anyhow::Context as _;
    use openpgp::parse::stream::*;

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
            // TODO: Try to `.build()` before? Would it work?
            //   Try and make sure nothing breaks.
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
}
