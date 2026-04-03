// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! An abstraction of the Prose Pod Dashboard.

use std::{sync::Arc, time::Duration};

use anyhow::anyhow;
use prose_backup::dtos::{BackupDto, BackupMetadataFullDto, BackupMetadataPartialDto};
use time::UtcDateTime;
use tokio::sync::{RwLock, RwLockReadGuard};

use crate::prose::api::ProsePodApi;

use super::api::CreateBackupEvent;

pub struct Dashboard {
    api: Arc<RwLock<Option<Box<dyn ProsePodApi>>>>,
}

impl Dashboard {
    pub fn new(api: Arc<RwLock<Option<Box<dyn ProsePodApi>>>>) -> Self {
        Self { api }
    }

    async fn api(&self) -> Result<RwLockReadGuard<'_, Box<dyn ProsePodApi>>, anyhow::Error> {
        let guard = self.api.read().await;
        RwLockReadGuard::try_map(guard, |opt| opt.as_ref())
            .map_err(|_| anyhow!("API is restarting"))
    }
}

#[allow(dead_code)]
pub struct BackupEntryModel {
    pub backup_id: String,
    pub description: String,
    pub is_signed: bool,
    pub is_encrypted: bool,
    pub created_at: UtcDateTime,
    pub size_bytes: u64,
}

#[allow(dead_code)]
pub struct BackupDetailsModel {
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
    pub async fn show_backups(&self) -> Result<Vec<BackupEntryModel>, anyhow::Error> {
        let backups = {
            tracing::trace!("Listing backups…");
            let api = self.api().await?;
            api.get_backups().await?
        };

        let mut list: Vec<BackupEntryModel> = Vec::with_capacity(backups.len());

        for dto in backups {
            list.push(BackupEntryModel::from(dto));
        }

        println!("Backups list:\n{}", BackupEntryModel::table_header());
        for backup in list.iter() {
            println!("{}", backup.table_row());
        }
        println!("{}", BackupEntryModel::table_footer());

        Ok(list)
    }

    #[allow(dead_code)]
    pub async fn create_backup(
        &self,
        description: impl Into<String>,
    ) -> Result<BackupEntryModel, anyhow::Error> {
        let response = {
            tracing::trace!("Creating a backup…");
            let api = self.api().await?;
            api.post_backups(description.into()).await?
        };

        Ok(BackupEntryModel::from(response.backup))
    }

    pub async fn create_backup_stream(
        &self,
        description: impl Into<String>,
        on_progress: impl Fn(u64, u64),
    ) -> Result<BackupEntryModel, anyhow::Error> {
        let response = {
            tracing::trace!("Creating a backup…");
            let api = self.api().await?;
            let mut events = api.post_backups_stream(description.into()).await?;

            'ret: {
                // NOTE: In a real app we’d debounce events here.
                while let Some(event) = events.recv().await {
                    match event {
                        CreateBackupEvent::Progress { progress, total } => {
                            on_progress(progress, total)
                        }
                        CreateBackupEvent::End(create_backup_success) => {
                            break 'ret create_backup_success?;
                        }
                    }
                }
                unreachable!()
            }
        };

        Ok(BackupEntryModel::from(response.backup))
    }

    pub async fn inspect_backup(
        &self,
        backup_id: String,
    ) -> Result<BackupDetailsModel, anyhow::Error> {
        let backup = {
            tracing::trace!("Inspecting backup…");
            let api = self.api().await?;
            api.get_backup(backup_id).await?
        };

        Ok(BackupDetailsModel::from(backup))
    }

    pub async fn download_backup(&self, backup_id: String) -> Result<String, anyhow::Error> {
        let download_url = {
            tracing::trace!("Getting backup download URL…");
            let api = self.api().await?;
            api.get_backup_download_url(backup_id, Duration::from_mins(5))
                .await?
        };

        // TODO: Download and save to a file instead of returning the URL.

        Ok(download_url)
    }

    pub async fn restore_backup(&self, backup_id: String) -> Result<(), anyhow::Error> {
        let result: () = {
            tracing::trace!("Restoring backup…");
            let api = self.api().await?;
            api.put_backup_restore(backup_id).await?
        };

        Ok(result)
    }

    pub async fn delete_backup(&self, backup_id: String) -> Result<(), anyhow::Error> {
        let result: () = {
            tracing::trace!("Deleting backup…");
            let api = self.api().await?;
            api.delete_backup(backup_id).await?
        };

        Ok(result)
    }
}

// MARK: - Display

impl BackupEntryModel {
    pub(crate) fn table_header() -> String {
        let col1_width = 32;
        let col2_width = 6;
        let col3_width = 30;
        let col4_width = 7;

        [
            format!(
                "┌─{:─>col1_width$}─┬─{:─>col2_width$}─┬─{:─>col3_width$}─┬─{:─>col4_width$}─┐",
                "", "", "", "",
            ),
            format!(
                "│ {:<col1_width$} │ {:<col2_width$} │ {:<col3_width$} │ {:>col4_width$} │",
                "Description", "Status", "Created", "Size",
            ),
            format!(
                "├─{:─>col1_width$}─┼─{:─>col2_width$}─┼─{:─>col3_width$}─┼─{:─>col4_width$}─┤",
                "", "", "", "",
            ),
        ]
        .join("\n")
    }

    pub(crate) fn table_footer() -> String {
        let col1_width = 32;
        let col2_width = 6;
        let col3_width = 30;
        let col4_width = 7;

        format!(
            "└─{:─>col1_width$}─┴─{:─>col2_width$}─┴─{:─>col3_width$}─┴─{:─>col4_width$}─┘",
            "", "", "", "",
        )
    }

    pub(crate) fn table_row(&self) -> String {
        let Self {
            backup_id: _,
            description,
            is_signed,
            is_encrypted,
            created_at,
            size_bytes,
        } = self;

        format!(
            "│ {description:<32} │ {signed}{encrypted}     │ {created_at:<30} │ {size_bytes:>6}B │",
            signed = if *is_signed { "S" } else { "" },
            encrypted = if *is_encrypted { "E" } else { "" }
        )
    }

    pub(crate) fn display(&self) -> String {
        let Self {
            backup_id,
            description,
            is_signed,
            is_encrypted,
            created_at,
            size_bytes,
        } = self;

        let is_signed = is_signed.to_string();
        let is_encrypted = is_encrypted.to_string();
        let created_at = created_at.to_string();
        let size_bytes = size_bytes.to_string();

        #[rustfmt::skip]
        let parts = [
            backup_id.as_str(),
            "\n  Description: ", description.as_str(),
            "\n  Signed? ", is_signed.as_str(),
            "\n  Encrypted? ", is_encrypted.as_str(),
            "\n  Created at: ", created_at.as_str(),
            "\n  Size: ", size_bytes.as_str(), "B",
        ];

        let mut str = String::new();
        for part in parts {
            str.push_str(part);
        }
        str
    }
}

impl BackupDetailsModel {
    pub(crate) fn display(&self) -> String {
        let Self {
            backup_id,
            description,
            is_signed,
            is_encrypted,
            created_at,
            size_bytes,
        } = self;

        let is_signed = is_signed.to_string();
        let is_encrypted = is_encrypted.to_string();
        let created_at = created_at.to_string();
        let size_bytes = size_bytes.to_string();

        #[rustfmt::skip]
        let parts = [
            backup_id.as_str(),
            "\n  Description: ", description.as_str(),
            "\n  Signed? ", is_signed.as_str(),
            "\n  Encrypted? ", is_encrypted.as_str(),
            "\n  Created at: ", created_at.as_str(),
            "\n  Size: ", size_bytes.as_str(), "B",
        ];

        let mut str = String::new();
        for part in parts {
            str.push_str(part);
        }
        str
    }
}

// MARK: - Boilerplate

impl From<BackupDto<BackupMetadataPartialDto>> for BackupEntryModel {
    fn from(dto: BackupDto<BackupMetadataPartialDto>) -> Self {
        Self {
            backup_id: dto.id.to_string(),
            description: dto.description,
            is_signed: dto.metadata.is_signed,
            is_encrypted: dto.metadata.is_encrypted,
            created_at: dto.metadata.created_at,
            size_bytes: dto.metadata.size_bytes,
        }
    }
}

impl From<BackupDto<BackupMetadataFullDto>> for BackupDetailsModel {
    fn from(dto: BackupDto<BackupMetadataFullDto>) -> Self {
        let fixme = "Add more fields";
        Self {
            backup_id: dto.id.to_string(),
            description: dto.description,
            is_signed: dto.metadata.is_signed,
            is_encrypted: dto.metadata.is_encrypted,
            created_at: dto.metadata.created_at,
            size_bytes: dto.metadata.size_bytes,
        }
    }
}
