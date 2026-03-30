// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! Tests that backing up and restoring both work with very large files.

mod common;

use std::{process::Command, time::Duration};

use prose_backup::{
    BackupConfig, BackupService, CreateBackupCommand, CreateBackupOutput, CreateBackupSuccess,
    ExtractAndRestoreSuccess,
    archiving::{ArchiveBlueprint, ArchivingContext},
};
use toml::toml;

use crate::common::prelude::*;

#[tokio::test(flavor = "multi_thread")]
async fn large_files() -> Result<(), anyhow::Error> {
    let context = init();
    let TestContext {
        now,
        ref test_data_path,
        ..
    } = context;

    println!();
    let backup_config = {
        let mut toml = toml! {
            [storage.backups]
            provider = "fs"
            fs.directory = "backups"

            [storage.checks]
            provider = "fs"
            fs.directory = "checks"
        };

        map_storage_directories_in_test_dir(&mut toml, test_data_path)?;

        BackupConfig::try_from(toml)
    }?;
    tracing::info!("Parsed config: {backup_config:#?}");

    let blueprint = ArchiveBlueprint::from_iter([("foo-data", "foo")].into_iter())
        .src_relative_to(&test_data_path);

    println!();
    tracing::info!("Creating test files…");
    create_files(&test_data_path, ["foo/"])?;
    let dd_status = Command::new("dd")
        .arg("if=/dev/urandom")
        .arg("of=foo/example.bin")
        .arg("bs=1M")
        .arg("count=1024")
        .current_dir(test_data_path)
        .status()
        .unwrap();
    assert!(dd_status.success());

    const BACKUP_VERSION: u8 = 1;
    let blueprints = BlueprintsBuilder::new()
        .insert(BACKUP_VERSION, blueprint.clone())
        .build();

    println!();
    tracing::info!("Creating service…");
    let service = BackupService::from_config_custom(
        &backup_config,
        ArchivingContext { blueprints },
        |_| unreachable!(),
        || -> openpgp::policy::StandardPolicy { unreachable!() },
    )
    .unwrap();

    println!();
    tracing::info!("Creating backup…");
    let CreateBackupSuccess {
        output: creation_output,
        stats: creation_stats,
        ..
    } = {
        let command = CreateBackupCommand {
            prefix: "prose-backup",
            description: "Test backup",
            version: BACKUP_VERSION,
            blueprint: &blueprint.clone(),
            additional_archive_data: vec![],
            created_at: now - Duration::from_mins(90),
        };
        service.create_backup(command).await?
    };
    let CreateBackupOutput { backup_id, .. } = creation_output;
    tracing::info!("creation_stats: {creation_stats:#?}");

    println!();
    tracing::info!("Restoring backup…");
    let ExtractAndRestoreSuccess {
        verification_report,
        decryption_report,
        extraction_stats,
        ..
    } = service.restore_backup(&backup_id, &blueprint).await?;
    tracing::info!("verification_report: {verification_report:#?}");
    tracing::info!("decryption_report: {decryption_report:#?}");
    tracing::info!("extraction_stats: {extraction_stats:#?}");

    Ok(())
}
