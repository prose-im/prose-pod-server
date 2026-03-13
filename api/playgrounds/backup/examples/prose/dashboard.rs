// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! An abstraction of the Prose Pod Dashboard.

use std::{sync::Arc, time::Duration};

use anyhow::anyhow;
use time::UtcDateTime;
use tokio::sync::{RwLock, RwLockReadGuard};

use crate::prose::api::Api;

pub struct Dashboard {
    api: Arc<RwLock<Option<Api>>>,
}

impl Dashboard {
    pub fn new(api: Arc<RwLock<Option<Api>>>) -> Self {
        Self { api }
    }

    async fn api(&self) -> Result<RwLockReadGuard<'_, Api>, anyhow::Error> {
        let guard = self.api.read().await;
        RwLockReadGuard::try_map(guard, |opt| opt.as_ref())
            .map_err(|_| anyhow!("API is restarting"))
    }
}

#[allow(dead_code)]
pub struct BackupUiModel {
    pub backup_id: String,
    pub description: String,
    pub is_signed: bool,
    pub is_encrypted: bool,
    pub created_at: UtcDateTime,
    pub size_bytes: u64,
}

// NOTE: Features return `Result`s. GUIs should save app state and display
//   errors as alert-looking elements, not be binary like `Result`. However,
//   for the purpose of this example, we won’t go into such detail.
impl Dashboard {
    pub async fn show_backups(&self) -> Result<Vec<BackupUiModel>, anyhow::Error> {
        let backups = {
            let api = self.api().await?;
            api.get_backups().await?
        };

        let mut list: Vec<BackupUiModel> = Vec::with_capacity(backups.len());

        for dto in backups {
            list.push(BackupUiModel {
                backup_id: dto.id.to_string(),
                description: dto.description,
                is_signed: dto.metadata.is_signed,
                is_encrypted: dto.metadata.is_encrypted,
                created_at: dto.metadata.created_at,
                size_bytes: dto.metadata.size_bytes,
            });
        }

        Ok(list)
    }

    pub async fn create_backup(
        &self,
        description: impl Into<String>,
    ) -> Result<BackupUiModel, anyhow::Error> {
        let backup = {
            let api = self.api().await?;
            api.post_backups(description.into()).await?
        };

        todo!()
    }

    pub async fn inspect_backup(&self, backup_id: String) -> Result<BackupUiModel, anyhow::Error> {
        let backup = {
            let api = self.api().await?;
            api.get_backup(backup_id).await?
        };

        todo!()
    }

    pub async fn download_backup(&self, backup_id: String) -> Result<String, anyhow::Error> {
        let download_url = {
            let api = self.api().await?;
            api.get_backup_download_url(backup_id, Duration::from_mins(5))
                .await?
        };

        // TODO: Download and save to a file instead of returning the URL.

        Ok(download_url)
    }
}
