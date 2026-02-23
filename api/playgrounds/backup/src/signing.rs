// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! When signing is enabled, we MUST still store a hash in addition to the
//! signature otherwise if signing is disabled then backups cannot be restored
//! anymore (no access to public key material to check the detached signature)!

#[non_exhaustive]
#[derive(Debug)]
pub enum SigningHelper<'a> {
    Gpg {
        cert: &'a openpgp::Cert,
        policy: &'a dyn openpgp::policy::Policy,
    },
}

pub use self::pgp::PgpSignatureWriter;
mod pgp {
    use std::{
        io::{self, Write},
        time::SystemTime,
    };

    use anyhow::Context as _;

    pub struct PgpSignatureWriter<'a> {
        signer: openpgp::serialize::stream::Signer<'a>,
    }

    impl<'cert> PgpSignatureWriter<'cert> {
        pub fn new<'policy, W>(
            writer: W,
            cert: &'cert openpgp::Cert,
            policy: &'policy dyn openpgp::policy::Policy,
            time: SystemTime,
        ) -> Result<Self, anyhow::Error>
        where
            W: Write + Send + Sync + 'cert,
        {
            use openpgp::serialize::stream::{Message, Signer};

            let keypair = cert
                .keys()
                .with_policy(policy, Some(time))
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
