// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! Encryption logic.

use std::{io::Write, time::SystemTime};

use openpgp::serialize::stream::*;

use crate::{
    CreateBackupError,
    writer_chain::{WriterChainBuilder, either::Either},
};

pub use self::EncryptionContext as Context;

#[non_exhaustive]
#[derive(Debug)]
pub enum EncryptionContext {
    Pgp {
        recipients: Vec<openpgp::Cert>,
        policy: Box<dyn openpgp::policy::Policy>,
    },
}

impl EncryptionContext {
    fn encrypt<'a, W: Write + Send + Sync + 'a>(
        &'a self,
        writer: W,
        created_at: SystemTime,
    ) -> Result<EncryptionWriter<'a>, anyhow::Error> {
        match self {
            Self::Pgp { recipients, policy } => {
                self::pgp::encrypt(writer, recipients.as_slice(), policy.as_ref(), created_at)
                    .map(EncryptionWriter::Pgp)
            }
        }
    }
}

pub enum EncryptionWriter<'a> {
    Pgp(Message<'a>),
}

impl<'a> Write for EncryptionWriter<'a> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            EncryptionWriter::Pgp(writer) => writer.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            EncryptionWriter::Pgp(writer) => writer.flush(),
        }
    }
}

impl<'a> EncryptionWriter<'a> {
    fn finalize(self) -> Result<(), anyhow::Error> {
        match self {
            EncryptionWriter::Pgp(message) => message.finalize(),
        }
    }
}

impl<M, F> WriterChainBuilder<M, F> {
    pub(crate) fn encrypt_if_possible<'a, InnerWriter, OuterWriter>(
        self,
        helper: Option<&'a EncryptionContext>,
        created_at: SystemTime,
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
                            .encrypt(writer, created_at)
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

mod pgp {
    use std::io::Write;

    use openpgp::policy::Policy;
    use openpgp::serialize::stream::*;

    pub(crate) fn encrypt<'cert, 'policy: 'cert, W: Write + Send + Sync + 'cert>(
        writer: W,
        recipient_certs: &'cert [openpgp::Cert],
        policy: &'policy dyn Policy,
        created_at: std::time::SystemTime,
    ) -> Result<Message<'cert>, anyhow::Error> {
        let message = Message::new(writer);

        let mut recipients = Vec::with_capacity(recipient_certs.len());

        for cert in recipient_certs.iter() {
            // NOTE: Do NOT cache this (e.g. in `EncryptionContext`)! It’s
            //   important that the recipients are computed at every backup,
            //   to detect when the key becomes invalid. If a backup is
            //   produced with an expired key, it will never be readable.
            let kas = cert
                .keys()
                // Validate keys and subkeys (check expiration, crypto algorithm…).
                .with_policy(policy, Some(created_at))
                // Filter out unwanted keys.
                .supported()
                .alive()
                .revoked(false)
                // Select key for encryption.
                .for_storage_encryption();
            for ka in kas.into_iter() {
                recipients.push((ka, cert));
            }
        }

        if cfg!(debug_assertions) {
            if tracing::enabled!(tracing::Level::DEBUG) {
                let recipients = recipients
                    .iter()
                    .map(|(ka, cert)| (ka.key().fingerprint(), cert.fingerprint()));
                tracing::debug!(
                    "Encrypting for {}.",
                    recipients
                        .map(|(ka, cert)| format!("`{ka}` of cert `{cert}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        }

        if recipients.is_empty() {
            return Err(anyhow::Error::msg("No valid encryption key."));
        }

        let encryptor =
            Encryptor::for_recipients(message, recipients.into_iter().map(|(ka, _)| ka)).build()?;

        // NOTE: Do not compress as we’re already using zstd for compression.

        // Wrap the plaintext in a OpenPGP literal data packet.
        // NOTE: This is where raw data bytes are stored,
        //   alongside other things like the file type.
        let literal = LiteralWriter::new(encryptor).build()?;

        Ok(literal)
    }
}
