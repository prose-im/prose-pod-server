// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! An abstraction of the Prose Pod API.

pub mod v2;

use std::collections::HashMap;

use prose_backup::{
    CreateBackupSuccess,
    dtos::{BackupDto, BackupMetadataFullDto, BackupMetadataPartialDto},
};
use tokio::sync::{RwLockReadGuard, mpsc};

pub use self::v2::start_v2;

// MARK: - Public API

#[async_trait::async_trait]
pub trait ProsePodApi: Send + Sync {
    /// `POST /backups`.
    async fn post_backups(&self, description: String)
    -> Result<CreateBackupSuccess, anyhow::Error>;

    /// `POST /backups Accept: text/event-stream`.
    async fn post_backups_stream(
        &self,
        description: String,
    ) -> Result<mpsc::Receiver<CreateBackupEvent>, anyhow::Error>;

    /// `GET /backups`.
    async fn get_backups(&self) -> Result<Vec<BackupDto<BackupMetadataPartialDto>>, anyhow::Error>;

    /// `GET /backups/{backup_id}`.
    async fn get_backup(
        &self,
        backup_id: String,
    ) -> Result<BackupDto<BackupMetadataFullDto>, anyhow::Error>;

    /// `DELETE /backups/{backup_id}`.
    async fn delete_backup(&self, backup_id: String) -> Result<(), anyhow::Error>;

    /// `PUT /backups/{backup_id}/restore`.
    async fn put_backup_restore(&self, backup_id: String) -> Result<(), anyhow::Error>;

    /// `PUT /backups/{backup_id}/restore Accept: text/event-stream`.
    async fn put_backup_restore_stream(
        &self,
        backup_id: String,
    ) -> Result<mpsc::Receiver<RestoreBackupEvent>, anyhow::Error>;

    /// `GET /backups/{backup_id}/download-url`.
    async fn get_backup_download_url(
        &self,
        backup_id: String,
        ttl: std::time::Duration,
    ) -> Result<String, anyhow::Error>;
}

pub enum CreateBackupEvent {
    Progress { progress: u64, total: u64 },
    End(Result<CreateBackupSuccess, anyhow::Error>),
}

pub enum RestoreBackupEvent {
    Progress { progress: u64, total: u64 },
    End(Result<(), anyhow::Error>),
}

// MARK: - Helpers

fn init_tsks(fs_root: impl AsRef<std::path::Path>) -> Result<(), anyhow::Error> {
    use anyhow::Context as _;
    use openpgp::serialize::Serialize as _;
    use std::time::SystemTime;

    let fs_root = fs_root.as_ref();

    fn generate_test_cert() -> Result<openpgp::Cert, anyhow::Error> {
        use openpgp::cert::CertBuilder;
        use std::time::Duration;

        let created_at = SystemTime::now() - Duration::from_hours(3);
        let validity = Duration::from_hours(24);

        // Build a TSK with user ID + primary key + subkey
        let (tsk, _signature) = CertBuilder::new()
            .set_profile(openpgp::Profile::RFC9580)?
            .add_userid("Test User <test@example.org>")
            .set_creation_time(created_at)
            .set_validity_period(validity)
            .add_signing_subkey()
            .add_storage_encryption_subkey()
            .generate()?;
        tracing::debug!(
            "Created TSK `{tsk}` valid from {} to {}.",
            time::UtcDateTime::from(created_at),
            time::UtcDateTime::from(created_at + validity)
        );

        Ok(tsk)
    }

    let cert = generate_test_cert()?;

    let certs_path = fs_root.join("usr/share/prose/certs");
    std::fs::create_dir_all(&certs_path).context(format!("Dir: {certs_path:?}"))?;

    let cert_path = certs_path.join("example.tsk");
    let mut file = std::fs::File::create_new(&cert_path).context(format!("File: {cert_path:?}"))?;
    cert.as_tsk().serialize(&mut file)?;

    Ok(())
}

fn init_prose_config(fs_root: impl AsRef<std::path::Path>) -> Result<(), anyhow::Error> {
    use anyhow::Context as _;
    use std::io::Write as _;
    use toml::toml;

    let fs_root = fs_root.as_ref();

    let pgp_tsk_path = fs_root
        .join("usr/share/prose/certs/example.tsk")
        .display()
        .to_string();
    let pgp_tsk_path = pgp_tsk_path.as_str();

    let config = toml! {
        [backups.encryption]
        mode = "pgp"
        pgp.tsk = pgp_tsk_path

        [backups.signing]
        pgp.enabled = true
        pgp.tsk = pgp_tsk_path

        [backups.storage]
        provider = "s3"
    };

    let config_path = fs_root.join("etc/prose/prose.toml");
    let mut config_file =
        std::fs::File::create(&config_path).context(format!("File: {config_path:?}"))?;
    config_file
        .write_all(config.to_string().as_bytes())
        .context(format!("File: {config_path:?}"))?;

    Ok(())
}
