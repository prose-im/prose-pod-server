// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! Signing logic.
//!
//! When signing is enabled, we MUST still store a hash in addition to the
//! signature otherwise if signing is disabled then backups cannot be restored
//! anymore (no access to public key material to check the detached signature)!

pub use self::SigningContext as Context;

pub mod errors {
    #[derive(Debug, thiserror::Error)]
    #[error("Cannot sign")]
    #[repr(transparent)]
    pub struct CannotSign(#[from] pub anyhow::Error);

    #[derive(Debug, thiserror::Error)]
    #[error("Signing failed")]
    #[repr(transparent)]
    pub struct SigningFailed(#[from] pub anyhow::Error);
}

#[non_exhaustive]
#[derive(Default)]
pub struct SigningContext {
    pub is_signing_mandatory: bool,
    pub pgp: Option<PgpSigningContext>,
}

pub use self::pgp::PgpSigningContext;
pub mod pgp {
    use std::{io::Write, time::SystemTime};

    use anyhow::Context as _;

    use crate::writer_chain::WriterChainBuilder;

    use super::errors::*;

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
    pub struct PgpSigner<W: 'static> {
        writer: W,

        #[borrows(mut writer)]
        #[not_covariant]
        signer: Option<openpgp::serialize::stream::Signer<'this>>,
    }

    impl<W> PgpSigner<W> {
        pub fn finalize(mut self) -> Result<W, anyhow::Error> {
            // SAFETY: Nothing takes the value out of the `Option` until `finalize`.
            // TODO: Try to `.build()` before? Would it work?
            //   Try and make sure nothing breaks.
            self.with_signer_mut(|signer| signer.take().unwrap().build()?.finalize())?;

            Ok(self.into_heads().writer)
        }
    }

    impl<W> Write for PgpSigner<W>
    where
        W: Write,
    {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            // SAFETY: Nothing takes the value out of the `Option` until `finalize`.
            self.with_signer_mut(|opt| opt.as_mut().unwrap().write(buf))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            // SAFETY: Nothing takes the value out of the `Option` until `finalize`.
            self.with_signer_mut(|opt| opt.as_mut().unwrap().flush())
        }
    }

    pub(crate) fn pgp_sign<'a, W>(
        context: &'a PgpSigningContext,
        time: SystemTime,
    ) -> WriterChainBuilder<
        impl FnOnce(W) -> Result<PgpSigner<W>, CannotSign>,
        impl FnOnce(PgpSigner<W>) -> Result<W, SigningFailed>,
    >
    where
        W: Write + Send + Sync,
    {
        WriterChainBuilder {
            make: move |writer: W| {
                PgpSigner::try_new(writer, |writer| context.new_writer(writer, time).map(Some))
                    .context("Failed building OpenPGP signer")
                    .map_err(CannotSign)
            },

            finalize: move |writer: PgpSigner<W>| writer.finalize().map_err(SigningFailed),
        }
    }

    pub struct PgpSigningContext {
        pub tsk: openpgp::Cert,
        pub policy: Box<dyn openpgp::policy::Policy>,
    }

    impl PgpSigningContext {
        pub fn new_writer<'a, W>(
            &self,
            writer: W,
            time: SystemTime,
        ) -> Result<openpgp::serialize::stream::Signer<'a>, anyhow::Error>
        where
            W: Write + Send + Sync + 'a,
        {
            use openpgp::serialize::stream::{Message, Signer};

            let keypair = (self.tsk)
                .keys()
                // Validate keys and subkeys (check expiration, crypto algorithm…).
                .with_policy(self.policy.as_ref(), Some(time))
                // Filter out unwanted keys.
                .supported()
                .alive()
                .revoked(false)
                // Get only signing keys.
                .for_signing()
                .secret()
                .next()
                .context("No signing key")?
                .key()
                .to_owned()
                .into_keypair()?;

            let message = Message::new(writer);
            let signer = Signer::new(message, keypair)?
                .detached()
                .creation_time(time);

            Ok(signer)
        }
    }
}
