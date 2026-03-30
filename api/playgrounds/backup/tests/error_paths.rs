// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! Tests error paths and cold branches.

mod common;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::anyhow;
use prose_backup::{
    BackupService, CreateBackupCommand, CreateBackupOutput, CreateBackupSuccess,
    ExtractAndRestoreSuccess,
    archiving::{ArchiveBlueprint, ArchivingContext},
    config::*,
};
use toml::toml;

use crate::common::prelude::*;

#[tokio::test(flavor = "multi_thread")]
async fn error_path_atomic_restore() {
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

    create_files(
        &test_data_path,
        [
            "foo/", "foo/a", "bar/", "bar/a",
        ],
    )
    .unwrap();

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
}

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
            [storage.backups]
            provider = "fs"
            fs.directory = "store"

            [storage.checks]
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
            [storage.backups]
            provider = "fs"
            fs.directory = "store"

            [storage.checks]
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
}

/// Ensures that restoring a backup works if a backup archive contains an
/// unknown entry. There might be use cases for it, so the library shouldn’t
/// prevent that. Instead, it logs a warning (not tested because annoying).
#[tokio::test(flavor = "multi_thread")]
async fn error_path_unknown_archive_entry_no_error() {
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

        map_storage_directories_in_test_dir(&mut toml, test_data_path).unwrap();

        BackupConfig::try_from(toml).unwrap()
    };
    tracing::debug!("Parsed config: {backup_config:#?}");

    let mut blueprint = ArchiveBlueprint::from_iter([("foo-data", "foo")].into_iter())
        .src_relative_to(&test_data_path);

    create_files(&test_data_path, ["foo/", "foo/a"]).unwrap();

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
}

/// Ensures the library falls back to integrity checking if the signature
/// comes from an unknown key.
///
/// Use cases:
///
/// - Old backup + PGP still configured but key lost.
/// - PGP now configured, malicious actor plants signatures for old unsigned
///   backups, rendering them unrestorable.
#[tokio::test(flavor = "multi_thread")]
async fn error_path_lost_signing_key() {
    let context = init();
    let TestContext {
        now,
        ref test_data_path,
        ..
    } = context;

    println!();
    let backup_config = {
        let mut toml = toml! {
            [signing]
            mandatory = false
            pgp.enabled = true
            pgp.tsk = "sign.pgp"

            [storage.backups]
            provider = "fs"
            fs.directory = "store"

            [storage.checks]
            provider = "fs"
            fs.directory = "store"
        };

        map_storage_directories_in_test_dir(&mut toml, test_data_path).unwrap();

        BackupConfig::try_from(toml).unwrap()
    };
    tracing::debug!("Parsed config: {backup_config:#?}");

    let blueprint = ArchiveBlueprint::from_iter([("foo-data", "foo")].into_iter())
        .src_relative_to(&test_data_path);

    create_files(&test_data_path, ["foo/", "foo/a"]).unwrap();

    const BACKUP_VERSION: u8 = 1;
    let blueprints = BlueprintsBuilder::new()
        .insert(BACKUP_VERSION, blueprint.clone())
        .build();

    println!();
    let certs: HashMap<PathBuf, openpgp::Cert> =
        make_test_certs([("sign.pgp", now - Duration::from_hours(23))]).unwrap();

    let pgp_policy = openpgp::policy::StandardPolicy::new();

    let mut service = BackupService::from_config_custom(
        &backup_config,
        ArchivingContext { blueprints },
        |path| {
            certs
                .get(path)
                .cloned()
                .ok_or(anyhow!("Unknown cert: `{}`.", path.display()))
        },
        || pgp_policy.clone(),
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

    {
        println!();
        let ExtractAndRestoreSuccess {
            verification_report,
            ..
        } = service
            .restore_backup(&backup_id, &blueprint)
            .await
            .unwrap();
        assert!(verification_report.is_signed);
        let report = verification_report.known_signing_keys.first().unwrap();
        assert!(report.is_valid);
    }

    let mut pgp_verification_context =
        std::mem::take(&mut service.verification_context.pgp).unwrap();
    assert_eq!(pgp_verification_context.certs.len(), 1);
    pgp_verification_context.certs = Arc::new(Vec::with_capacity(0));
    service.verification_context.pgp = Some(pgp_verification_context);

    {
        println!();
        let ExtractAndRestoreSuccess {
            verification_report,
            ..
        } = service
            .restore_backup(&backup_id, &blueprint)
            .await
            .unwrap();
        assert!(verification_report.is_signed);
        assert!(verification_report.signature.is_some());
        assert!(verification_report.known_signing_keys.is_empty());
    }
}
