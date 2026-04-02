// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! Tests error paths and cold branches.

mod common;

use crate::common::prelude::*;

/// Tests that creating a backup fails if a path is missing.
#[tokio::test(flavor = "multi_thread")]
async fn error_path_backup_missing_file() {
    let context = init();
    let TestContext {
        now,
        ref test_data_path,
        ..
    } = context;

    println!();
    let backup_config = {
        let mut toml = toml! {
            [storage]
            provider = "fs"
            fs.directory = "store"
        };

        map_storage_directories_in_test_dir(&mut toml, test_data_path).unwrap();

        BackupConfig::try_from(toml).unwrap()
    };
    tracing::debug!("Parsed config: {backup_config:#?}");

    let blueprint = ArchiveBlueprint::from_iter(
        [
            ("foo-data", "foo"),
            ("bar-data", "bar"),
        ]
        .into_iter(),
    )
    .src_relative_to(&test_data_path);

    create_files(&test_data_path, ["foo/", "foo/a"]).unwrap();

    let backup_version: u8 = 1;
    let blueprints = BlueprintsBuilder::new()
        .insert(backup_version, blueprint.clone())
        .build();

    let service = BackupService::from_config_custom(
        &backup_config,
        ArchivingContext { blueprints },
        |_| unreachable!(),
        || -> openpgp::policy::StandardPolicy { unreachable!() },
    )
    .unwrap();

    println!();
    let res = {
        let command = CreateBackupCommand {
            prefix: "prose-backup",
            description: "Test backup",
            version: backup_version,
            blueprint: &blueprint.clone(),
            additional_archive_data: vec![],
            created_at: now - Duration::from_mins(90),
        };
        service.create_backup(command).await
    };
    assert!(res.is_err());
    let err = format!("{err:#}", err = anyhow::Error::from(res.err().unwrap()));
    tracing::info!("Error: {err}");
    assert!(err.contains("Cannot archive: Missing file"));
}

/// Tests that restoring a backup fails if the archive is mising a path.
#[tokio::test(flavor = "multi_thread")]
async fn error_path_restore_missing_file() {
    let context = init();
    let TestContext {
        now,
        ref test_data_path,
        ..
    } = context;

    println!();
    let backup_config = {
        let mut toml = toml! {
            [storage]
            provider = "fs"
            fs.directory = "store"
        };

        map_storage_directories_in_test_dir(&mut toml, test_data_path).unwrap();

        BackupConfig::try_from(toml).unwrap()
    };
    tracing::debug!("Parsed config: {backup_config:#?}");

    let mut blueprint = ArchiveBlueprint::from_iter(
        [
            ("foo-data", "foo"),
            ("bar-data", "bar"),
        ]
        .into_iter(),
    )
    .src_relative_to(&test_data_path);
    let entry = blueprint.paths.pop().unwrap();

    create_files(&test_data_path, ["foo/", "foo/a"]).unwrap();

    let backup_version: u8 = 1;
    let blueprints = BlueprintsBuilder::new()
        .insert(backup_version, blueprint.clone())
        .build();

    let mut service = BackupService::from_config_custom(
        &backup_config,
        ArchivingContext { blueprints },
        |_| unreachable!(),
        || -> openpgp::policy::StandardPolicy { unreachable!() },
    )
    .unwrap();

    println!();
    let CreateBackupSuccess {
        output: creation_output,
        ..
    } = {
        let command = CreateBackupCommand {
            prefix: "prose-backup",
            description: "Test backup",
            version: backup_version,
            blueprint: &blueprint.clone(),
            additional_archive_data: vec![],
            created_at: now - Duration::from_mins(90),
        };
        service.create_backup(command).await.unwrap()
    };
    let CreateBackupOutput { backup_id, .. } = creation_output;

    println!();
    blueprint.paths.push(entry);
    (service.archiving_context.blueprints).insert(backup_version, blueprint.clone());
    let res = service.restore_backup(&backup_id, &blueprint).await;
    assert!(res.is_err());
    let err = format!("{err:#}", err = anyhow::Error::from(res.err().unwrap()));
    tracing::info!("Error: {err}");
    assert!(err.contains("Invalid backup: Missing data"));
}
