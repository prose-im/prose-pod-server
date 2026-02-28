// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

#[non_exhaustive]
#[derive(Debug, Default)]
pub struct DecryptionHelper {
    pub gpg: Option<GpgDecryptionHelper>,
}

pub use self::gpg::GpgDecryptionHelper;
mod gpg {
    use std::sync::Arc;

    use openpgp::{
        crypto::SessionKey, packet::prelude::*, parse::stream::*, types::SymmetricAlgorithm,
    };

    #[derive(Debug)]
    pub struct GpgDecryptionHelper {
        pub cert: openpgp::Cert,
        pub policy: Arc<dyn openpgp::policy::Policy>,
    }

    impl GpgDecryptionHelper {
        pub fn new(cert: openpgp::Cert) -> Self {
            use openpgp::policy::StandardPolicy;

            Self {
                cert,
                policy: Arc::new(StandardPolicy::new()),
            }
        }
    }

    impl openpgp::parse::stream::DecryptionHelper for &GpgDecryptionHelper {
        // NOTE: Inspired by [`DecryptionHelper`] docs.
        fn decrypt(
            &mut self,
            pkesks: &[PKESK],
            _skesks: &[SKESK],
            sym_algo: Option<SymmetricAlgorithm>,
            decrypt: &mut dyn FnMut(Option<SymmetricAlgorithm>, &SessionKey) -> bool,
        ) -> Result<Option<openpgp::Cert>, anyhow::Error> {
            let cert = self.cert.clone();

            let todo = "Get inspiration from https://gitlab.com/sequoia-pgp/sequoia-sq/-/blob/main/lib/src/decrypt.rs#L770";
            let fixme = "Pass time!";
            let fixme = "Support key password";

            // Second, we try those keys that we can use without
            // prompting for a password.
            for pkesk in pkesks {
                for key in cert
                    .keys()
                    .with_policy(self.policy.as_ref(), None)
                    .for_storage_encryption()
                    .secret()
                {
                    let mut keypair = key.key().to_owned().into_keypair()?;
                    if pkesk
                        .decrypt(&mut keypair, sym_algo)
                        .map(|(algo, sk)| decrypt(algo, &sk))
                        .unwrap_or(false)
                    {
                        drop(key);
                        return Ok(Some(cert));
                    }
                }
            }

            Err(openpgp::Error::MissingSessionKey("No matching key found".into()).into())
        }
    }

    impl VerificationHelper for &GpgDecryptionHelper {
        fn get_certs(
            &mut self,
            _ids: &[openpgp::KeyHandle],
        ) -> Result<Vec<openpgp::Cert>, anyhow::Error> {
            let fixme = "Return multiple certs";

            Ok(vec![self.cert.clone()])
        }

        fn check(&mut self, structure: MessageStructure) -> Result<(), anyhow::Error> {
            for (i, layer) in structure.into_iter().enumerate() {
                match layer {
                    MessageLayer::Encryption { .. } if i == 0 => {
                        // FIXME: Do something?
                    }

                    layer => {
                        return Err(anyhow::anyhow!("Unexpected message structure ({layer:?})",));
                    }
                }
            }

            Ok(())
        }
    }
}
