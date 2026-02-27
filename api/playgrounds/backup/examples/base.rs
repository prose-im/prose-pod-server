// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::{
    collections::HashMap,
    fs,
    io::Read as _,
    path::{Path, PathBuf},
    sync::Arc,
};

use bytes::Bytes;
use prose_backup::{
    BackupService, CreateBackupOutput,
    config::{EncryptionMode, HashingAlgorithm, *},
    decryption::{DecryptionHelper, GpgDecryptionHelper},
    encryption::EncryptionContext,
    openpgp,
    signing::PgpSigningContext,
    verification::pgp::{PgpVerificationContext, PgpVerificationHelper},
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
    let encryption_config = EncryptionConfig {
        enabled: true,
        mandatory: true,
        mode: EncryptionMode::Gpg,
        gpg: Some(EncryptionGpgConfig {
            key: Path::new("cert1").to_path_buf(),
            additional_encryption_keys: vec![],
            additional_decryption_keys: vec![],
        }),
    };
    // let encryption_config = None;
    let hashing_config = HashingConfig {
        algorithm: HashingAlgorithm::Sha256,
    };
    let signing_config = Some(SigningConfig {
        mandatory: false,
        pgp: Some(SigningPgpConfig {
            key: Path::new("cert2").to_path_buf(),
            additional_encryption_keys: vec![],
            additional_decryption_keys: vec![],
        }),
    });
    // let signing_config = None;

    let certs: HashMap<PathBuf, openpgp::Cert> = [
        (Path::new("cert1").to_path_buf(), generate_test_cert()?),
        (Path::new("cert2").to_path_buf(), generate_test_cert()?),
    ]
    .into_iter()
    .collect();

    let pgp_policy = openpgp::policy::StandardPolicy::new();

    let encryption_context = if encryption_config.enabled {
        match encryption_config.gpg.as_ref() {
            Some(pgp) => {
                let pgp_cert = certs.get(&pgp.key).unwrap();
                Some(EncryptionContext::Gpg {
                    cert: &pgp_cert,
                    policy: &pgp_policy,
                })
            }
            None => None,
        }
    } else {
        None
    };
    let pgp_signing_context = match signing_config.as_ref() {
        Some(config) => match config.pgp.as_ref() {
            Some(pgp) => {
                let pgp_cert = certs.get(&pgp.key).unwrap();
                Some(PgpSigningContext {
                    cert: &pgp_cert,
                    policy: &pgp_policy,
                })
            }
            None => None,
        },
        None => None,
    };
    let pgp_verification_context = match signing_config.as_ref() {
        Some(config) => match config.pgp.as_ref() {
            Some(pgp) => {
                let pgp_cert = certs.get(&pgp.key).unwrap();
                Some(PgpVerificationContext {
                    helper: PgpVerificationHelper { cert: &pgp_cert },
                    policy: &pgp_policy,
                })
            }
            None => None,
        },
        None => None,
    };
    let decryption_helper = if encryption_config.enabled {
        match encryption_config.gpg.as_ref() {
            Some(pgp) => {
                let pgp_cert = certs.get(&pgp.key).unwrap();
                let mut helper = DecryptionHelper::default();
                helper.gpg = Some(GpgDecryptionHelper {
                    cert: pgp_cert.to_owned(),
                    policy: Arc::new(pgp_policy.to_owned()),
                });
                helper
            }
            None => DecryptionHelper::default(),
        }
    } else {
        DecryptionHelper::default()
    };

    let fs_prefix = Path::new(".out");

    let fs_prefix_backups = fs_prefix.join("backups");
    fs::create_dir_all(&fs_prefix_backups)?;
    let backup_store = prose_backup::stores::Fs::default()
        .overwrite(true)
        .directory(&fs_prefix_backups);

    let fs_prefix_integrity_checks = fs_prefix.join("checks");
    fs::create_dir_all(&fs_prefix_integrity_checks)?;
    let check_store = prose_backup::stores::Fs::default()
        .overwrite(true)
        .directory(&fs_prefix_integrity_checks);

    let service = BackupService {
        fs_root: PathBuf::from("./data"),
        archiving_config,
        compression_config,
        hashing_config,
        signing_config,
        pgp_signing_context,
        encryption_context,
        backup_store,
        check_store,
        pgp_verification_context,
        decryption_helper,
    };

    let CreateBackupOutput {
        backup_id,
        digest_ids,
        signature_ids,
    } = {
        let backup_name = "backup";
        service
            .create_backup(backup_name, prose_pod_api_data)
            .await?
    };
    let integrity_check_file_name = digest_ids
        .first()
        .expect("At least one digest should have been created");
    tracing::info!("Created backup '{backup_id}'.");

    print!("\n");
    let backups = service.list_backups().await?;
    tracing::info!("Backups: {backups:#?}");

    print!("\n");
    let fs_prefix_extract = fs_prefix.join("extract");
    std::fs::create_dir_all(&fs_prefix_extract)?;
    let mut restore_result = service
        .restore_backup(&backup_id, fs_prefix_extract)
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
        fs::remove_file(fs_prefix_backups.join(backup_id))?;
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
