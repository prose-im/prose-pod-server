// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

mod common;

use std::{
    collections::HashMap,
    fs,
    io::Read as _,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use anyhow::anyhow;
use bytes::Bytes;
use openpgp::{policy::StandardPolicy, types::ReasonForRevocation};
use prose_backup::{
    BackupService, CreateBackupCommand, CreateBackupOutput, config::*,
    decryption::PgpDecryptionContext, openpgp,
};

use crate::common::revoke_subkey_simple;

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
            tsk: Path::new("encrypt.pgp").to_path_buf(),
            additional_decryption_keys: vec![],
            additional_recipients: vec![],
        }),
    };
    let hashing_config = HashingConfig {
        algorithm: HashingAlgorithm::Sha256,
    };
    let signing_config = SigningConfig {
        mandatory: false,
        pgp: Some(SigningPgpConfig {
            tsk: Path::new("sign.pgp").to_path_buf(),
            additional_trusted_issuers: vec![],
        }),
    };
    let backup_config = BackupConfig {
        archiving: archiving_config,
        compression: compression_config,
        hashing: hashing_config,
        signing: signing_config,
        encryption: encryption_config.clone(),
    };

    let certs: HashMap<PathBuf, openpgp::Cert> = [
        (
            Path::new("encrypt.pgp").to_path_buf(),
            generate_test_cert(now - Duration::from_hours(23))?,
        ),
        (
            Path::new("sign.pgp").to_path_buf(),
            generate_test_cert(now - Duration::from_hours(23))?,
        ),
    ]
    .into_iter()
    .collect();

    let pgp_policy = openpgp::policy::StandardPolicy::new();

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

    let mut service = BackupService::from_config_custom(
        backup_config,
        PathBuf::from("./data"),
        backup_store,
        check_store,
        |path| {
            certs
                .get(path)
                .cloned()
                .ok_or(anyhow!("Unknown cert: `{}`.", path.display()))
        },
        || pgp_policy.clone(),
    )?;

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
                policy: Box::new(pgp_policy.clone()),
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
