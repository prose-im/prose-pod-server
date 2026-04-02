// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! Ensure the library output is valid and compatible with outside tooling.
//!
//! The library could _work_ without being _correct_ (it happened at some point
//! during development). These tests ensure things like OpenPGP signatures
//! are correct and can be verified with the appropriate tooling.

mod common;

use std::process::{Command, Stdio};

use crate::common::prelude::*;

/// Ensures that OpenPGP signatures and backup encryption are valid and can be
/// processed using external OpenPGP tooling (here, `sequoia-sq`).
#[tokio::test(flavor = "multi_thread")]
async fn compatibility_openpgp() {
    let context = init();
    let TestContext {
        now,
        ref test_data_path,
        ..
    } = context;

    println!();
    let backup_config = {
        let mut toml = toml! {
            [encryption]
            mode = "pgp"
            pgp.tsk = "cert.tsk"

            [signing]
            pgp.enabled = true
            pgp.tsk = "cert.tsk"

            [storage]
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
    let certs: HashMap<PathBuf, openpgp::Cert> =
        make_test_certs([("cert.tsk", now - Duration::from_hours(23))]).unwrap();
    save_certs(test_data_path, &certs);

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
        service
            .create_backup(command, &mut DebugEventHandler::default())
            .await
            .unwrap()
    };
    let CreateBackupOutput {
        backup_id,
        signature_ids,
        ..
    } = creation_output;

    let backup_path = test_data_path.join("store").join(&backup_id.to_string());
    let cert_path = test_data_path.join("certs").join("cert.tsk");

    // Check backup.
    {
        run_command(
            format!(
                "Verifying backup at `{path}`",
                path = backup_path.display().to_string()
            ),
            Command::new("sq")
                .arg("decrypt")
                .arg("--recipient-file")
                .arg(&cert_path)
                .arg(&backup_path)
                .stdout(Stdio::null()),
        );
        tracing::info!(
            "Encrypted backup `{path}` is valid.",
            path = backup_path.display().to_string()
        );
        sq_packet_dump(&backup_path, &cert_path);
    }

    // Check signature(s).
    for signature_id in signature_ids {
        let signature_path = test_data_path.join("store").join(&signature_id.to_string());

        run_command(
            format!(
                "Verifying signature at `{path}`",
                path = signature_path.display().to_string()
            ),
            Command::new("sq")
                .arg("verify")
                .arg("--signer-file")
                .arg(&cert_path)
                .arg("--signature-file")
                .arg(&signature_path)
                .arg(&backup_path),
        );
        tracing::info!(
            "Signature `{path}` is valid.",
            path = signature_path.display().to_string()
        );
        sq_packet_dump(&signature_path, &cert_path);
    }
}
