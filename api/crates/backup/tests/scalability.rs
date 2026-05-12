// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! Tests that the library scales properly (e.g. very large files).
//!
//! It’s not stress tests, as we don’t apply much load. It just ensures the
//! library doesn’t have virtual limits we hadn’t noticed.

mod common;

use std::process::Command;

use crate::common::prelude::*;

/// Ensures that backing up and restoring both work with very large files.
#[tokio::test(flavor = "multi_thread")]
async fn scalability_large_files() {
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

        BackupConfig::try_from(toml)
    }
    .unwrap();
    tracing::info!("Parsed config: {backup_config:#?}");

    let blueprint =
        ArchiveBlueprint::new(1, [("foo-data", "foo")]).src_relative_to(&test_data_path);

    println!();
    tracing::info!("Creating test files…");
    create_files(&test_data_path, ["foo/"]).unwrap();
    let dd_status = Command::new("dd")
        .arg("if=/dev/urandom")
        .arg("of=foo/example.bin")
        .arg("bs=1M")
        .arg("count=1024")
        .current_dir(test_data_path)
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(dd_status.success());

    let blueprints = BlueprintsBuilder::new().insert(blueprint.clone()).build();

    println!();
    tracing::info!("Creating service…");
    let service = BackupService::from_config_custom(
        &backup_config,
        ArchivingContext { blueprints },
        RestorationContext { migrations: vec![] },
        |_| unreachable!(),
        || -> openpgp::policy::StandardPolicy { unreachable!() },
    )
    .unwrap();

    println!();
    tracing::info!("Creating backup…");
    let mut creation_event_handler = DebugCreateBackupEventHandler::default();
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
            .create_backup(command, &mut creation_event_handler)
            .await
            .unwrap()
    };
    let CreateBackupOutput { backup_id, .. } = creation_output;
    tracing::info!(
        "Upload stats: {upload_stats:#?}",
        upload_stats = creation_event_handler.upload_durations
    );

    println!();
    tracing::info!("Restoring backup…");
    let mut extraction_event_handler = DebugExtractBackupEventHandler::default();
    let RestoreBackupSuccess {
        verification_report,
        ..
    } = service
        .restore_backup(&backup_id, &blueprint, &mut extraction_event_handler)
        .await
        .unwrap();
    tracing::info!("verification_report: {verification_report:#?}");
    tracing::info!(
        "decryption_report: {:#?}",
        extraction_event_handler.decryption_report
    );
    print_stats(&extraction_event_handler);
}
