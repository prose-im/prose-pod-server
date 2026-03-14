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
use tokio::sync::RwLockReadGuard;

pub use self::v2::start_v2;

// MARK: - Public API

#[async_trait::async_trait]
pub trait ProsePodApi {
    /// `POST /backups`.
    async fn post_backups(&self, description: String)
    -> Result<CreateBackupSuccess, anyhow::Error>;

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

    /// `GET /backups/{backup_id}/download-url`.
    async fn get_backup_download_url(
        &self,
        backup_id: String,
        ttl: std::time::Duration,
    ) -> Result<String, anyhow::Error>;
}
