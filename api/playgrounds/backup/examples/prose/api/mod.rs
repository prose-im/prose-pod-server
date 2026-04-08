// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! An abstraction of the Prose Pod API.

#![allow(dead_code, unused_imports, unused_macros)]

pub mod v2;
pub mod v3;

use std::collections::HashMap;

use prose_backup::{
    CreateBackupSuccess,
    dtos::{BackupDto, BackupMetadataFullDto, BackupMetadataPartialDto},
};
use tokio::sync::{RwLockReadGuard, mpsc};

pub use self::v2::start_v2;
pub use self::v3::start_v3;

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

// Map env to simulate real configuration.
macro_rules! map_env {
    ($from:literal -> $to:literal) => {
        let val = env_required!($from);
        std::env::set_var($to, val);
    };
}
pub(crate) use map_env;

fn load_config<'de, T: serde::Deserialize<'de>>(
    path: impl AsRef<std::path::Path>,
) -> Result<T, anyhow::Error> {
    use figment::Figment;

    fn default_config_static() -> toml::Table {
        use toml::toml;

        let backups_default = prose_backup::config::default_config_static();

        let defaults = toml! {
            backups = backups_default
        };

        defaults
    }

    fn default_figment() -> Figment {
        use figment::providers::Serialized;

        Figment::from(Serialized::defaults(default_config_static()))
    }

    fn with_dynamic_defaults(mut figment: Figment) -> Result<Figment, anyhow::Error> {
        use figment::providers::*;

        let backups = prose_backup::config::with_dynamic_defaults(figment.focus("backups"))?;
        let backups_value = backups.extract::<figment::value::Value>()?;

        figment = figment
            // NOTE: `Figment::merge` merges objects which does not remove
            //   existing keys. Merging `()` first does the trick.
            .merge(Serialized::default("backups", figment::value::Empty::Unit))
            .merge(Serialized::default("backups", backups_value));

        Ok(figment)
    }

    pub fn figment_at_path(path: impl AsRef<std::path::Path>) -> Figment {
        use figment::providers::*;

        default_figment()
            .merge(Toml::file(path))
            .merge(Env::prefixed("PROSE_").split("__"))
    }

    fn try_from<'de, T: serde::Deserialize<'de>>(
        figment: figment::Figment,
    ) -> Result<T, anyhow::Error> {
        with_dynamic_defaults(figment)?
            .extract::<T>()
            .map_err(anyhow::Error::from)
    }

    try_from(figment_at_path(path))
}
