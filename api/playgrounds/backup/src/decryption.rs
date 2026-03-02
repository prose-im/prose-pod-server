// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

#[non_exhaustive]
#[derive(Debug, Default)]
pub struct DecryptionContext<'a> {
    pub pgp: Option<PgpDecryptionContext<'a>>,
}

pub use self::pgp::{PgpDecryptionContext, PgpDecryptionHelper};
mod pgp {
    use openpgp::{
        crypto::SessionKey, packet::prelude::*, parse::stream::*, types::SymmetricAlgorithm,
    };

    #[derive(Debug)]
    pub struct PgpDecryptionContext<'policy> {
        pub certs: Vec<openpgp::Cert>,
        pub policy: &'policy dyn openpgp::policy::Policy,
    }

    #[derive(Debug)]
    pub struct PgpDecryptionHelper<'cert, 'policy> {
        pub certs: &'cert [openpgp::Cert],
        pub policy: &'policy dyn openpgp::policy::Policy,
        pub time: std::time::SystemTime,
    }

    impl<'cert, 'policy> PgpDecryptionHelper<'cert, 'policy> {
        // TODO: Cache? See [`DecryptionHelper`] docs for an example.
        fn lookup_key(
            &self,
            recipient: &openpgp::KeyHandle,
        ) -> Option<(
            openpgp::Cert,
            key::Key<key::SecretParts, key::UnspecifiedRole>,
        )> {
            for cert in self.certs.iter() {
                for ka in cert
                    .keys()
                    // Validate keys and subkeys (check expiration, crypto algorithm…).
                    .with_policy(self.policy, Some(self.time))
                    // Filter out unwanted keys.
                    .supported()
                    .alive()
                    .revoked(false)
                    // Select key for encryption.
                    .for_storage_encryption()
                    .secret()
                {
                    if ka.key().key_handle() == *recipient {
                        tracing::trace!("Lookup found key `{}`.", ka.key());
                        return Some((cert.clone(), ka.key().clone()));
                    }
                }
            }

            tracing::debug!("Lookup found no key.");
            None
        }
    }

    impl<'cert, 'policy> openpgp::parse::stream::DecryptionHelper
        for PgpDecryptionHelper<'cert, 'policy>
    {
        // NOTE: Inspired by [`DecryptionHelper`] docs.
        fn decrypt(
            &mut self,
            pkesks: &[PKESK],
            _skesks: &[SKESK],
            sym_algo: Option<SymmetricAlgorithm>,
            decrypt: &mut dyn FnMut(Option<SymmetricAlgorithm>, &SessionKey) -> bool,
        ) -> Result<Option<openpgp::Cert>, anyhow::Error> {
            let todo = "Get inspiration from https://gitlab.com/sequoia-pgp/sequoia-sq/-/blob/main/lib/src/decrypt.rs#L770";
            let fixme = "Support key password";
            let fixme = "Make PgpDecryptionHelper not public by moving OpenPGP decryption-related code into this module.";

            // Second, we try those keys that we can use without
            // prompting for a password.
            for pkesk in pkesks {
                let Some(recipient) = pkesk.recipient() else {
                    if cfg!(debug_assertions) {
                        panic!("Prose backups should not contain PKESKs with no recipient.");
                    }
                    continue;
                };

                if let Some((cert, key)) = self.lookup_key(&recipient) {
                    if !key.secret().is_encrypted() {
                        let mut keypair = key.clone().into_keypair()?;
                        tracing::trace!("Trying encryption key: `{key}`.");
                        if pkesk
                            .decrypt(&mut keypair, sym_algo)
                            .map(|(algo, sk)| decrypt(algo, &sk))
                            .unwrap_or(false)
                        {
                            tracing::trace!("Found encryption key: `{key}`.");
                            return Ok(Some(cert));
                        }
                    }
                }
            }

            Err(openpgp::Error::MissingSessionKey("No matching key found.".into()).into())
        }
    }

    impl<'cert, 'policy> VerificationHelper for PgpDecryptionHelper<'cert, 'policy> {
        fn get_certs(
            &mut self,
            // NOTE: Not filtering certs because the certs we have access to
            //   are probably all relevant (e.g. not a whole contact list).
            _ids: &[openpgp::KeyHandle],
        ) -> Result<Vec<openpgp::Cert>, anyhow::Error> {
            Ok(self.certs.to_vec())
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
