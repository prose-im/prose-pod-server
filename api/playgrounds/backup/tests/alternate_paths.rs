// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! Tests cold branches (non-happy paths).
//!
//! Errors might be encountered, but recovered from.
//! If recovery is impossible, it becomes an “error path”.

mod common;

use crate::common::prelude::*;

/// Ensures that the library works even if people use the same bucket and prefix
/// as storage for backups and checks.
///
/// This test uses the `fs` storage provider for faster execution time, but
/// it’d be the same with S3.
///
/// NOTE: Other tests happen to cover this path already, but that’s just an
///   implementation detail which could be changed without noticing. This test
///   makes it explicit this is the tested scenario.
#[tokio::test(flavor = "multi_thread")]
async fn alternate_path_single_store() {
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

        BackupConfig::try_from(toml)
    }
    .unwrap();
    tracing::info!("Parsed config: {backup_config:#?}");

    let blueprint = ArchiveBlueprint::from_iter([("foo-data", "foo")].into_iter())
        .src_relative_to(&test_data_path);

    create_files(&test_data_path, ["foo/", "foo/a"]).unwrap();

    let backup_version: u8 = 1;
    let blueprints = BlueprintsBuilder::new()
        .insert(backup_version, blueprint.clone())
        .build();

    println!();
    let certs: HashMap<PathBuf, openpgp::Cert> = make_test_certs([
        ("encrypt.pgp", now - Duration::from_hours(23)),
        ("sign.pgp", now - Duration::from_hours(23)),
    ])
    .unwrap();

    let pgp_policy = openpgp::policy::StandardPolicy::new();

    let service = BackupService::from_config_custom(
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
            version: backup_version,
            blueprint: &blueprint.clone(),
            additional_archive_data: vec![],
            created_at: now - Duration::from_mins(90),
        };
        service.create_backup(command).await.unwrap()
    };
    let CreateBackupOutput {
        backup_id,
        digest_ids,
        ..
    } = creation_output;
    tracing::info!("Created backup '{backup_id}'.");
    tracing::info!("Integrity checks: {digest_ids:#?}");

    println!();
    let backups = service.list_backups().await.unwrap();
    tracing::info!("Backups: {backups:#?}");
    assert_eq!(backups.len(), 1);

    println!();
    service
        .restore_backup(&backup_id, &blueprint)
        .await
        .unwrap();

    println!();
    () = service.delete_backup(&backup_id).await.unwrap();
}

/// Ensures that restoring a backup works if a backup archive contains an
/// unknown entry. There might be use cases for it, so the library shouldn’t
/// prevent that. Instead, it logs a warning (not tested because annoying).
#[tokio::test(flavor = "multi_thread")]
async fn alternate_path_unknown_archive_entry_no_error() {
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

    let mut blueprint = ArchiveBlueprint::from_iter([("foo-data", "foo")].into_iter())
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
async fn alternate_path_lost_signing_key() {
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

            [storage]
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

    let backup_version: u8 = 1;
    let blueprints = BlueprintsBuilder::new()
        .insert(backup_version, blueprint.clone())
        .build();

    println!();
    let certs: HashMap<PathBuf, openpgp::Cert> =
        make_test_certs([("sign.pgp", now - Duration::from_hours(23))]).unwrap();
    save_certs(test_data_path, &certs);

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
            version: backup_version,
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

/// Ensures one can change the hashing algorithm without breaking older backups
/// created using the default one. For better generalization, it tests the
/// other way around too.
#[tokio::test(flavor = "multi_thread")]
async fn alternate_path_change_hashing_algorithm() {
    let context = init();
    let TestContext {
        now,
        ref test_data_path,
        ..
    } = context;

    let blueprint = ArchiveBlueprint::from_iter([("foo-data", "foo")].into_iter())
        .src_relative_to(&test_data_path);

    create_files(&test_data_path, ["foo/", "foo/a"]).unwrap();

    async fn make(
        algorithm: &'static str,
        test_data_path: impl AsRef<Path>,
        blueprint: &ArchiveBlueprint,
        created_at: SystemTime,
    ) -> (BackupService, BackupId) {
        let backup_config = {
            let mut toml = toml! {
                [hashing]
                algorithm = algorithm

                [storage]
                provider = "fs"
                fs.directory = "store"
            };

            map_storage_directories_in_test_dir(&mut toml, test_data_path).unwrap();

            BackupConfig::try_from(toml).unwrap()
        };

        let backup_version: u8 = 1;
        let blueprints = BlueprintsBuilder::new()
            .insert(backup_version, blueprint.clone())
            .build();

        let service = BackupService::from_config_custom(
            &backup_config,
            ArchivingContext { blueprints },
            |_| unreachable!(),
            || unreachable!() as openpgp::policy::StandardPolicy,
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
                blueprint,
                additional_archive_data: vec![],
                created_at,
            };
            service.create_backup(command).await.unwrap()
        };
        let CreateBackupOutput { backup_id, .. } = creation_output;

        (service, backup_id)
    }

    let (blake3_service, blake3_backup_id) = make(
        "BLAKE3",
        test_data_path,
        &blueprint,
        now - Duration::from_mins(90),
    )
    .await;
    let (sha256_service, sha256_backup_id) = make(
        "SHA-256",
        test_data_path,
        &blueprint,
        now - Duration::from_mins(60),
    )
    .await;

    blake3_service
        .restore_backup(&sha256_backup_id, &blueprint)
        .await
        .unwrap();
    sha256_service
        .restore_backup(&blake3_backup_id, &blueprint)
        .await
        .unwrap();
}
