// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

extern crate sequoia_openpgp as openpgp;

use std::{
    fs,
    io::Read as _,
    path::{Path, PathBuf},
    sync::Arc,
};

use bytes::Bytes;
use prose_backup::{
    ArchivingConfig, BackupService, CompressionConfig, EncryptionConfig, GpgHelper,
    encryption::EncryptionMode,
};

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    tracing_subscriber::fmt()
        .compact()
        .without_time()
        .with_target(false)
        .with_max_level(tracing::Level::TRACE)
        .init();

    let prose_pod_api_data = Bytes::new();

    let archiving_config = ArchivingConfig {
        version: prose_backup::CURRENT_VERSION,
    };
    let compression_config = CompressionConfig {
        zstd_compression_level: 5,
    };
    let gpg_config = Arc::new(GpgHelper::new(generate_test_cert()?));
    let encryption_config = EncryptionConfig {
        enabled: true,
        mandatory: true,
        mode: EncryptionMode::Gpg,
        gpg: gpg_config,
    };
    // let encryption_config = None;
    let integrity_config = Some(EncryptionConfig::new(generate_test_cert()?));
    // let integrity_config = None;

    let fs_prefix = Path::new(".out");

    let fs_prefix_backups = fs_prefix.join("backups");
    fs::create_dir_all(&fs_prefix_backups)?;
    let backup_store = prose_backup::stores::Fs::default()
        .overwrite(true)
        .directory(&fs_prefix_backups);

    let fs_prefix_integrity_checks = fs_prefix.join("integrity-checks");
    fs::create_dir_all(&fs_prefix_integrity_checks)?;
    let integrity_check_store = prose_backup::stores::Fs::default()
        .overwrite(true)
        .directory(&fs_prefix_integrity_checks);

    let service = BackupService {
        fs_root: PathBuf::from("./data"),
        archiving_config,
        compression_config,
        encryption_config,
        integrity_config,
        backup_store,
        check_store,
    };

    let (backup_file_name, integrity_check_file_name) = {
        let backup_name = "backup";
        service
            .create_backup(backup_name, prose_pod_api_data)
            .await?
    };
    tracing::info!("Created backup '{backup_file_name}'.");

    print!("\n");
    let backups = service.list_backups().await?;
    tracing::info!("Backups: {backups:#?}");

    print!("\n");
    let fs_prefix_extract = fs_prefix.join("extract");
    std::fs::create_dir_all(&fs_prefix_extract)?;
    let mut restore_result = service
        .restore_backup(&backup_file_name, fs_prefix_extract)
        .await?;

    print!("\n");
    tracing::info!("Reading Pod API data…");
    let prose_pod_api_data_len = restore_result
        .prose_pod_api_data
        .metadata()
        .map_or(0, |meta| meta.len());
    // NOTE: This `as` is safe to use as we’re working with unsigned integers
    //   and it’s fine if capacity is less than real size. Also there is
    //   no chance we’ll go above `u32::MAX` on a 32-bit target.
    let mut prose_pod_api_data: Vec<u8> = Vec::with_capacity(prose_pod_api_data_len as usize);
    restore_result
        .prose_pod_api_data
        .read_to_end(&mut prose_pod_api_data)?;

    if std::env::var("NO_DELETE").is_err() {
        fs::remove_file(fs_prefix_backups.join(backup_file_name))?;
        fs::remove_file(fs_prefix_integrity_checks.join(integrity_check_file_name))?;
    }

    Ok(())
}

// MARK: - Helpers

fn generate_test_cert() -> Result<openpgp::Cert, anyhow::Error> {
    use openpgp::cert::CertBuilder;

    // Build a cert with user ID + primary key + subkey
    let (cert, _signature) = CertBuilder::new()
        .add_userid("Test User <test@example.org>")
        .add_signing_subkey()
        .add_storage_encryption_subkey()
        .set_validity_period(std::time::Duration::from_secs(3600))
        .generate()?;

    Ok(cert)
}
