// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::sync::Arc;

use openpgp::{
    cert::amalgamation::ValidAmalgamation,
    crypto::SessionKey,
    packet::prelude::*,
    parse::stream::*,
    types::{ReasonForRevocation, RevocationStatus, SymmetricAlgorithm},
};

fn is_ever_compromised<P>(ka: &openpgp::cert::prelude::ValidErasedKeyAmalgamation<'_, P>) -> bool
where
    P: key::KeyParts,
{
    match ka.revocation_status() {
        RevocationStatus::Revoked(signatures) => {
            for signature in signatures {
                if let Some((reason, message)) = signature.reason_for_revocation() {
                    match reason {
                        ReasonForRevocation::KeyCompromised => {
                            tracing::debug!(
                                "Key '{key}' was compromised: {reason:?}",
                                key = ka.key(),
                                reason = String::from_utf8_lossy(message)
                            );
                            return true;
                        }
                        ReasonForRevocation::KeyRetired => {
                            tracing::debug!(
                                "Key '{key}' was retired (soft revocation)",
                                key = ka.key()
                            );
                        }
                        ReasonForRevocation::KeySuperseded => {
                            tracing::debug!(
                                "Key '{key}' was superseded (soft revocation)",
                                key = ka.key()
                            );
                        }
                        reason => {
                            tracing::debug!("Key '{key}' was revoked: {reason:?}", key = ka.key());
                        }
                    }
                }
            }

            // Suppose it hasn’t been compromised otherwise.
            false
        }
        RevocationStatus::CouldBe(_signatures) => {
            tracing::debug!(
                "Key '{key}' might be revoked externally. Not checking.",
                key = ka.key()
            );

            // TODO: Check for external revocation?
            false
        }
        RevocationStatus::NotAsFarAsWeKnow => false,
    }
}
