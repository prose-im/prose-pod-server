// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::time::SystemTime;

use openpgp::{
    cert::prelude::*,
    packet::{Key, Signature, key},
    policy::StandardPolicy,
    types::ReasonForRevocation,
};

pub fn revoke_subkey_simple(
    tsk: openpgp::Cert,
    filter: impl FnOnce(
        ValidKeyAmalgamationIter<key::PublicParts, key::SubordinateRole>,
    ) -> ValidKeyAmalgamationIter<key::PublicParts, key::SubordinateRole>,
    revocation_time: SystemTime,
    code: ReasonForRevocation,
) -> openpgp::Result<openpgp::Cert> {
    let policy = StandardPolicy::new();

    let subkeys = tsk.keys().subkeys().with_policy(&policy, None);
    let revoked_subkey = filter(subkeys).next().unwrap().key();

    let mut primary_keypair = tsk
        .primary_key()
        .key()
        .clone()
        .parts_into_secret()?
        .into_keypair()?;

    let (tsk, _sig_superseded) = revoke_subkey(
        &tsk,
        &revoked_subkey,
        &mut primary_keypair,
        revocation_time,
        code,
        b"No reason specified.",
    )?;
    tracing::debug!(
        "Revoked subkey `{revoked_subkey}` ({code:?}) at {}.",
        time::UtcDateTime::from(revocation_time)
    );

    Ok(tsk)
}

pub fn revoke_subkey<P: key::KeyParts>(
    cert: &openpgp::Cert,
    subkey: &Key<P, key::SubordinateRole>,
    signer: &mut dyn openpgp::crypto::Signer,
    time: impl Into<std::time::SystemTime>,
    code: ReasonForRevocation,
    reason: impl AsRef<[u8]>,
) -> openpgp::Result<(openpgp::Cert, Signature)> {
    use openpgp::packet::signature::SignatureBuilder;

    // Build the revocation signature.
    let revocation = SignatureBuilder::new(openpgp::types::SignatureType::SubkeyRevocation)
        .set_signature_creation_time(time)?
        .set_reason_for_revocation(code, reason)?
        .sign_subkey_binding(signer, cert.primary_key().key(), subkey)?;

    // Add the revocation packet to the cert.
    let revoked_cert = cert.clone().insert_packets(revocation.clone())?.0;

    Ok((revoked_cert, revocation))
}
