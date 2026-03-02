// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::{
    collections::HashMap,
    fs,
    io::Read as _,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use bytes::Bytes;
use openpgp::{cert::prelude::*, packet::key, policy::StandardPolicy, types::ReasonForRevocation};
use prose_backup::{
    BackupService, CreateBackupCommand, CreateBackupOutput,
    config::{EncryptionMode, HashingAlgorithm, *},
    decryption::{DecryptionContext, PgpDecryptionContext},
    encryption::EncryptionContext,
    openpgp,
    signing::PgpSigningContext,
    verification::pgp::{PgpVerificationContext, PgpVerificationHelper},
};

#[tokio::test]
async fn test_example1() -> Result<(), anyhow::Error> {
    tracing_subscriber::fmt()
        .compact()
        .without_time()
        .with_target(false)
        .with_max_level(tracing::Level::TRACE)
        .init();

    let prose_pod_api_data = Bytes::new();

    let now = SystemTime::now();

    let archiving_config = ArchivingConfig {
        version: prose_backup::CURRENT_VERSION,
    };
    let compression_config = CompressionConfig {
        zstd_compression_level: 5,
    };
    let encryption_config = EncryptionConfig {
        enabled: true,
        mode: EncryptionMode::Pgp,
        pgp: Some(EncryptionPgpConfig {
            tsk: Path::new("cert1").to_path_buf(),
            additional_decryption_keys: vec![],
            additional_recipients: vec![],
        }),
    };
    let hashing_config = HashingConfig {
        algorithm: HashingAlgorithm::Sha256,
    };
    let signing_config = Some(SigningConfig {
        mandatory: false,
        pgp: Some(SigningPgpConfig {
            tsk: Path::new("cert2").to_path_buf(),
            additional_trusted_issuers: vec![],
        }),
    });

    let certs: HashMap<PathBuf, openpgp::Cert> = [
        (
            Path::new("cert1").to_path_buf(),
            generate_test_cert(now - Duration::from_hours(23))?,
        ),
        (
            Path::new("cert2").to_path_buf(),
            generate_test_cert(now - Duration::from_hours(23))?,
        ),
    ]
    .into_iter()
    .collect();

    let pgp_policy = openpgp::policy::StandardPolicy::new();

    let encryption_context = if encryption_config.enabled {
        match encryption_config.pgp.as_ref() {
            Some(pgp) => {
                let pgp_cert = certs.get(&pgp.tsk).unwrap();
                Some(EncryptionContext::Pgp {
                    cert: &pgp_cert,
                    policy: &pgp_policy,
                })
            }
            None => None,
        }
    } else {
        None
    };
    let pgp_signing_context = match signing_config.as_ref() {
        Some(config) => match config.pgp.as_ref() {
            Some(pgp) => {
                let pgp_cert = certs.get(&pgp.tsk).unwrap();
                Some(PgpSigningContext {
                    tsk: &pgp_cert,
                    policy: &pgp_policy,
                })
            }
            None => None,
        },
        None => None,
    };
    let pgp_verification_context = match signing_config.as_ref() {
        Some(config) => match config.pgp.as_ref() {
            Some(pgp) => {
                let pgp_cert = certs.get(&pgp.tsk).unwrap();
                Some(PgpVerificationContext {
                    helper: PgpVerificationHelper {
                        certs: vec![pgp_cert.clone()],
                    },
                    policy: &pgp_policy,
                })
            }
            None => None,
        },
        None => None,
    };
    let decryption_context = if encryption_config.enabled {
        match encryption_config.pgp.as_ref() {
            Some(pgp) => {
                let pgp_cert = certs.get(&pgp.tsk).unwrap();
                let mut context = DecryptionContext::default();
                context.pgp = Some(PgpDecryptionContext {
                    tsks: vec![pgp_cert.clone()],
                    policy: &pgp_policy,
                });
                context
            }
            None => DecryptionContext::default(),
        }
    } else {
        DecryptionContext::default()
    };

    let fs_prefix = Path::new(".out");

    let fs_prefix_backups = fs_prefix.join("backups");
    fs::create_dir_all(&fs_prefix_backups)?;
    let backup_store = prose_backup::stores::Fs::default()
        .overwrite(true)
        .directory(&fs_prefix_backups);

    let fs_prefix_integrity_checks = fs_prefix.join("checks");
    fs::create_dir_all(&fs_prefix_integrity_checks)?;
    let check_store = prose_backup::stores::Fs::default()
        .overwrite(true)
        .directory(&fs_prefix_integrity_checks);

    let mut service = BackupService {
        fs_root: PathBuf::from("./data"),
        archiving_config,
        compression_config,
        hashing_config,
        signing_config,
        pgp_signing_context,
        encryption_context,
        backup_store,
        check_store,
        pgp_verification_context,
        decryption_context,
    };

    let CreateBackupOutput {
        backup_id,
        digest_ids,
        ..
    } = {
        let command = CreateBackupCommand {
            description: "backup",
            created_at: now - Duration::from_mins(90),
        };
        service.create_backup(command, prose_pod_api_data).await?
    };
    let integrity_check_file_name = digest_ids
        .first()
        .expect("At least one digest should have been created");
    tracing::info!("Created backup '{backup_id}'.");

    if encryption_config.enabled {
        if let Some(pgp) = encryption_config.pgp.as_ref() {
            let mut pgp_cert = certs.get(&pgp.tsk).unwrap().clone();

            pgp_cert = revoke_subkey_simple(
                pgp_cert,
                |keys| keys.for_storage_encryption(),
                SystemTime::now() - Duration::from_mins(10),
                ReasonForRevocation::KeySuperseded,
            )?;

            service.decryption_context.pgp = Some(PgpDecryptionContext {
                tsks: vec![pgp_cert],
                policy: &pgp_policy,
            });
            service.pgp_verification_context = None;
        }
    }

    print!("\n");
    let backups = service.list_backups().await?;
    tracing::info!("Backups: {backups:#?}");

    print!("\n");
    let fs_prefix_extract = fs_prefix.join("extract");
    std::fs::create_dir_all(&fs_prefix_extract)?;
    let mut restore_result = service
        .restore_backup(&backup_id, fs_prefix_extract)
        .await?;

    print!("\n");
    tracing::info!("Reading Pod API data…");
    let prose_pod_api_data_len = restore_result
        .prose_pod_api_data
        .metadata()
        .map_or(0, |meta| meta.len());
    // NOTE: This `as` is safe to use as we’re working with unsigned integers
    //   and it’s fine if capacity is less than real size. Also there is
    //   no chance we’ll go above `u32::MAX` on a 32-bit target.
    let mut prose_pod_api_data: Vec<u8> = Vec::with_capacity(prose_pod_api_data_len as usize);
    restore_result
        .prose_pod_api_data
        .read_to_end(&mut prose_pod_api_data)?;

    if std::env::var("NO_DELETE").is_err() {
        fs::remove_file(fs_prefix_backups.join(backup_id))?;
        fs::remove_file(fs_prefix_integrity_checks.join(integrity_check_file_name))?;
    }

    Ok(())
}

// MARK: - Helpers

fn generate_test_cert(created_at: SystemTime) -> Result<openpgp::Cert, anyhow::Error> {
    use openpgp::cert::CertBuilder;
    use std::time::Duration;

    let validity = Duration::from_hours(24);

    // Build a cert with user ID + primary key + subkey
    let (mut cert, _signature) = CertBuilder::new()
        .add_userid("Test User <test@example.org>")
        .set_creation_time(created_at)
        .set_validity_period(validity)
        .add_signing_subkey()
        .add_storage_encryption_subkey()
        .generate()?;
    tracing::debug!(
        "Created cert `{cert}` valid from {} to {}.",
        time::UtcDateTime::from(created_at),
        time::UtcDateTime::from(created_at + validity)
    );

    let revoke_encryption_key = false;
    if revoke_encryption_key {
        cert = revoke_subkey_simple(
            cert,
            |keys| keys.for_storage_encryption(),
            created_at + Duration::from_hours(1),
            ReasonForRevocation::KeySuperseded,
        )?;

        assert_eq!(
            cert.keys()
                .with_policy(&StandardPolicy::new(), None)
                .revoked(true)
                .count(),
            1
        );
    }

    Ok(cert)
}

fn revoke_subkey_simple(
    cert: openpgp::Cert,
    filter: impl FnOnce(
        ValidKeyAmalgamationIter<key::PublicParts, key::SubordinateRole>,
    ) -> ValidKeyAmalgamationIter<key::PublicParts, key::SubordinateRole>,
    revocation_time: SystemTime,
    code: openpgp::types::ReasonForRevocation,
) -> openpgp::Result<openpgp::Cert> {
    let policy = StandardPolicy::new();

    let subkeys = cert.keys().subkeys().with_policy(&policy, None);
    let revoked_subkey = filter(subkeys).next().unwrap().key();

    let mut primary_keypair = cert
        .primary_key()
        .key()
        .clone()
        .parts_into_secret()?
        .into_keypair()?;

    let (cert, _sig_superseded) = revoke_subkey(
        &cert,
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

    Ok(cert)
}

fn revoke_subkey<P: key::KeyParts>(
    cert: &openpgp::Cert,
    subkey: &openpgp::packet::Key<P, key::SubordinateRole>,
    signer: &mut dyn openpgp::crypto::Signer,
    time: impl Into<std::time::SystemTime>,
    code: openpgp::types::ReasonForRevocation,
    reason: impl AsRef<[u8]>,
) -> openpgp::Result<(openpgp::Cert, openpgp::packet::Signature)> {
    use openpgp::packet::prelude::*;

    // Build the revocation signature.
    let revocation = SignatureBuilder::new(openpgp::types::SignatureType::SubkeyRevocation)
        .set_signature_creation_time(time)?
        .set_reason_for_revocation(code, reason)?
        .sign_subkey_binding(signer, cert.primary_key().key(), subkey)?;

    // Add the revocation packet to the cert.
    let revoked_cert = cert.clone().insert_packets(revocation.clone())?.0;

    Ok((revoked_cert, revocation))
}
