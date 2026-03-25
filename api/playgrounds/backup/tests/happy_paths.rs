// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! Tests happy paths of all features given multiple configurations.
//!
//! Essentially ensures that the library works as intended and all features it
//! says are supported really are and work.

#[allow(dead_code, unused_imports, unused_macros)]
mod common;

use std::{
    collections::HashMap,
    path::PathBuf,
    time::{Duration, SystemTime},
};

use anyhow::{Context as _, anyhow};
use openpgp::types::ReasonForRevocation;
use prose_backup::{
    BackupService, CreateBackupCommand, CreateBackupOutput, CreateBackupSuccess, ExtractionSuccess,
    archiving, config::*, decryption::PgpDecryptionContext, openpgp,
};
use toml::toml;

use crate::common::{prelude::*, print::print_stats};

#[tokio::test(flavor = "multi_thread")]
async fn happy_path_noenc_nosign() -> Result<(), anyhow::Error> {
    let config = toml! {
        [encryption]
        mode = "off"

        [signing]
        pgp.enabled = false

        [storage.backups]
        provider = "fs"
        fs.directory = "backups"

        [storage.checks]
        provider = "fs"
        fs.directory = "checks"
    };

    test_happy_path_(config).await
}

#[tokio::test(flavor = "multi_thread")]
async fn happy_path_enc_pgp_nosign() -> Result<(), anyhow::Error> {
    let config = toml! {
        [encryption]
        mode = "pgp"
        pgp.tsk = "encrypt.pgp"

        [signing]
        pgp.enabled = false

        [storage.backups]
        provider = "fs"
        fs.directory = "backups"

        [storage.checks]
        provider = "fs"
        fs.directory = "checks"
    };

    test_happy_path_(config).await
}

#[tokio::test(flavor = "multi_thread")]
async fn happy_path_noenc_sign_pgp() -> Result<(), anyhow::Error> {
    let config = toml! {
        [encryption]
        mode = "off"

        [signing]
        pgp.enabled = true
        pgp.tsk = "sign.pgp"

        [storage.backups]
        provider = "fs"
        fs.directory = "backups"

        [storage.checks]
        provider = "fs"
        fs.directory = "checks"
    };

    test_happy_path_(config).await
}

#[tokio::test(flavor = "multi_thread")]
async fn happy_path_enc_pgp_sign_pgp() -> Result<(), anyhow::Error> {
    let config = toml! {
        [encryption]
        mode = "pgp"
        pgp.tsk = "encrypt.pgp"

        [signing]
        pgp.enabled = true
        pgp.tsk = "sign.pgp"

        [storage.backups]
        provider = "fs"
        fs.directory = "backups"

        [storage.checks]
        provider = "fs"
        fs.directory = "checks"
    };

    test_happy_path_(config).await
}

/// Tests all features of the library, given a configuration.
async fn test_happy_path_(mut config_toml: toml::Table) -> Result<(), anyhow::Error> {
    let context = init();
    let TestContext {
        now,
        ref test_data_path,
        ..
    } = context;

    map_storage_directories_in_test_dir(&mut config_toml, test_data_path)?;

    println!();
    let backup_config = BackupConfig::try_from(config_toml).context("BackupConfig::try_from")?;
    tracing::info!("Parsed config: {backup_config:#?}");

    let blueprints = test_blueprints();

    let prose_pod_api_dir = env_required!("PROSE_POD_API_DIR");
    let archive_version = BLUEPRINT_POD_API_DEMO;
    let pod_api_demo_blueprint = blueprints.get(&archive_version).unwrap();
    let blueprint = pod_api_demo_blueprint
        .src_relative_to(format!("{prose_pod_api_dir}/local-run/scenarios/demo"));
    let restore_blueprint = pod_api_demo_blueprint.src_relative_to(test_data_path.join("restore"));

    println!();
    let certs: HashMap<PathBuf, openpgp::Cert> = make_test_certs([
        ("encrypt.pgp", now - Duration::from_hours(23)),
        ("sign.pgp", now - Duration::from_hours(23)),
    ])?;

    let pgp_policy = openpgp::policy::StandardPolicy::new();

    let encryption_config = backup_config.encryption.clone();

    let mut service = BackupService::from_config_custom(
        &backup_config,
        archiving::Context { blueprints },
        |path| {
            certs
                .get(path)
                .cloned()
                .ok_or(anyhow!("Unknown cert: `{}`.", path.display()))
        },
        || pgp_policy.clone(),
    )
    .context("BackupService::from_config_custom")?;

    println!();
    let CreateBackupSuccess {
        output: creation_output,
        ..
    } = {
        let command = CreateBackupCommand {
            prefix: "prose-backup",
            description: "Test backup",
            version: archive_version,
            blueprint: &blueprint,
            additional_archive_data: vec![],
            created_at: now - Duration::from_mins(90),
        };
        service
            .create_backup(command)
            .await
            .context("create_backup")?
    };
    let CreateBackupOutput {
        backup_id,
        digest_ids,
        ..
    } = creation_output;
    tracing::info!("Created backup '{backup_id}'.");
    tracing::info!("Integrity checks: {digest_ids:#?}");

    println!();
    if let EncryptionConfig::Pgp { config: pgp } = &encryption_config {
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

    println!();
    let backups = service.list_backups().await.context("list_backups")?;
    tracing::info!("Backups: {backups:#?}");

    println!();
    let details = service
        .get_details(&backup_id)
        .await
        .context("get_details")?;
    tracing::info!("Backup details: {details:#?}");

    println!();
    let download_url = service
        .get_download_url(&backup_id, Duration::from_secs(3))
        .await
        .context("get_download_url")?;
    tracing::info!("Download URL: <{download_url}>.");

    println!();
    let ExtractionSuccess {
        extraction_output,
        extraction_stats,
        ..
    } = service
        .extract_backup(&backup_id)
        .await
        .context("extract_backup")?;
    print_stats(
        &extraction_stats.raw_read_stats,
        &extraction_stats.decryption_stats,
        &extraction_stats.decompression_stats,
        extraction_stats.extracted_bytes_count,
    );

    println!();
    service
        .restore_extracted_backup(extraction_output, &restore_blueprint)
        .await
        .context("restore_extracted_backup")?;

    println!();
    () = service
        .delete_backup(&backup_id)
        .await
        .context("delete_backup")?;

    Ok(())
}
