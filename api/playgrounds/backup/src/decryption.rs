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
        crypto::SessionKey, packet::prelude::*, parse::stream::*, parse::Parse as _,
        types::SymmetricAlgorithm,
    };

    use crate::pgp::lookup_secret_key;

    #[derive(Debug)]
    pub struct PgpDecryptionContext<'policy> {
        pub tsks: Vec<openpgp::Cert>,
        pub policy: &'policy dyn openpgp::policy::Policy,
    }

    struct PgpDecryptionHelper<'keys, 'policy> {
        tsks: &'keys [openpgp::Cert],
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
            tsks: context.tsks.as_slice(),
            policy: context.policy,
            time: (*created_at).into(),
        };
        let decryptor = DecryptorBuilder::from_reader(backup_reader)
            .context("Failed creating decryptor builder")?
            .with_policy(context.policy, Some(helper.time.clone()), helper)
            .context("Failed creating decryptor")?;

        Ok(decryptor)
    }

    impl<'keys, 'policy> openpgp::parse::stream::DecryptionHelper
        for PgpDecryptionHelper<'keys, 'policy>
    {
        // NOTE: Inspired by [`DecryptionHelper`] docs.
        // TODO: Improve by looking at <https://gitlab.com/sequoia-pgp/sequoia-sq/-/blob/main/lib/src/decrypt.rs#L770> too.
        // TODO: Add support for encrypted secrets (passphrase-protected).
        fn decrypt(
            &mut self,
            pkesks: &[PKESK],
            _skesks: &[SKESK],
            sym_algo: Option<SymmetricAlgorithm>,
            decrypt: &mut dyn FnMut(Option<SymmetricAlgorithm>, &SessionKey) -> bool,
        ) -> Result<Option<openpgp::Cert>, anyhow::Error> {
            // Try unencrypted secret keys (not passphrase-protected).
            for pkesk in pkesks {
                let Some(recipient) = pkesk.recipient() else {
                    if cfg!(debug_assertions) {
                        panic!("Prose backups should not contain PKESKs with no recipient.");
                    }
                    continue;
                };

                if let Some((cert, key)) =
                    lookup_secret_key(&recipient, self.tsks, self.policy, self.time)
                {
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

            Err(anyhow::Error::new(openpgp::Error::MissingSessionKey(
                "No matching key found when decrypting. Matching keys might exist, \
                but they would be expired, compromised, encrypted (passphrase-protected), \
                or any other similar reason leading to it being dismissed."
                    .to_owned(),
            )))
        }
    }

    impl<'keys, 'policy> VerificationHelper for PgpDecryptionHelper<'keys, 'policy> {
        fn get_certs(
            &mut self,
            // NOTE: Not filtering certs because the certs we have access to
            //   are probably all relevant (e.g. not a whole contact list).
            _ids: &[openpgp::KeyHandle],
        ) -> Result<Vec<openpgp::Cert>, anyhow::Error> {
            Ok(self.tsks.to_vec())
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
