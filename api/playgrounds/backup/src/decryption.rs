// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

#[non_exhaustive]
#[derive(Debug, Default)]
pub struct DecryptionContext<'a> {
    pub pgp: Option<PgpDecryptionContext<'a>>,
}

pub(crate) fn reader<'a, R>(
    backup_reader: R,
    context: &'a DecryptionContext,
    parsed_backup_name @ crate::BackupFileNameComponents {
        extensions,
        ..
    }: &crate::BackupFileNameComponents,
    stats: &mut crate::stats::ReadStats,
) -> Result<impl std::io::Read, anyhow::Error>
where
    R: std::io::Read + Send + Sync + 'a,
{
    if extensions.ends_with(".pgp") {
        if let Some(context) = context.pgp.as_ref() {
            let decryptor = pgp::decryptor(backup_reader, context, parsed_backup_name)?;

            let decryptor = crate::stats::StatsReader::new(decryptor, stats);

            Ok(Either::A(decryptor))
        } else {
            Err(anyhow::Error::msg(
                "Encryption not configured. Cannot find private keys.",
            ))
        }
    } else {
        tracing::debug!("NOT DECRYPTING");

        Ok(Either::B(backup_reader))
    }
}

use crate::writer_chain::either::Either;

pub use self::pgp::PgpDecryptionContext;
mod pgp {
    use anyhow::Context as _;
    use openpgp::{
        crypto::SessionKey, packet::prelude::*, parse::Parse as _, parse::stream::*,
        types::SymmetricAlgorithm,
    };

    #[derive(Debug)]
    pub struct PgpDecryptionContext<'policy> {
        pub certs: Vec<openpgp::Cert>,
        pub policy: &'policy dyn openpgp::policy::Policy,
    }

    struct PgpDecryptionHelper<'cert, 'policy> {
        certs: &'cert [openpgp::Cert],
        policy: &'policy dyn openpgp::policy::Policy,
        time: std::time::SystemTime,
    }

    pub fn decryptor<'a, R>(
        backup_reader: R,
        context: &'a PgpDecryptionContext,
        crate::BackupFileNameComponents { created_at, .. }: &crate::BackupFileNameComponents,
    ) -> Result<impl std::io::Read, anyhow::Error>
    where
        R: std::io::Read + Send + Sync + 'a,
    {
        let helper = PgpDecryptionHelper {
            certs: context.certs.as_slice(),
            policy: context.policy,
            time: (*created_at).into(),
        };
        let decryptor = DecryptorBuilder::from_reader(backup_reader)
            .context("Failed creating decryptor builder")?
            .with_policy(context.policy, Some(helper.time.clone()), helper)
            .context("Failed creating decryptor")?;

        Ok(decryptor)
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
