// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

#[allow(dead_code, unused_imports, unused_macros)]
mod common;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::anyhow;
use prose_backup::{
    BackupConfig, BackupService, CreateBackupCommand, CreateBackupOutput, CreateBackupSuccess,
    archiving,
};
use toml::toml;

use crate::common::prelude::*;

/// Tests that the library works even if people use the same bucket and prefix
/// as storage for backups and checks.
///
/// This test uses the `fs` storage provider for faster execution time, but
/// it’d be the same with S3.
#[tokio::test(flavor = "multi_thread")]
async fn single_store() -> Result<(), anyhow::Error> {
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

        BackupConfig::try_from(toml)
    }?;
    tracing::info!("Parsed config: {backup_config:#?}");

    let blueprints = test_blueprints();
    let local_data_blueprint = blueprints.get(&BLUEPRINT_LOCAL_DATA).unwrap();
    let blueprint = &local_data_blueprint.src_relative_to(Path::new(TEST_DATA_DIR));
    let restore_blueprint = &local_data_blueprint.src_relative_to(test_data_path.join("restore"));

    println!();
    let certs: HashMap<PathBuf, openpgp::Cert> = make_test_certs([
        ("encrypt.pgp", now - Duration::from_hours(23)),
        ("sign.pgp", now - Duration::from_hours(23)),
    ])?;

    let pgp_policy = openpgp::policy::StandardPolicy::new();

    let service = BackupService::from_config_custom(
        &backup_config,
        archiving::Context { blueprints },
        |path| {
            certs
                .get(path)
                .cloned()
                .ok_or(anyhow!("Unknown cert: `{}`.", path.display()))
        },
        || pgp_policy.clone(),
    )?;

    println!();
    let CreateBackupSuccess {
        output: creation_output,
        ..
    } = {
        let command = CreateBackupCommand {
            prefix: "prose-backup",
            description: "Test backup",
            version: BLUEPRINT_LOCAL_DATA,
            blueprint,
            additional_archive_data: vec![],
            created_at: now - Duration::from_mins(90),
        };
        service.create_backup(command).await?
    };
    let CreateBackupOutput {
        backup_id,
        digest_ids,
        ..
    } = creation_output;
    tracing::info!("Created backup '{backup_id}'.");
    tracing::info!("Integrity checks: {digest_ids:#?}");

    println!();
    let backups = service.list_backups().await?;
    tracing::info!("Backups: {backups:#?}");

    println!();
    service
        .restore_backup(&backup_id, restore_blueprint)
        .await?;

    println!();
    () = service.delete_backup(&backup_id).await?;

    Ok(())
}
