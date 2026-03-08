// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

#[allow(dead_code, unused_imports)]
mod common;

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use anyhow::anyhow;
use openpgp::types::ReasonForRevocation;
use prose_backup::{
    BackupService, CreateBackupCommand, CreateBackupOutput, ExtractionSuccess, config::*,
    decryption::PgpDecryptionContext, openpgp, stats::print_stats,
};
use toml::toml;

use crate::common::*;

#[tokio::test]
async fn test_example1() -> Result<(), anyhow::Error> {
    init();

    let now = SystemTime::now();
    let test_id = unique_hex();
    tracing::info!("Test id: {test_id}");

    let backup_config = BackupConfig::try_from(toml! {
        [encryption]
        mode = "pgp"
        pgp.tsk = "encrypt.pgp"

        [signing]
        pgp.enabled = true
        pgp.tsk = "sign.pgp"
    })?;
    tracing::debug!("Parsed config: {backup_config:#?}");

    let blueprints = test_blueprints();

    let prose_pod_api_dir = std::env::var("PROSE_POD_API_DIR")
        .expect("Environment variable `PROSE_POD_API_DIR` should be defined");
    let current_blueprint = blueprints
        .get(&BLUEPRINT_POD_API_DEMO)
        .unwrap()
        .src_relative_to(format!("{prose_pod_api_dir}/local-run/scenarios/demo"));

    let certs: HashMap<PathBuf, openpgp::Cert> = make_test_certs([
        ("encrypt.pgp", now - Duration::from_hours(23)),
        ("sign.pgp", now - Duration::from_hours(23)),
    ])?;

    let pgp_policy = openpgp::policy::StandardPolicy::new();

    let out_dir = Path::new(".out").join(test_id);
    let backup_store = fs_store(out_dir.join("backups"))?;
    let check_store = fs_store(out_dir.join("checks"))?;

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
            prefix: "prose-backup",
            description: "Test backup",
            created_at: now - Duration::from_mins(90),
        };
        service.create_backup(command, &current_blueprint).await?
    };
    tracing::info!("Created backup '{backup_id}'.");
    tracing::info!("Integrity checks: {digest_ids:#?}");

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
    let details = service.get_details(&backup_id, &blueprints).await?;
    tracing::info!("Backup details: {details:#?}");

    print!("\n");
    let download_url = service
        .get_download_url(&backup_id, Duration::from_secs(3))
        .await?;
    tracing::info!("Download URL: <{download_url}>.");

    print!("\n");
    let ExtractionSuccess {
        mut extraction_output,
        extraction_stats,
        ..
    } = service.extract_backup(&backup_id, &blueprints).await?;
    print_stats(
        &extraction_stats.raw_read_stats,
        &extraction_stats.decryption_stats,
        &extraction_stats.decompression_stats,
        extraction_stats.extracted_bytes_count,
    );

    print!("\n");
    let restore_blueprint = blueprints
        .get(&BLUEPRINT_POD_API_DEMO)
        .unwrap()
        .src_relative_to(out_dir.join("restore"));
    extraction_output.blueprint = &restore_blueprint;
    service.restore_backup(extraction_output).await?;

    if std::env::var("e").is_err() {
        fs::remove_dir_all(out_dir)?;
    }

    Ok(())
}
