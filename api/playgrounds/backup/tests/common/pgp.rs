// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use openpgp::{
    cert::prelude::*,
    packet::{Key, Signature, key},
    policy::StandardPolicy,
    types::ReasonForRevocation,
};

pub fn make_test_certs<C>(certs: C) -> Result<HashMap<PathBuf, openpgp::Cert>, anyhow::Error>
where
    C: IntoIterator<Item = (&'static str, SystemTime)>,
    C::IntoIter: ExactSizeIterator,
{
    let iter = certs.into_iter();

    let mut res = HashMap::with_capacity(iter.len());

    for (path, created_at) in iter {
        res.insert(
            Path::new(path).to_path_buf(),
            generate_test_cert(created_at)?,
        );
    }

    Ok(res)
}

pub fn generate_test_cert(created_at: SystemTime) -> Result<openpgp::Cert, anyhow::Error> {
    use openpgp::cert::CertBuilder;
    use std::time::Duration;

    let validity = Duration::from_hours(24);

    // Build a TSK with user ID + primary key + subkey
    let (mut tsk, _signature) = CertBuilder::new()
        .add_userid("Test User <test@example.org>")
        .set_creation_time(created_at)
        .set_validity_period(validity)
        .add_signing_subkey()
        .add_storage_encryption_subkey()
        .generate()?;
    tracing::debug!(
        "Created TSK `{tsk}` valid from {} to {}.",
        time::UtcDateTime::from(created_at),
        time::UtcDateTime::from(created_at + validity)
    );

    let revoke_encryption_key = false;
    if revoke_encryption_key {
        tsk = revoke_subkey_simple(
            tsk,
            |keys| keys.for_storage_encryption(),
            created_at + Duration::from_hours(1),
            ReasonForRevocation::KeySuperseded,
        )?;

        assert_eq!(
            tsk.keys()
                .with_policy(&StandardPolicy::new(), None)
                .revoked(true)
                .count(),
            1
        );
    }

    Ok(tsk)
}

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
