// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! Tests error paths and cold branches.

mod common;

use crate::common::{limited_store::LimitedStore, prelude::*};

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

    let blueprint = ArchiveBlueprint::new(
        1,
        [
            ("foo-data", "foo"),
            ("bar-data", "bar"),
        ],
    )
    .src_relative_to(&test_data_path);

    create_files(&test_data_path, ["foo/", "foo/a"]).unwrap();

    let blueprints = BlueprintsBuilder::new().insert(blueprint.clone()).build();

    let service = BackupService::from_config_custom(
        &backup_config,
        ArchivingContext { blueprints },
        RestorationContext { migrations: vec![] },
        |_| unreachable!(),
        || -> openpgp::policy::StandardPolicy { unreachable!() },
    )
    .unwrap();

    println!();
    let res = {
        let command = CreateBackupCommand {
            prefix: "prose-backup",
            description: "Test backup",
            blueprint: &blueprint.clone(),
            additional_archive_data: Option::<()>::None,
            created_at: now - Duration::from_mins(90),
        };
        service.create_backup(command, &mut NoopEventHandler).await
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

    let mut blueprint = ArchiveBlueprint::new(
        1,
        [
            ("foo-data", "foo"),
            ("bar-data", "bar"),
        ],
    )
    .src_relative_to(&test_data_path);
    let entry = blueprint.paths.pop().unwrap();

    create_files(&test_data_path, ["foo/", "foo/a"]).unwrap();

    let blueprints = BlueprintsBuilder::new().insert(blueprint.clone()).build();

    let mut service = BackupService::from_config_custom(
        &backup_config,
        ArchivingContext { blueprints },
        RestorationContext { migrations: vec![] },
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
            blueprint: &blueprint.clone(),
            additional_archive_data: Option::<()>::None,
            created_at: now - Duration::from_mins(90),
        };
        service
            .create_backup(command, &mut NoopEventHandler)
            .await
            .unwrap()
    };
    let CreateBackupOutput { backup_id, .. } = creation_output;

    println!();
    blueprint.paths.push(entry);
    (service.archiving_context.blueprints).insert(blueprint.version, blueprint.clone());
    let res = service
        .restore_backup(&backup_id, &blueprint, &mut NoopEventHandler)
        .await;
    assert!(res.is_err());
    let err = format!("{err:#}", err = anyhow::Error::from(res.err().unwrap()));
    tracing::info!("Error: {err}");
    assert!(err.contains("Invalid backup: Missing data"));
}

/// Tests that partially uploaded backups are deleted if upload fails.
#[tokio::test(flavor = "multi_thread")]
async fn error_path_upload_fail() {
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

    let blueprint =
        ArchiveBlueprint::new(1, [("foo-data", "foo")]).src_relative_to(&test_data_path);

    create_files(&test_data_path, ["foo/", "foo/a"]).unwrap();

    let blueprints = BlueprintsBuilder::new().insert(blueprint.clone()).build();

    let mut service = BackupService::from_config_custom(
        &backup_config,
        ArchivingContext { blueprints },
        RestorationContext { migrations: vec![] },
        |_| unreachable!(),
        || -> openpgp::policy::StandardPolicy { unreachable!() },
    )
    .unwrap();

    LimitedStore::wrap(&mut service.check_store, 16, false);

    println!();
    let res = service
        .create_backup(
            CreateBackupCommand {
                prefix: "prose-backup",
                description: "Test backup",
                blueprint: &blueprint.clone(),
                additional_archive_data: Option::<()>::None,
                created_at: now - Duration::from_mins(90),
            },
            &mut NoopEventHandler,
        )
        .await;
    assert!(res.is_err());
    let err = format!("{err:#}", err = anyhow::Error::from(res.err().unwrap()));
    tracing::info!("Error: {err}");
    assert_eq!(
        err.as_str(),
        "Failed uploading backup integrity check: `std::io::copy` failed: LimitedWriter limit reached."
    );

    let files = std::fs::read_dir(test_data_path.join("store"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    assert_eq!(files.len(), 0, "{files:#?}");
}
