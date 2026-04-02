// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! Signing logic.
//!
//! When signing is enabled, we MUST still store a hash in addition to the
//! signature otherwise if signing is disabled then backups cannot be restored
//! anymore (no access to public key material to check the detached signature)!

pub(crate) use self::SigningContext as Context;

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
    use composable_stream::ComposableStreamBuilder;

    use crate::CreateBackupError;

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
        message: Option<openpgp::serialize::stream::Message<'this>>,
    }

    impl<W: Write> PgpSigner<W> {
        #[cfg_attr(not(coverage), allow(unused_mut))]
        pub fn finalize(mut self) -> Result<W, anyhow::Error> {
            #[cfg(coverage)]
            self.flush()?;

            // SAFETY: Nothing takes the value out of the `Option` until `finalize`.
            self.with_message_mut(|opt| opt.take().unwrap().finalize())?;

            Ok(self.into_heads().writer)
        }
    }

    impl<W> Write for PgpSigner<W>
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

    pub(crate) fn pgp_sign<W>(
        context: &PgpSigningContext,
        time: SystemTime,
    ) -> ComposableStreamBuilder<impl FnOnce(W) -> Result<PgpSigner<W>, CreateBackupError>>
    where
        W: Write + Send + Sync,
    {
        ComposableStreamBuilder {
            make: move |writer: W| {
                PgpSigner::try_new(writer, |writer| context.new_writer(writer, time).map(Some))
                    .context("Failed building OpenPGP signer")
                    .map_err(CreateBackupError::CannotSign)
            },
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
        ) -> Result<openpgp::serialize::stream::Message<'a>, anyhow::Error>
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
                .context("No signing-capable secret key material")?
                .key()
                .to_owned()
                .into_keypair()?;

            let message = Message::new(writer);
            let signer = Signer::new(message, keypair)?
                .detached()
                .creation_time(time);
            let message = signer.build()?;

            Ok(message)
        }
    }
}
