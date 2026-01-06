// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::io::Write;

use openpgp::serialize::stream::*;

use crate::{
    CreateBackupError,
    gpg::GpgConfig,
    writer_chain::{WriterChainBuilder, either::Either},
};

pub type EncryptionConfig = GpgConfig;

fn encrypt<'a, W: Write + Send + Sync + 'a>(
    writer: W,
    config: &'a EncryptionConfig,
) -> Result<Message<'a>, anyhow::Error> {
    let message = Message::new(writer);

    let recipients = (config.cert)
        .keys()
        // Validate keys and subkeys (check expiration, crypto algorithm…).
        .with_policy(config.policy.as_ref(), None)
        // Filter out unwanted keys.
        .alive()
        .revoked(false)
        .for_storage_encryption();

    let encryptor = Encryptor::for_recipients(message, recipients).build()?;

    // NOTE: Do not compress as we’re already using zstd for compression.

    // Wrap the plaintext in a OpenPGP literal data packet.
    // NOTE: This is where raw data bytes are stored,
    //   alongside other things like the file type.
    let literal = LiteralWriter::new(encryptor).build()?;

    Ok(literal)
}

impl<M, F> WriterChainBuilder<M, F> {
    pub(crate) fn encrypt_if_possible<'a, InnerWriter, OuterWriter>(
        self,
        config: Option<&'a EncryptionConfig>,
    ) -> WriterChainBuilder<
        impl FnOnce(InnerWriter) -> Result<OuterWriter, CreateBackupError>,
        impl FnOnce(OuterWriter) -> Result<(), CreateBackupError>,
    >
    where
        InnerWriter: Write + Send + Sync + 'a,
        M: FnOnce(Either<Message<'a>, InnerWriter>) -> Result<OuterWriter, CreateBackupError>,
        F: FnOnce(OuterWriter) -> Result<Either<Message<'a>, InnerWriter>, CreateBackupError>,
    {
        let Self { make, finalize, .. } = self;

        WriterChainBuilder {
            make: move |writer: InnerWriter| {
                let writer: Either<_, _> = match config {
                    Some(config) => {
                        let encrypted_writer =
                            encrypt(writer, config).map_err(CreateBackupError::CannotEncrypt)?;
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
