// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! Tests happy paths of all features given multiple configurations.
//!
//! Essentially ensures that the library works as intended and all features it
//! says are supported really are and work.

mod common;

use prose_backup::event_handlers::NoopEventHandler;

use crate::common::prelude::*;

#[tokio::test(flavor = "multi_thread")]
async fn happy_path_noenc_nosign() {
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
async fn happy_path_enc_pgp_nosign() {
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
async fn happy_path_noenc_sign_pgp() {
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
async fn happy_path_enc_pgp_sign_pgp() {
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

/// Tests that backup restorations are atomic.
#[tokio::test(flavor = "multi_thread")]
async fn happy_path_atomic_restore() {
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

    create_files(
        &test_data_path,
        [
            "foo/", "foo/a", "bar/", "bar/a",
        ],
    )
    .unwrap();

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
            version: backup_version,
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

    {
        // Override `foo/a`.
        std::fs::write(&foo_a, "overriden").unwrap();

        // Restore the backup.
        println!();
        let res = service
            .restore_backup(&backup_id, &blueprint, &mut NoopEventHandler)
            .await;
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
        let res = service
            .restore_backup(&backup_id, &blueprint, &mut NoopEventHandler)
            .await;
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

/// Tests that backup restorations restore file permissions.
#[tokio::test(flavor = "multi_thread")]
async fn happy_path_file_permissions() {
    use std::{fs::Permissions, os::unix::fs::PermissionsExt as _};

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

    let blueprint = ArchiveBlueprint::from_iter([("foo-data", "foo")].into_iter())
        .src_relative_to(&test_data_path);

    create_files(
        &test_data_path,
        [
            "foo/", "foo/a", "foo/b",
        ],
    )
    .unwrap();

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

    // Set some custom permissions.
    // NOTE: Uses two different files to ensure the library works and we’re not
    //   just using a default value.
    let foo_a = test_data_path.join("foo/a");
    let foo_a_permissions = Permissions::from_mode(0o100401);
    std::fs::set_permissions(&foo_a, foo_a_permissions.clone()).unwrap();
    let foo_b = test_data_path.join("foo/b");
    let foo_b_permissions = Permissions::from_mode(0o100402);
    std::fs::set_permissions(&foo_b, foo_b_permissions.clone()).unwrap();

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
            additional_archive_data: Option::<()>::None,
            created_at: now - Duration::from_mins(90),
        };
        service
            .create_backup(command, &mut NoopEventHandler)
            .await
            .unwrap()
    };
    let CreateBackupOutput { backup_id, .. } = creation_output;

    // Override permissions.
    std::fs::set_permissions(&foo_a, Permissions::from_mode(0o100600)).unwrap();
    std::fs::set_permissions(&foo_a, Permissions::from_mode(0o100600)).unwrap();

    // Restore the backup.
    println!();
    let res = service
        .restore_backup(&backup_id, &blueprint, &mut NoopEventHandler)
        .await;
    assert!(res.is_ok(), "Error: {:#?}", res.err().unwrap());

    // Test that permissions were reverted.
    assert_eq!(
        std::fs::metadata(&foo_a).unwrap().permissions(),
        foo_a_permissions
    );
    assert_eq!(
        std::fs::metadata(&foo_b).unwrap().permissions(),
        foo_b_permissions
    );
}

// MARK: - Helpers

/// Tests all features of the library, given a configuration.
async fn test_happy_path_(mut config_toml: toml::Table) {
    let context = init();
    let TestContext {
        now,
        ref test_data_path,
        ..
    } = context;

    map_storage_directories_in_test_dir(&mut config_toml, test_data_path).unwrap();

    println!();
    let backup_config = BackupConfig::try_from(config_toml)
        .context("BackupConfig::try_from")
        .unwrap();
    tracing::info!("Parsed config: {backup_config:#?}");

    // Ensure compression is enabled. Most tests don’t use it for simplicity
    // but it would very likely be enabled in production apps.
    assert!(matches!(
        backup_config.compression,
        CompressionConfig::Zstd { .. }
    ));

    let blueprint = ArchiveBlueprint::from_iter(
        [
            ("foo-data", "foo"),
            ("bar-data", "bar"),
        ]
        .into_iter(),
    )
    .src_relative_to(&test_data_path);

    #[rustfmt::skip]
    create_files(
        &test_data_path,
        [
            "foo/", "foo/a",
            // NOTE: Tests that a standalone file can be archived too.
            "bar",
        ],
    ).unwrap();

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
    save_certs(test_data_path, &certs);

    let pgp_policy = openpgp::policy::StandardPolicy::new();

    let signing_config = backup_config.signing.clone();
    let encryption_config = backup_config.encryption.clone();

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
    .context("BackupService::from_config_custom")
    .unwrap();

    println!();
    let mut creation_event_handler = DebugCreateBackupEventHandler::default();
    let CreateBackupSuccess {
        output: creation_output,
        ..
    } = {
        let command = CreateBackupCommand {
            prefix: "prose-backup",
            description: "Test backup",
            version: backup_version,
            blueprint: &blueprint.clone(),
            additional_archive_data: Option::<()>::None,
            created_at: now - Duration::from_mins(90),
        };
        service
            .create_backup(command, &mut creation_event_handler)
            .await
            .context("create_backup")
            .unwrap()
    };
    let CreateBackupOutput {
        backup_id,
        digest_ids,
        ..
    } = creation_output;
    tracing::info!("Created backup '{backup_id}'.");
    tracing::info!("Integrity checks: {digest_ids:#?}");
    tracing::debug!(
        "Effective archive size: {}",
        creation_event_handler.effective_archive_size
    );
    assert_eq!(
        creation_event_handler.expected_archive_size,
        creation_event_handler.effective_archive_size
    );

    println!();
    if let EncryptionConfig::Pgp { config: pgp } = &encryption_config {
        let mut pgp_cert = certs.get(&pgp.tsk).unwrap().clone();

        pgp_cert = revoke_subkey_simple(
            pgp_cert,
            |keys| keys.for_storage_encryption(),
            SystemTime::now() - Duration::from_mins(10),
            openpgp::types::ReasonForRevocation::KeySuperseded,
        )
        .unwrap();

        service.decryption_context.pgp = Some(PgpDecryptionContext {
            tsks: vec![pgp_cert],
            policy: Box::new(pgp_policy.clone()),
        });
    }

    println!();
    let backups = service
        .list_backups()
        .await
        .context("list_backups")
        .unwrap();
    tracing::info!("Backups: {backups:#?}");

    println!();
    let details = service
        .get_details(&backup_id)
        .await
        .context("get_details")
        .unwrap();
    tracing::info!("Backup details: {details:#?}");

    println!();
    let download_url = service
        .get_download_url(&backup_id, Duration::from_secs(3))
        .await
        .context("get_download_url")
        .unwrap();
    tracing::info!("Download URL: <{download_url}>.");

    println!();
    let mut extraction_event_handler = DebugExtractBackupEventHandler::default();
    let ExtractionSuccess {
        extraction_output,
        verification_report,
        ..
    } = service
        .extract_backup(&backup_id, &mut extraction_event_handler)
        .await
        .context("extract_backup")
        .unwrap();
    print_stats(&extraction_event_handler);
    if let Some(SigningPgpConfig { tsk, .. }) = &signing_config.pgp {
        let pgp_cert = certs.get(tsk).unwrap().clone();

        assert!(!verification_report.known_signing_keys.is_empty());
        verification_report
            .known_signing_keys
            .iter()
            .all(|report| report.cert_fingerprint == pgp_cert.fingerprint());
    }

    println!();
    service
        .restore_extracted_backup(
            &backup_id,
            extraction_output,
            &blueprint,
            &mut NoopEventHandler,
        )
        .await
        .context("restore_extracted_backup")
        .unwrap();

    println!();
    () = service
        .delete_backup(&backup_id)
        .await
        .context("delete_backup")
        .unwrap();
}
