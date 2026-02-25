// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::io::Write;

use openpgp::serialize::stream::*;

use crate::{
    CreateBackupError,
    writer_chain::{WriterChainBuilder, either::Either},
};

#[non_exhaustive]
#[derive(Debug)]
pub enum EncryptionContext<'a> {
    Gpg {
        cert: &'a openpgp::Cert,
        policy: &'a dyn openpgp::policy::Policy,
    },
}

impl<'a> EncryptionContext<'a> {
    fn encrypt<W: Write + Send + Sync + 'a>(
        &self,
        writer: W,
    ) -> Result<EncryptionWriter<'a>, anyhow::Error> {
        match *self {
            Self::Gpg { cert, policy } => {
                self::gpg::encrypt(writer, cert, policy).map(EncryptionWriter::Gpg)
            }
        }
    }
}

pub enum EncryptionWriter<'a> {
    Gpg(Message<'a>),
}

impl<'a> Write for EncryptionWriter<'a> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            EncryptionWriter::Gpg(writer) => writer.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            EncryptionWriter::Gpg(writer) => writer.flush(),
        }
    }
}

impl<'a> EncryptionWriter<'a> {
    fn finalize(self) -> Result<(), anyhow::Error> {
        match self {
            EncryptionWriter::Gpg(message) => message.finalize(),
        }
    }
}

impl<M, F> WriterChainBuilder<M, F> {
    pub(crate) fn encrypt_if_possible<'a, InnerWriter, OuterWriter>(
        self,
        helper: Option<&'a EncryptionContext>,
    ) -> WriterChainBuilder<
        impl FnOnce(InnerWriter) -> Result<OuterWriter, CreateBackupError>,
        impl FnOnce(OuterWriter) -> Result<(), CreateBackupError>,
    >
    where
        InnerWriter: Write + Send + Sync + 'a,
        M: FnOnce(
            Either<EncryptionWriter<'a>, InnerWriter>,
        ) -> Result<OuterWriter, CreateBackupError>,
        F: FnOnce(
            OuterWriter,
        ) -> Result<Either<EncryptionWriter<'a>, InnerWriter>, CreateBackupError>,
    {
        let Self { make, finalize, .. } = self;

        WriterChainBuilder {
            make: move |writer: InnerWriter| {
                let writer: Either<_, _> = match helper {
                    Some(helper) => {
                        let encrypted_writer = helper
                            .encrypt(writer)
                            .map_err(CreateBackupError::CannotEncrypt)?;
                        Either::A(encrypted_writer)
                    }
                    None => Either::B(writer),
                };

                make(writer)
            },

            finalize: move |writer: OuterWriter| {
                let writer: Either<_, _> = finalize(writer)?;

                let res = match writer {
                    Either::A(message) => message
                        .finalize()
                        .map_err(CreateBackupError::EncryptionFailed),
                    Either::B(_) => Ok(()),
                };

                res
            },
        }
    }
}

mod gpg {
    use std::io::Write;

    use openpgp::policy::Policy;
    use openpgp::serialize::stream::*;

    pub fn encrypt<'c, W: Write + Send + Sync + 'c>(
        writer: W,
        cert: &'c openpgp::Cert,
        policy: &'c dyn Policy,
    ) -> Result<Message<'c>, anyhow::Error> {
        let message = Message::new(writer);

        let recipients = cert
            .keys()
            // Validate keys and subkeys (check expiration, crypto algorithm…).
            .with_policy(policy, None)
            // Filter out unwanted keys.
            .supported()
            .alive()
            .revoked(false)
            // Select key for encryption.
            .for_storage_encryption()
            .map(Recipient::from)
            .collect::<Vec<_>>();

        let encryptor = Encryptor::for_recipients(message, recipients).build()?;

        // NOTE: Do not compress as we’re already using zstd for compression.

        // Wrap the plaintext in a OpenPGP literal data packet.
        // NOTE: This is where raw data bytes are stored,
        //   alongside other things like the file type.
        let literal = LiteralWriter::new(encryptor).build()?;

        Ok(literal)
    }
}
