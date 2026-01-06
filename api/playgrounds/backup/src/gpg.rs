// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use openpgp::{
    crypto::SessionKey, packet::prelude::*, parse::stream::*, types::SymmetricAlgorithm,
};

#[derive(Debug)]
pub struct GpgConfig {
    pub cert: openpgp::Cert,
    pub policy: Box<dyn openpgp::policy::Policy>,
}

impl GpgConfig {
    pub fn new(cert: openpgp::Cert) -> Self {
        use openpgp::policy::StandardPolicy;

        Self {
            cert,
            policy: Box::new(StandardPolicy::new()),
        }
    }
}

impl DecryptionHelper for &GpgConfig {
    /// NOTE: Inspired by [`DecryptionHelper`] docs.
    fn decrypt(
        &mut self,
        pkesks: &[PKESK],
        _skesks: &[SKESK],
        sym_algo: Option<SymmetricAlgorithm>,
        decrypt: &mut dyn FnMut(Option<SymmetricAlgorithm>, &SessionKey) -> bool,
    ) -> Result<Option<openpgp::Cert>, anyhow::Error> {
        let cert = self.cert.clone();

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

impl VerificationHelper for &GpgConfig {
    fn get_certs(
        &mut self,
        _ids: &[openpgp::KeyHandle],
    ) -> Result<Vec<openpgp::Cert>, anyhow::Error> {
        Ok(vec![self.cert.clone()])
    }

    fn check(&mut self, structure: MessageStructure) -> Result<(), anyhow::Error> {
        for (i, layer) in structure.into_iter().enumerate() {
            match layer {
                MessageLayer::SignatureGroup { ref results } if i == 0 => {
                    if !results.iter().any(|r| r.is_ok()) {
                        return Err(anyhow::anyhow!("No valid signature"));
                    }
                }

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
