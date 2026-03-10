// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

#[allow(dead_code, unused_imports)]
mod common;

use std::{
    collections::HashMap,
    path::PathBuf,
    time::{Duration, SystemTime},
};

use anyhow::anyhow;
use prose_backup::{
    BackupConfig, BackupService, CreateBackupCommand, CreateBackupSuccess,
    ExtractAndRestoreSuccess, stats::print_stats,
};
use toml::toml;

use crate::common::*;

#[tokio::test(flavor = "multi_thread")]
async fn s3() -> Result<(), anyhow::Error> {
    let context = init();
    let TestContext {
        now,
        ref test_data_path,
        ..
    } = context;

    let prose_pod_api_dir = env_required!("PROSE_POD_API_DIR");
    let region = env_required!("S3_REGION");
    let endpoint_url = env_required!("S3_ENDPOINT_URL");
    let access_key = env_required!("S3_ACCESS_KEY");
    let secret_key = env_required!("S3_SECRET_KEY");
    let bucket_name_backups = env_required!("S3_BUCKET_NAME_BACKUPS");
    let bucket_name_checks = env_required!("S3_BUCKET_NAME_CHECKS");

    print!("\n");
    let backup_config = BackupConfig::try_from(toml! {
        [encryption]
        mode = "pgp"
        pgp.tsk = "encrypt.pgp"

        [signing]
        pgp.enabled = true
        pgp.tsk = "sign.pgp"

        [storage.backups]
        mode = "s3"
        s3.bucket_name = bucket_name_backups

        [storage.checks]
        mode = "s3"
        s3.bucket_name = bucket_name_checks

        [s3]
        region = region
        endpoint_url = endpoint_url
        access_key = access_key
        secret_key = secret_key
    })?;
    tracing::info!("Parsed config: {backup_config:#?}");

    print!("\n");
    let certs: HashMap<PathBuf, openpgp::Cert> = make_test_certs([
        ("encrypt.pgp", now - Duration::from_hours(23)),
        ("sign.pgp", now - Duration::from_hours(23)),
    ])?;

    let pgp_policy = openpgp::policy::StandardPolicy::new();

    print!("\n");
    let service = BackupService::from_config_custom(
        backup_config,
        |path| {
            certs
                .get(path)
                .cloned()
                .ok_or(anyhow!("Unknown cert: `{}`.", path.display()))
        },
        || pgp_policy.clone(),
    )?;

    let mut blueprints = test_blueprints();
    let pod_api_demo_blueprint = blueprints.get(&BLUEPRINT_POD_API_DEMO).unwrap();
    let blueprint = pod_api_demo_blueprint
        .src_relative_to(format!("{prose_pod_api_dir}/local-run/scenarios/demo"));

    print!("\n");
    let CreateBackupSuccess {
        creation_output,
        creation_stats,
    } = service
        .create_backup(
            CreateBackupCommand {
                prefix: "test-backup",
                description: "Test backup",
                created_at: SystemTime::now(),
            },
            &blueprint,
        )
        .await?;
    let created_backup_id = creation_output.backup_id;
    tracing::info!("Upload stats: {creation_stats:#?}");

    print!("\n");
    let backups = service.list_backups().await?;
    tracing::info!("Backups: {backups:#?}");
    assert!(backups.iter().any(|backup| backup.id == created_backup_id));

    print!("\n");
    let details = service.get_details(&created_backup_id, &blueprints).await?;
    tracing::info!("Backup details: {details:#?}");

    print!("\n");
    let download_url = service
        .get_download_url(&created_backup_id, Duration::from_secs(3))
        .await?;
    tracing::info!("Download URL: <{download_url}>.");

    print!("\n");
    blueprints.insert(
        BLUEPRINT_POD_API_DEMO,
        pod_api_demo_blueprint.src_relative_to(test_data_path.join("restore")),
    );
    let ExtractAndRestoreSuccess {
        extraction_stats, ..
    } = service
        .restore_backup(&created_backup_id, &blueprints)
        .await?;
    print_stats(
        &extraction_stats.raw_read_stats,
        &extraction_stats.decryption_stats,
        &extraction_stats.decompression_stats,
        extraction_stats.extracted_bytes_count,
    );

    print!("\n");
    () = service.delete_backup(&created_backup_id).await?;

    Ok(())
}
