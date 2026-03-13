// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! An abstraction of the Prose Pod API.

use std::collections::HashMap;

use prose_backup::{
    BackupConfig, BackupFileName, BackupService, CreateBackupCommand,
    archiving::ArchiveBlueprint,
    dtos::{BackupDto, BackupMetadataFullDto, BackupMetadataPartialDto},
};
use tokio::sync::{RwLock, RwLockReadGuard};

use crate::common::util::*;

// MARK: - Public API

impl Api {
    pub fn start_v1() -> Result<Self, anyhow::Error> {
        let state = ApiState::new_v1()?;

        Ok(Self {
            constants: ApiConstans::v1(),
            state: RwLock::new(state),
        })
    }
}

impl Api {
    /// `POST /backups`.
    pub async fn post_backups(&self, description: String) -> Result<String, anyhow::Error> {
        let state = self.state().await;

        let backups_version = self.constants.backups_version;
        let ref blueprint = self.constants.backup_blueprints[&backups_version];

        state
            .backup_service
            .create_backup(CreateBackupCommand {
                prefix: concat!("example-", env!("CARGO_CRATE_NAME")),
                description: &description,
                version: backups_version,
                blueprint,
                // Just to make rust-analyzer happy…
                #[cfg(feature = "test")]
                created_at: std::time::SystemTime::now(),
            })
            .await?;

        todo!()
    }

    /// `GET /backups`.
    pub async fn get_backups(
        &self,
    ) -> Result<Vec<BackupDto<BackupMetadataPartialDto>>, anyhow::Error> {
        let state = self.state().await;

        let backups = state.backup_service.list_backups().await?;

        Ok(backups)
    }

    /// `GET /backups/{backup_id}`.
    pub async fn get_backup(
        &self,
        backup_id: String,
    ) -> Result<BackupDto<BackupMetadataFullDto>, anyhow::Error> {
        let state = self.state().await;

        let backup_id = BackupFileName::try_from(&backup_id)?;

        let backup = state.backup_service.get_details(&backup_id).await?;

        Ok(backup)
    }

    /// `DELETE /backups/{backup_id}`.
    pub async fn delete_backup(&self, backup_id: String) -> Result<(), anyhow::Error> {
        let state = self.state().await;

        let backup_id = BackupFileName::try_from(&backup_id)?;

        state.backup_service.delete_backup(&backup_id).await?;

        Ok(())
    }

    /// `PUT /backups/{backup_id}/restore`.
    pub async fn put_backup_restore(&self, backup_id: String) -> Result<(), anyhow::Error> {
        let state = self.state().await;

        todo!()
    }

    /// `GET /backups/{backup_id}/download-url`.
    pub async fn get_backup_download_url(
        &self,
        backup_id: String,
        ttl: std::time::Duration,
    ) -> Result<String, anyhow::Error> {
        let state = self.state().await;

        let backup_id = BackupFileName::try_from(&backup_id)?;

        let backup = state
            .backup_service
            .get_download_url(&backup_id, ttl)
            .await?;

        Ok(backup)
    }
}

// MARK: - Implementation details

// MARK: API

pub struct Api {
    constants: ApiConstans,
    state: RwLock<ApiState>,
}

impl Api {
    async fn state(&self) -> RwLockReadGuard<'_, ApiState> {
        self.state.read().await
    }
}

/// This would be hard-coded as constants in the Prose Pod API code.
pub struct ApiConstans {
    backups_version: u8,
    backup_blueprints: HashMap<u8, ArchiveBlueprint>,
}

impl ApiConstans {
    fn v1() -> Self {
        Self {
            backups_version: 1,
            backup_blueprints: todo!(),
        }
    }
}

// MARK: API config

#[derive(Debug, serde::Deserialize)]
struct ApiConfig {
    backups: BackupConfig,
}

fn load_config() -> Result<ApiConfig, anyhow::Error> {
    todo!()
}

// MARK: API state

pub struct ApiState {
    backup_service: BackupService,
}

impl ApiState {
    fn new_v1() -> Result<Self, anyhow::Error> {
        let api_config = load_config()?;

        let prose_pod_api_dir = env_required!("PROSE_POD_API_DIR");

        todo!()
    }
}
