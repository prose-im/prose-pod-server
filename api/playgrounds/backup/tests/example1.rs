// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

mod common;

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use anyhow::anyhow;
use openpgp::{policy::StandardPolicy, types::ReasonForRevocation};
use prose_backup::{
    ArchiveBlueprint, BackupService, CreateBackupCommand, CreateBackupOutput, config::*,
    decryption::PgpDecryptionContext, openpgp,
};
use toml::toml;

use crate::common::revoke_subkey_simple;

#[tokio::test]
async fn test_example1() -> Result<(), anyhow::Error> {
    tracing_subscriber::fmt()
        .compact()
        .without_time()
        .with_target(false)
        .with_max_level(tracing::Level::TRACE)
        .init();

    let now = SystemTime::now();

    let backup_config = BackupConfig::try_from(toml! {
        [encryption]
        mode = "pgp"
        pgp.tsk = "encrypt.pgp"

        [signing]
        pgp.enabled = true
        pgp.tsk = "sign.pgp"
    })?;
    tracing::debug!("Parsed config: {backup_config:#?}");

    fn with_fs_root(
        root: impl AsRef<Path>,
        paths: impl AsRef<[(String, PathBuf)]>,
    ) -> Vec<(String, PathBuf)> {
        paths
            .as_ref()
            .iter()
            .map(|(src, dst)| (src.clone(), root.as_ref().join(dst)))
            .collect()
    }

    let pod_api_scenario_base_paths = [
        ("prosody-data".to_owned(), PathBuf::from("prosody/data")),
        ("prosody-config".to_owned(), PathBuf::from("prosody/config")),
    ];
    let blueprints = HashMap::from_iter(
        [
            ArchiveBlueprint::from_paths(
                1,
                vec![
                    (
                        "prosody-data".to_owned(),
                        PathBuf::from("./data/var/lib/prosody"),
                    ),
                    (
                        "prosody-config".to_owned(),
                        PathBuf::from("./data/etc/prosody"),
                    ),
                ],
            ),
            ArchiveBlueprint::from_paths(
                2,
                with_fs_root(
                    "/Users/prose/prose-pod-api/local-run/scenarios/demo",
                    &pod_api_scenario_base_paths,
                ),
            ),
        ]
        .into_iter()
        .map(|blueprint| (blueprint.version, blueprint)),
    );
    let current_blueprint = blueprints.get(&2).unwrap();

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

    let encryption_config = backup_config.encryption.clone();

    let mut service = BackupService::from_config_custom(
        backup_config,
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
        service.create_backup(command, &current_blueprint).await?
    };
    let integrity_check_file_name = digest_ids
        .first()
        .expect("At least one digest should have been created");
    tracing::info!("Created backup '{backup_id}'.");

    if encryption_config.mode == EncryptionMode::Pgp {
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
        }
    }

    print!("\n");
    let backups = service.list_backups().await?;
    tracing::info!("Backups: {backups:#?}");

    print!("\n");
    let mut extract_result = service.extract_backup(&backup_id, &blueprints).await?;

    print!("\n");
    let restore_blueprint = ArchiveBlueprint::from_paths(
        2,
        with_fs_root(".out/restore", &pod_api_scenario_base_paths),
    );
    extract_result.blueprint = &restore_blueprint;
    service.restore_backup(extract_result).await?;

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
