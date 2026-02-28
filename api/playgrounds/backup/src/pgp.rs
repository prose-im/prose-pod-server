// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use openpgp::anyhow::anyhow;
    use openpgp::cert::CertBuilder;
    use openpgp::cert::amalgamation::{ValidAmalgamation, ValidateAmalgamation};
    use openpgp::packet::prelude::*;
    use openpgp::policy::*;
    use openpgp::types::*;

    /// It’s important that compromised keys are not considered valid at any
    /// point in time once they are marked as compromised. It’s not clear that
    /// `sequoia_openpgp` handles that internally so here is a test to prove it.
    /// No need to handle this case ourselves.
    #[test]
    fn test_standard_policy_handles_retroactive_compromission() -> openpgp::Result<()> {
        let (cert, _) = CertBuilder::new()
            .set_creation_time(SystemTime::now() - Duration::from_hours(8))
            .add_userid("Alice")
            .add_signing_subkey()
            .add_authentication_subkey()
            .generate()?;

        let policy = StandardPolicy::new();

        let compromised_subkey_fingerprint = cert
            .keys()
            .with_policy(&policy, None)
            // NOTE: In a production app, we’d filter more than that.
            .for_signing()
            .next()
            .unwrap()
            .key()
            .fingerprint();
        let superseded_subkey_fingerprint = cert
            .keys()
            .with_policy(&policy, None)
            // NOTE: In a production app, we’d filter more than that.
            .for_authentication()
            .next()
            .unwrap()
            .key()
            .fingerprint();

        let mut keypair = cert
            .primary_key()
            .key()
            .clone()
            .parts_into_secret()?
            .into_keypair()?;

        let revocation_time = SystemTime::now() - Duration::from_hours(4);

        let (cert, sig_compromised) = revoke_subkey(
            &cert,
            &compromised_subkey_fingerprint,
            &mut keypair,
            revocation_time.clone(),
            ReasonForRevocation::KeyCompromised,
            b"It was the maid :/",
        )?;
        let (cert, sig_superseded) = revoke_subkey(
            &cert,
            &superseded_subkey_fingerprint,
            &mut keypair,
            revocation_time.clone(),
            ReasonForRevocation::KeySuperseded,
            b"Rotated.",
        )?;

        fn after(time: &SystemTime) -> SystemTime {
            time.checked_add(Duration::from_hours(1)).unwrap()
        }
        fn before(time: &SystemTime) -> SystemTime {
            time.checked_sub(Duration::from_hours(1)).unwrap()
        }

        // The default standard policy handles retroactive compromission.
        {
            let standard_policy = StandardPolicy::new();

            let compromised_key = cert
                .keys()
                .subkeys()
                .find(|ka| ka.key().fingerprint() == compromised_subkey_fingerprint)
                .expect("Subkey should exist");

            // Compromission is retroactive.
            assert_eq!(
                compromised_key.revocation_status(&standard_policy, after(&revocation_time)),
                RevocationStatus::Revoked(vec![&sig_compromised])
            );
            assert_eq!(
                compromised_key.revocation_status(&standard_policy, before(&revocation_time)),
                RevocationStatus::Revoked(vec![&sig_compromised])
            );

            let superseded_key = cert
                .keys()
                .subkeys()
                .find(|ka| ka.key().fingerprint() == superseded_subkey_fingerprint)
                .expect("Subkey should exist");

            // Supersession is NOT retroactive.
            assert_eq!(
                superseded_key.revocation_status(&standard_policy, after(&revocation_time)),
                RevocationStatus::Revoked(vec![&sig_superseded])
            );
            assert_eq!(
                superseded_key.revocation_status(&standard_policy, before(&revocation_time)),
                RevocationStatus::NotAsFarAsWeKnow
            );
        }

        // It also works if we apply a later policy first.
        {
            let compromised_subkey_at_later = cert
                .keys()
                .with_policy(&policy, after(&revocation_time))
                .subkeys()
                .find(|ka| ka.key().fingerprint() == compromised_subkey_fingerprint)
                .expect("Subkey should exist");

            // Compromission is retroactive.
            assert_eq!(
                compromised_subkey_at_later.revocation_status(),
                RevocationStatus::Revoked(vec![&sig_compromised])
            );
            assert_eq!(
                compromised_subkey_at_later
                    .with_policy(&policy, before(&revocation_time))?
                    .revocation_status(),
                RevocationStatus::Revoked(vec![&sig_compromised])
            );

            let superseded_key_at_later = cert
                .keys()
                .with_policy(&policy, after(&revocation_time))
                .subkeys()
                .find(|ka| ka.key().fingerprint() == superseded_subkey_fingerprint)
                .expect("Subkey should exist");

            // Supersession is NOT retroactive.
            assert_eq!(
                superseded_key_at_later.revocation_status(),
                RevocationStatus::Revoked(vec![&sig_superseded])
            );
            assert_eq!(
                superseded_key_at_later
                    .with_policy(&policy, before(&revocation_time))?
                    .revocation_status(),
                RevocationStatus::NotAsFarAsWeKnow
            );
        }

        println!("OK");
        Ok(())
    }

    fn revoke_subkey(
        cert: &openpgp::Cert,
        subkey_fingerprint: &openpgp::Fingerprint,
        signer: &mut dyn openpgp::crypto::Signer,
        time: impl Into<SystemTime>,
        code: ReasonForRevocation,
        reason: impl AsRef<[u8]>,
    ) -> openpgp::Result<(openpgp::Cert, Signature)> {
        // Find the subkey.
        let Some(ka) = cert
            .keys()
            .subkeys()
            .find(|ka| ka.key().fingerprint() == *subkey_fingerprint)
        else {
            return Err(anyhow!("Subkey not found."));
        };

        // Build the revocation signature.
        let revocation = SignatureBuilder::new(SignatureType::SubkeyRevocation)
            .set_signature_creation_time(time)?
            .set_reason_for_revocation(code, reason)?
            .sign_subkey_binding(signer, cert.primary_key().key(), ka.key())?;

        // Add the revocation packet to the cert.
        let revoked_cert = cert.clone().insert_packets(revocation.clone())?.0;

        Ok((revoked_cert, revocation))
    }
}
