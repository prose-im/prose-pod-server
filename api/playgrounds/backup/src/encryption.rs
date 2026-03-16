// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! Encryption logic.

use std::{io::Write, time::SystemTime};

use crate::{CreateBackupError, writer_chain::WriterChainBuilder};

pub use self::EncryptionContext as Context;

#[non_exhaustive]
#[derive(Debug)]
pub enum EncryptionContext {
    Pgp {
        recipients: Vec<openpgp::Cert>,
        policy: Box<dyn openpgp::policy::Policy>,
    },
}

pub enum EncryptionWriter<'a, W> {
    Pgp(pgp::PgpEncryptedWriter<'a, W>),
}

pub(crate) fn encrypt<'a, W>(
    context: &'a EncryptionContext,
    created_at: SystemTime,
) -> WriterChainBuilder<
    impl FnOnce(W) -> Result<EncryptionWriter<'a, W>, CreateBackupError>,
    impl FnOnce(EncryptionWriter<'a, W>) -> Result<W, CreateBackupError>,
>
where
    W: Write + Send + Sync,
{
    WriterChainBuilder {
        make: move |writer: W| match context {
            EncryptionContext::Pgp { recipients, policy } => {
                let pgp_writer = pgp::PgpEncryptedWriter::try_new(
                    writer,
                    policy,
                    recipients.clone(),
                    |writer, policy, recipients| {
                        self::pgp::encrypt(
                            writer,
                            recipients.as_slice(),
                            policy.as_ref(),
                            created_at,
                        )
                        .map(Some)
                    },
                )
                .map_err(CreateBackupError::CannotEncrypt)?;

                Ok(EncryptionWriter::Pgp(pgp_writer))
            }
        },

        finalize: move |writer: EncryptionWriter<'a, W>| match writer {
            EncryptionWriter::Pgp(pgp_writer) => pgp_writer
                .finalize()
                .map_err(CreateBackupError::EncryptionFailed),
        },
    }
}

impl<'a, W> Write for EncryptionWriter<'a, W>
where
    W: Write,
{
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

mod pgp {
    use std::io::Write;

    use openpgp::policy::Policy;
    use openpgp::serialize::stream::*;

    /// [`openpgp::serialize::stream::Message`] takes ownership of the writer,
    /// but never gives it back. We need a wrapper that holds both the owned
    /// writer but also the owned `Message` which holds a mutable reference to
    /// it. This causes self-referencing issues, which must be worked around
    /// using [`ouroboros`].
    ///
    /// We opened an issue in `sequoia_openpgp` to hopefully monomorphize
    /// writers so we can get back ownership of the inner writer (see
    /// [Monomorphize streams to give back inner writer ownership (#1237) · Issue · sequoia-pgp/sequoia](https://gitlab.com/sequoia-pgp/sequoia/-/work_items/1237)).
    /// Until then, we’ll use this verbose version.
    #[ouroboros::self_referencing(no_doc, pub_extras)]
    pub struct PgpEncryptedWriter<'a, W: 'a> {
        writer: W,
        policy: &'a Box<dyn Policy>,
        certs: Vec<openpgp::Cert>,

        #[borrows(mut writer, policy, certs)]
        #[not_covariant]
        message: Option<Message<'this>>,
    }

    impl<'a, W> PgpEncryptedWriter<'a, W> {
        pub fn finalize(mut self) -> Result<W, anyhow::Error> {
            // SAFETY: Nothing takes the value out of the `Option` until `finalize`.
            self.with_message_mut(|message| message.take().unwrap().finalize())?;

            Ok(self.into_heads().writer)
        }
    }

    impl<'a, W> Write for PgpEncryptedWriter<'a, W>
    where
        W: Write,
    {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            // SAFETY: Nothing takes the value out of the `Option` until `finalize`.
            self.with_message_mut(|opt| opt.as_mut().unwrap().write(buf))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            // SAFETY: Nothing takes the value out of the `Option` until `finalize`.
            self.with_message_mut(|opt| opt.as_mut().unwrap().flush())
        }
    }

    pub(crate) fn encrypt<'a, W: Write + Send + Sync + 'a>(
        writer: W,
        recipient_certs: &'a [openpgp::Cert],
        policy: &'a dyn Policy,
        created_at: std::time::SystemTime,
    ) -> Result<Message<'a>, anyhow::Error> {
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
