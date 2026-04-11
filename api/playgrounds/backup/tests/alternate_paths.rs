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

    let blueprint =
        ArchiveBlueprint::new(1, [("foo-data", "foo")]).src_relative_to(&test_data_path);

    create_files(&test_data_path, ["foo/", "foo/a"]).unwrap();

    let blueprints = BlueprintsBuilder::new().insert(blueprint.clone()).build();

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
        RestorationContext { migrations: vec![] },
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
            blueprint: &blueprint.clone(),
            additional_archive_data: Option::<()>::None,
            created_at: now - Duration::from_mins(90),
        };
        service
            .create_backup(command, &mut NoopEventHandler)
            .await
            .unwrap()
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
        .restore_backup(&backup_id, &blueprint, &mut NoopEventHandler)
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

    let mut blueprint =
        ArchiveBlueprint::new(1, [("foo-data", "foo")]).src_relative_to(&test_data_path);

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
    blueprint.paths.swap_remove(0);
    service
        .restore_backup(&backup_id, &blueprint, &mut NoopEventHandler)
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

    let blueprint =
        ArchiveBlueprint::new(1, [("foo-data", "foo")]).src_relative_to(&test_data_path);

    create_files(&test_data_path, ["foo/", "foo/a"]).unwrap();

    let blueprints = BlueprintsBuilder::new().insert(blueprint.clone()).build();

    println!();
    let certs: HashMap<PathBuf, openpgp::Cert> =
        make_test_certs([("sign.pgp", now - Duration::from_hours(23))]).unwrap();
    save_certs(test_data_path, &certs);

    let pgp_policy = openpgp::policy::StandardPolicy::new();

    let mut service = BackupService::from_config_custom(
        &backup_config,
        ArchivingContext { blueprints },
        RestorationContext { migrations: vec![] },
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
        println!();
        let ExtractAndRestoreSuccess {
            verification_report,
            ..
        } = service
            .restore_backup(&backup_id, &blueprint, &mut NoopEventHandler)
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
            .restore_backup(&backup_id, &blueprint, &mut NoopEventHandler)
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

    let blueprint =
        ArchiveBlueprint::new(1, [("foo-data", "foo")]).src_relative_to(&test_data_path);

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

        let blueprints = BlueprintsBuilder::new().insert(blueprint.clone()).build();

        let service = BackupService::from_config_custom(
            &backup_config,
            ArchivingContext { blueprints },
            RestorationContext { migrations: vec![] },
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
                blueprint,
                additional_archive_data: Option::<()>::None,
                created_at,
            };
            service
                .create_backup(command, &mut NoopEventHandler)
                .await
                .unwrap()
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
        .restore_backup(&sha256_backup_id, &blueprint, &mut NoopEventHandler)
        .await
        .unwrap();
    sha256_service
        .restore_backup(&blake3_backup_id, &blueprint, &mut NoopEventHandler)
        .await
        .unwrap();
}

/// Ensures backups can be restored even if they are older than one version old.
/// Naive migrations could only support migrating from v1 to v2 fr example.
/// This ensures one can migrate from v1 to v3.
/// Note that this doesn’t test v3 to v1.
#[tokio::test(flavor = "multi_thread")]
async fn alternate_path_transitive_migrations() {
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

    let blueprints = BlueprintsBuilder::new()
        .insert(
            ArchiveBlueprint::new(
                1,
                [
                    ("foo1-data", "foo1"),
                    ("foo1-data.d", "foo1.d"),
                ],
            )
            .src_relative_to(&test_data_path),
        )
        .insert(
            ArchiveBlueprint::new(
                2,
                [
                    ("foo2-data", "foo2"),
                    ("foo2-data.d", "foo2.d"),
                ],
            )
            .src_relative_to(&test_data_path),
        )
        .insert(
            ArchiveBlueprint::new(
                3,
                [
                    ("foo3-data", "foo3"),
                    ("foo3-data.d", "foo3.d"),
                ],
            )
            .src_relative_to(&test_data_path),
        )
        .insert(
            ArchiveBlueprint::new(
                4,
                [
                    ("foo4-data", "foo4"),
                    ("foo4-data.d", "foo4.d"),
                ],
            )
            .src_relative_to(&test_data_path),
        )
        .insert(
            ArchiveBlueprint::new(
                5,
                [
                    ("foo5-data", "foo5"),
                    ("foo5-data.d", "foo5.d"),
                ],
            )
            .src_relative_to(&test_data_path),
        )
        .insert(
            ArchiveBlueprint::new(
                6,
                [
                    ("foo6-data", "foo6"),
                    ("foo6-data.d", "foo6.d"),
                ],
            )
            .src_relative_to(&test_data_path),
        )
        .build();

    create_files(
        &test_data_path,
        [
            "foo2", "foo2.d/", "foo2.d/a",
        ],
    )
    .unwrap();

    let service = BackupService::from_config_custom(
        &backup_config,
        ArchivingContext {
            blueprints: blueprints.clone(),
        },
        RestorationContext {
            migrations: vec![
                ArchiveMigration::new(
                    2,
                    [
                        ("foo1-data", "foo2-data"),
                        ("foo1-data.d", "foo2-data.d"),
                    ],
                ),
                ArchiveMigration::new(
                    3,
                    [
                        ("foo2-data", "foo3-data"),
                        ("foo2-data.d", "foo3-data.d"),
                    ],
                ),
                ArchiveMigration::new(
                    4,
                    [
                        ("foo3-data", "foo4-data"),
                        ("foo3-data.d", "foo4-data.d"),
                    ],
                ),
                ArchiveMigration::new(
                    5,
                    [
                        ("foo4-data", "foo5-data"),
                        ("foo4-data.d", "foo5-data.d"),
                    ],
                ),
            ],
        },
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
            blueprint: &blueprints.get(&2).unwrap(),
            additional_archive_data: Option::<()>::None,
            created_at: now - Duration::from_mins(90),
        };
        service
            .create_backup(command, &mut NoopEventHandler)
            .await
            .unwrap()
    };
    let CreateBackupOutput { backup_id, .. } = creation_output;

    // Restoring version 2 to 4 should work.
    // NOTE: This has the side effect of ensuring we don’t apply unnecessary
    //   migrations (1 to 2 and 4 to 5).
    service
        .restore_backup(
            &backup_id,
            blueprints.get(&4).unwrap(),
            &mut NoopEventHandler,
        )
        .await
        .unwrap();

    // Restoring version 2 to 6 should fail because no migration is known
    // from 5 to 6.
    let res = service
        .restore_backup(
            &backup_id,
            blueprints.get(&6).unwrap(),
            &mut NoopEventHandler,
        )
        .await;
    assert!(res.is_err());
}

/// Ensures the library supports passphrase-protected OpenPGP secret keys.
#[tokio::test(flavor = "multi_thread")]
async fn alternate_path_openpgp_encrypted_secret_key() {
    let context = init();
    let TestContext {
        now,
        ref test_data_path,
        ..
    } = context;

    println!();
    let cert_password = "password";
    let cert = generate_test_cert(now - Duration::from_hours(23), |cert| {
        cert.set_password(Some(cert_password.into()))
    })
    .unwrap();
    let certs: HashMap<PathBuf, openpgp::Cert> =
        [("tsk.pgp".into(), cert.clone())].into_iter().collect();
    save_certs(test_data_path, &certs);

    println!();
    let backup_config = {
        let pgp_passphrases = [(cert.fingerprint().to_string(), cert_password)]
            .into_iter()
            .collect::<HashMap<_, _>>();
        let mut toml = toml! {
            [encryption]
            mode = "pgp"
            pgp.tsk = "tsk.pgp"

            [signing]
            mandatory = false
            pgp.enabled = true
            pgp.tsk = "tsk.pgp"

            [storage]
            provider = "fs"
            fs.directory = "store"

            [pgp]
            // WARN: In a real app, pass this via environment variables!
            passphrases = pgp_passphrases
        };

        map_storage_directories_in_test_dir(&mut toml, test_data_path).unwrap();

        BackupConfig::try_from(toml).unwrap()
    };
    tracing::debug!("Parsed config: {backup_config:#?}");

    let blueprint =
        ArchiveBlueprint::new(1, [("foo-data", "foo")]).src_relative_to(&test_data_path);

    create_files(&test_data_path, ["foo/", "foo/a"]).unwrap();

    let blueprints = BlueprintsBuilder::new().insert(blueprint.clone()).build();

    let pgp_policy = openpgp::policy::StandardPolicy::new();

    let mut service = BackupService::from_config_custom(
        &backup_config,
        ArchivingContext { blueprints },
        RestorationContext { migrations: vec![] },
        |path| {
            certs
                .get(path)
                .cloned()
                .ok_or(anyhow!("Unknown cert: `{}`.", path.display()))
        },
        || pgp_policy.clone(),
    )
    .unwrap();

    // Test backup creation (happy path).
    println!();
    let backup_id = service
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
        .await
        .unwrap()
        .output
        .backup_id;

    // Test error on signing if passhrase missing.
    {
        println!();
        let pw = service
            .signing_context
            .pgp
            .as_mut()
            .unwrap()
            .passphrases
            .remove(&cert.fingerprint())
            .unwrap();

        let res = service
            .create_backup(
                CreateBackupCommand {
                    prefix: "prose-backup",
                    description: "Test backup 2",
                    blueprint: &blueprint.clone(),
                    additional_archive_data: Option::<()>::None,
                    created_at: now - Duration::from_mins(90),
                },
                &mut NoopEventHandler,
            )
            .await;
        assert!(res.is_err());
        assert_eq!(
            format!("{:#}", res.err().unwrap()),
            "Cannot sign".to_owned()
        );

        service
            .signing_context
            .pgp
            .as_mut()
            .unwrap()
            .passphrases
            .insert(cert.fingerprint(), pw);
    }

    // Test error on restoration if passhrase missing.
    {
        println!();
        let pw = service
            .decryption_context
            .pgp
            .as_mut()
            .unwrap()
            .passphrases
            .remove(&cert.fingerprint())
            .unwrap();

        let res = service
            .restore_backup(&backup_id, &blueprint.clone(), &mut NoopEventHandler)
            .await;
        assert!(res.is_err());
        assert_eq!(
            format!("{:#}", res.err().unwrap()),
            "Extraction failed".to_owned()
        );

        service
            .decryption_context
            .pgp
            .as_mut()
            .unwrap()
            .passphrases
            .insert(cert.fingerprint(), pw);
    }
}
