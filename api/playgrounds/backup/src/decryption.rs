// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! Decryption logic.

use composable_stream::Either;

use crate::stats::ReadStats;

pub(crate) use self::DecryptionContext as Context;

#[non_exhaustive]
#[derive(Debug, Default)]
pub struct DecryptionContext {
    pub pgp: Option<PgpDecryptionContext>,
}

#[allow(unused_variables)]
pub trait DecryptionEventHandler: Send + Sync {
    #[inline]
    fn used_cert_and_subkey(
        &mut self,
        cert: &openpgp::Cert,
        subkey: &openpgp::packet::Key<
            openpgp::packet::key::SecretParts,
            openpgp::packet::key::UnspecifiedRole,
        >,
    ) {
    }
}

#[derive(Debug, Default)]
pub struct DecryptionReport {
    pub used_cert_and_subkey: Option<(openpgp::Fingerprint, openpgp::Fingerprint)>,
}

impl DecryptionEventHandler for DecryptionReport {
    fn used_cert_and_subkey(
        &mut self,
        cert: &openpgp::Cert,
        subkey: &openpgp::packet::Key<
            openpgp::packet::key::SecretParts,
            openpgp::packet::key::UnspecifiedRole,
        >,
    ) {
        self.used_cert_and_subkey = Some((cert.fingerprint(), subkey.fingerprint()));
    }
}

impl crate::ExtractBackupEventHandler for DecryptionReport {
    fn on_decryption_finished(
        &mut self,
        _backup_id: &crate::BackupId,
        _stats: ReadStats,
        report: DecryptionReport,
    ) {
        *self = report;
    }
}

pub(crate) fn reader<'ctx: 'ev, 'ev, R, EventHandler: DecryptionEventHandler>(
    backup_reader: R,
    context: &'ctx DecryptionContext,
    backup_id: &crate::BackupId,
    stats: &mut crate::stats::ReadStats,
    event_handler: &'ev mut EventHandler,
) -> Result<impl std::io::Read, anyhow::Error>
where
    R: std::io::Read + Send + Sync + 'ctx + 'ev,
{
    if backup_id.extensions.contains(&Box::from("pgp")) {
        if let Some(context) = context.pgp.as_ref() {
            let decryptor = pgp::decryptor(backup_reader, context, backup_id, event_handler)?;

            let decryptor = crate::stats::MeteredStream::new(decryptor, stats);

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

pub use self::pgp::*;
pub mod pgp {
    use std::collections::HashMap;

    use anyhow::Context as _;
    use openpgp::{
        crypto::SessionKey, packet::prelude::*, parse::Parse as _, parse::stream::*,
        types::SymmetricAlgorithm,
    };

    use crate::pgp::lookup_secret_key;

    use super::DecryptionEventHandler;

    #[derive(Debug)]
    pub struct PgpDecryptionContext {
        pub tsks: Vec<openpgp::Cert>,
        pub policy: Box<dyn openpgp::policy::Policy>,
        pub passphrases: HashMap<openpgp::Fingerprint, openpgp::crypto::Password>,
    }

    struct PgpDecryptionHelper<'a, EventHandler> {
        tsks: &'a [openpgp::Cert],
        policy: &'a dyn openpgp::policy::Policy,
        passphrases: &'a HashMap<openpgp::Fingerprint, openpgp::crypto::Password>,
        time: std::time::SystemTime,
        event_handler: &'a mut EventHandler,
    }

    pub(crate) fn decryptor<'ctx: 'ev, 'ev, R, EventHandler: DecryptionEventHandler>(
        backup_reader: R,
        context: &'ctx PgpDecryptionContext,
        crate::BackupId { created_at, .. }: &crate::BackupId,
        event_handler: &'ev mut EventHandler,
    ) -> Result<impl std::io::Read, anyhow::Error>
    where
        R: std::io::Read + Send + Sync + 'ctx,
    {
        let helper = PgpDecryptionHelper {
            tsks: context.tsks.as_slice(),
            policy: context.policy.as_ref(),
            passphrases: &context.passphrases,
            time: (*created_at).into(),
            event_handler,
        };
        let decryptor = DecryptorBuilder::from_reader(backup_reader)
            .context("Failed creating decryptor builder")?
            .with_policy(context.policy.as_ref(), Some(helper.time), helper)
            .context("Failed creating decryptor")?;

        Ok(decryptor)
    }

    impl<'keys, E> openpgp::parse::stream::DecryptionHelper for PgpDecryptionHelper<'keys, E>
    where
        E: DecryptionEventHandler,
    {
        // NOTE: Inspired by [`DecryptionHelper`] docs.
        // TODO: Improve by looking at <https://gitlab.com/sequoia-pgp/sequoia-sq/-/blob/main/lib/src/decrypt.rs#L770> too.
        fn decrypt(
            &mut self,
            pkesks: &[PKESK],
            _skesks: &[SKESK],
            sym_algo: Option<SymmetricAlgorithm>,
            decrypt: &mut dyn FnMut(Option<SymmetricAlgorithm>, &SessionKey) -> bool,
        ) -> Result<Option<openpgp::Cert>, anyhow::Error> {
            // Collect recipients upfront to avoid repeated lookups.
            let recipients = pkesks
                .iter()
                .filter_map(|pkesk| {
                    let Some(recipient) = pkesk.recipient() else {
                        if cfg!(debug_assertions) {
                            panic!("Prose backups should not contain PKESKs with no recipient.");
                        }
                        return None;
                    };
                    let (cert, key) =
                        lookup_secret_key(&recipient, self.tsks, self.policy, self.time)?;
                    Some((pkesk, cert, key))
                })
                .collect::<Vec<_>>();

            // First pass: try unencrypted (no passphrase) secret keys.
            for (pkesk, cert, key) in recipients.iter() {
                if key.secret().is_encrypted() {
                    continue;
                }

                let mut keypair = key.clone().into_keypair()?;
                tracing::trace!("Trying unencrypted key: `{key}`.");

                if pkesk
                    .decrypt(&mut keypair, sym_algo)
                    .map(|(algo, sk)| decrypt(algo, &sk))
                    .unwrap_or(false)
                {
                    tracing::trace!("Decrypting with unencrypted key: `{key}`.");
                    self.event_handler.used_cert_and_subkey(cert, key);
                    return Ok(Some(cert.clone()));
                }
            }

            // Second pass: try passphrase-protected secret keys.
            for (pkesk, cert, key) in recipients.iter() {
                if !key.secret().is_encrypted() {
                    continue; // Already tried above.
                }

                for fingerprint in [
                    key.fingerprint(),
                    cert.fingerprint(),
                ] {
                    let Some(passphrase) = self.passphrases.get(&fingerprint) else {
                        tracing::trace!("No passphrase found for fingerprint `{fingerprint}`.");
                        continue;
                    };

                    tracing::trace!("Found passphrase for fingerprint `{fingerprint}`.");

                    let decrypted_key = match key.clone().decrypt_secret(passphrase) {
                        Ok(k) => k,
                        Err(err) => {
                            tracing::trace!(
                                "Failed to decrypt key `{fingerprint}` with passphrase: {err}"
                            );
                            continue;
                        }
                    };

                    let mut keypair = match decrypted_key.into_keypair() {
                        Ok(kp) => kp,
                        Err(err) => {
                            tracing::trace!("Failed to build keypair from `{key}`: {err}");
                            continue;
                        }
                    };

                    tracing::trace!("Trying passphrase-protected key: `{key}`.");

                    if pkesk
                        .decrypt(&mut keypair, sym_algo)
                        .map(|(algo, sk)| decrypt(algo, &sk))
                        .unwrap_or(false)
                    {
                        tracing::trace!("Decrypting with passphrase-protected key: `{key}`.");
                        self.event_handler.used_cert_and_subkey(cert, key);
                        return Ok(Some(cert.clone()));
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

    impl<'keys, E> VerificationHelper for PgpDecryptionHelper<'keys, E> {
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
