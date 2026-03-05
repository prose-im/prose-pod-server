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

#[non_exhaustive]
#[derive(Default)]
pub struct SigningContext {
    pub is_signing_mandatory: bool,
    pub pgp: Option<PgpSigningContext>,
}

pub use self::pgp::PgpSigningContext;
mod pgp {
    use std::{
        io::{self, Write},
        time::SystemTime,
    };

    use anyhow::Context as _;

    pub struct PgpSigningContext {
        pub tsk: openpgp::Cert,
        pub policy: Box<dyn openpgp::policy::Policy>,
    }

    pub struct PgpSignatureWriter<'a> {
        signer: openpgp::serialize::stream::Signer<'a>,
    }

    impl PgpSigningContext {
        pub fn new_writer<'a, W>(
            &self,
            writer: W,
            time: SystemTime,
        ) -> Result<PgpSignatureWriter<'a>, anyhow::Error>
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

            Ok(PgpSignatureWriter { signer })
        }
    }

    impl<'a> PgpSignatureWriter<'a> {
        pub fn finalize(self) -> Result<(), anyhow::Error> {
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
