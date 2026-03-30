// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! Tests error paths and cold branches.

mod common;

use std::{path::Path, time::Duration};

use prose_backup::{
    BackupService, CreateBackupCommand, CreateBackupOutput, CreateBackupSuccess,
    archiving::{ArchiveBlueprint, ArchivingContext},
    config::*,
};
use toml::toml;

use crate::common::prelude::*;

#[tokio::test(flavor = "multi_thread")]
async fn error_path_atomic_restore() -> Result<(), anyhow::Error> {
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
            fs.directory = "store"

            [storage.checks]
            provider = "fs"
            fs.directory = "store"
        };

        map_storage_directories_in_test_dir(&mut toml, test_data_path)?;

        BackupConfig::try_from(toml).unwrap()
    };
    tracing::info!("Parsed config: {backup_config:#?}");

    let mut blueprint = ArchiveBlueprint::from_iter(
        [
            ("foo-data", "foo"),
            ("bar-data", "bar"),
        ]
        .into_iter(),
    )
    .src_relative_to(&test_data_path);

    create_files(
        &test_data_path,
        [
            "foo/", "foo/a", "bar/", "bar/a",
        ],
    )?;

    const BACKUP_VERSION: u8 = 1;
    let blueprints = BlueprintsBuilder::new()
        .insert(BACKUP_VERSION, blueprint.clone())
        .build();

    let service = BackupService::from_config_custom(
        &backup_config,
        ArchivingContext { blueprints },
        |_| unreachable!(),
        || -> openpgp::policy::StandardPolicy { unreachable!() },
    )
    .unwrap();

    // Write some random content in `foo/a` to check reversion.
    let original_data = unique_hex().unwrap();
    let foo_a = test_data_path.join("foo/a");
    std::fs::write(&foo_a, &original_data).unwrap();

    println!();
    let CreateBackupSuccess {
        output: creation_output,
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
        service.create_backup(command).await.unwrap()
    };
    let CreateBackupOutput { backup_id, .. } = creation_output;

    {
        // Override `foo/a`.
        std::fs::write(&foo_a, "overriden").unwrap();

        // Restore the backup.
        println!();
        let res = service.restore_backup(&backup_id, &blueprint).await;
        assert!(res.is_ok(), "Error: {:#?}", res.err().unwrap());

        // Test that `foo/a` was reverted.
        assert_eq!(std::fs::read_to_string(&foo_a).unwrap(), original_data);
    }

    {
        // Override `foo/a` again.
        std::fs::write(&foo_a, "overriden").unwrap();

        // Change the second path to one that cannot be written to. Since
        // restoration happens in a sequential manner, we suppose that the
        // file `foo/a` will be written, then reverted when `bar/` fails to be
        // restored. We could use the `notify` crate to watch for file changes
        // and make this test more robust. Note that test coverage confirms
        // that we do indeed revert the directory (as expected).
        blueprint.paths[1].1 = Path::new("/dev/null").to_path_buf();

        // Try to restore the backup (should fail).
        println!();
        let res = service.restore_backup(&backup_id, &blueprint).await;
        assert!(res.is_err());
        let err = format!("{err:#}", err = anyhow::Error::from(res.err().unwrap()));
        tracing::info!("Error: {err}");
        assert!(err.contains("Move failed"));

        // Test that `foo/a` wasn’t changed.
        assert_eq!(std::fs::read_to_string(&foo_a).unwrap(), "overriden");
    }

    // TODO: Test that no new directory was created (i.e. backup directories
    //   cleaned up).

    Ok(())
}

/// Tests that creating a backup fails if a path is missing.
#[tokio::test(flavor = "multi_thread")]
async fn error_path_backup_missing_file() -> Result<(), anyhow::Error> {
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
            fs.directory = "store"

            [storage.checks]
            provider = "fs"
            fs.directory = "store"
        };

        map_storage_directories_in_test_dir(&mut toml, test_data_path)?;

        BackupConfig::try_from(toml).unwrap()
    };
    tracing::info!("Parsed config: {backup_config:#?}");

    let blueprint = ArchiveBlueprint::from_iter(
        [
            ("foo-data", "foo"),
            ("bar-data", "bar"),
        ]
        .into_iter(),
    )
    .src_relative_to(&test_data_path);

    create_files(&test_data_path, ["foo/", "foo/a"])?;

    const BACKUP_VERSION: u8 = 1;
    let blueprints = BlueprintsBuilder::new()
        .insert(BACKUP_VERSION, blueprint.clone())
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
            version: BACKUP_VERSION,
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

    Ok(())
}

/// Tests that restoring a backup fails if the archive is mising a path.
#[tokio::test(flavor = "multi_thread")]
async fn error_path_restore_missing_file() -> Result<(), anyhow::Error> {
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
            fs.directory = "store"

            [storage.checks]
            provider = "fs"
            fs.directory = "store"
        };

        map_storage_directories_in_test_dir(&mut toml, test_data_path)?;

        BackupConfig::try_from(toml).unwrap()
    };
    tracing::info!("Parsed config: {backup_config:#?}");

    let mut blueprint = ArchiveBlueprint::from_iter(
        [
            ("foo-data", "foo"),
            ("bar-data", "bar"),
        ]
        .into_iter(),
    )
    .src_relative_to(&test_data_path);
    let entry = blueprint.paths.pop().unwrap();

    create_files(&test_data_path, ["foo/", "foo/a"])?;

    const BACKUP_VERSION: u8 = 1;
    let blueprints = BlueprintsBuilder::new()
        .insert(BACKUP_VERSION, blueprint.clone())
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
            version: BACKUP_VERSION,
            blueprint: &blueprint.clone(),
            additional_archive_data: vec![],
            created_at: now - Duration::from_mins(90),
        };
        service.create_backup(command).await.unwrap()
    };
    let CreateBackupOutput { backup_id, .. } = creation_output;

    println!();
    blueprint.paths.push(entry);
    (service.archiving_context.blueprints).insert(BACKUP_VERSION, blueprint.clone());
    let res = service.restore_backup(&backup_id, &blueprint).await;
    assert!(res.is_err());
    let err = format!("{err:#}", err = anyhow::Error::from(res.err().unwrap()));
    tracing::info!("Error: {err}");
    assert!(err.contains("Invalid backup: Missing data"));

    Ok(())
}

/// Ensures that restoring a backup works if a backup archive contains an
/// unknown entry. There might be use cases for it, so the library shouldn’t
/// prevent that. Instead, it logs a warning (not tested because annoying).
#[tokio::test(flavor = "multi_thread")]
async fn error_path_unknown_archive_entry_no_error() -> Result<(), anyhow::Error> {
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
            fs.directory = "store"

            [storage.checks]
            provider = "fs"
            fs.directory = "store"
        };

        map_storage_directories_in_test_dir(&mut toml, test_data_path)?;

        BackupConfig::try_from(toml).unwrap()
    };
    tracing::info!("Parsed config: {backup_config:#?}");

    let mut blueprint = ArchiveBlueprint::from_iter([("foo-data", "foo")].into_iter())
        .src_relative_to(&test_data_path);

    create_files(&test_data_path, ["foo/", "foo/a"])?;

    const BACKUP_VERSION: u8 = 1;
    let blueprints = BlueprintsBuilder::new()
        .insert(BACKUP_VERSION, blueprint.clone())
        .build();

    let service = BackupService::from_config_custom(
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
            version: BACKUP_VERSION,
            blueprint: &blueprint.clone(),
            additional_archive_data: vec![],
            created_at: now - Duration::from_mins(90),
        };
        service.create_backup(command).await.unwrap()
    };
    let CreateBackupOutput { backup_id, .. } = creation_output;

    println!();
    blueprint.paths.swap_remove(0);
    service
        .restore_backup(&backup_id, &blueprint)
        .await
        .unwrap();

    Ok(())
}
