// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! The version 2 of the Prose Pod API, where the Prose Pod API has state
//! and it calls the Prose Pod Server API for some operations.

use std::path::Path;

use prose_backup::{BackupConfig, BackupFileName, BackupService, archiving::ArchiveBlueprint};
use tokio::sync::RwLock;

use crate::common::{lifecycle::EXAMPLE_TMPDIR_VAR_NAME, util::*};

use super::*;

// MARK: - Public API

#[async_trait::async_trait]
impl ProsePodApi for ProsePodApiV2 {
    /// `POST /backups`.
    async fn post_backups(
        &self,
        description: String,
    ) -> Result<CreateBackupSuccess, anyhow::Error> {
        let buf: Vec<u8> = Vec::new();
        let mut builder = tar::Builder::new(buf);

        // NOTE: Example data, the Prose Pod API saves other files.
        builder.append_dir_all(
            &self.constants.backup_data_key_self,
            Path::new(EXAMPLE_TMPDIR_VAR_NAME).join("var/lib/prose-pod-api"),
        )?;

        let prose_pod_api_data = builder.into_inner()?;

        self.server_api
            .post_backups(description, prose_pod_api_data.into())
            .await
    }

    /// `GET /backups`.
    async fn get_backups(&self) -> Result<Vec<BackupDto<BackupMetadataPartialDto>>, anyhow::Error> {
        self.server_api.get_backups().await
    }

    /// `GET /backups/{backup_id}`.
    async fn get_backup(
        &self,
        backup_id: String,
    ) -> Result<BackupDto<BackupMetadataFullDto>, anyhow::Error> {
        self.server_api.get_backup(backup_id).await
    }

    /// `DELETE /backups/{backup_id}`.
    async fn delete_backup(&self, backup_id: String) -> Result<(), anyhow::Error> {
        self.server_api.delete_backup(backup_id).await
    }

    /// `PUT /backups/{backup_id}/restore`.
    async fn put_backup_restore(&self, backup_id: String) -> Result<(), anyhow::Error> {
        self.server_api.put_backup_restore(backup_id).await
    }

    /// `GET /backups/{backup_id}/download-url`.
    async fn get_backup_download_url(
        &self,
        backup_id: String,
        ttl: std::time::Duration,
    ) -> Result<String, anyhow::Error> {
        self.server_api
            .get_backup_download_url(backup_id, ttl)
            .await
    }
}

pub fn start_v2() -> Result<ProsePodApiV2, anyhow::Error> {
    let server_api = {
        let state = ProsePodServerState::new_v2()?;

        ProsePodServerApiV2 {
            constants: ProsePodServerApiConstants::v2(),
            state: RwLock::new(state),
        }
    };

    Ok(ProsePodApiV2 {
        constants: ProsePodApiConstants::v2(),
        server_api,
    })
}

// MARK: - Internals

const PROSE_POD_API_ARCHIVE_KEY: &str = "prose-pod-api-data";

impl ProsePodServerApiV2 {
    /// `POST /backups`.
    async fn post_backups(
        &self,
        description: String,
        prose_pod_api_data: bytes::Bytes,
    ) -> Result<CreateBackupSuccess, anyhow::Error> {
        let state = self.state().await;

        let backups_version = self.constants.backups_version;
        let blueprint = &self.constants.backup_blueprints[&backups_version];

        let mut command = prose_backup::CreateBackupCommand::new(
            concat!("example-", env!("CARGO_CRATE_NAME")),
            &description,
            backups_version,
            blueprint,
        );
        command.additional_archive_data =
            vec![(PROSE_POD_API_ARCHIVE_KEY.to_owned(), prose_pod_api_data)];

        let response = state.backup_service.create_backup(command).await?;

        Ok(response)
    }

    /// `GET /backups`.
    async fn get_backups(&self) -> Result<Vec<BackupDto<BackupMetadataPartialDto>>, anyhow::Error> {
        let state = self.state().await;

        let backups = state.backup_service.list_backups().await?;

        Ok(backups)
    }

    /// `GET /backups/{backup_id}`.
    async fn get_backup(
        &self,
        backup_id: String,
    ) -> Result<BackupDto<BackupMetadataFullDto>, anyhow::Error> {
        let state = self.state().await;

        let backup_id = BackupFileName::try_from(&backup_id)?;

        let backup = state.backup_service.get_details(&backup_id).await?;

        Ok(backup)
    }

    /// `DELETE /backups/{backup_id}`.
    async fn delete_backup(&self, backup_id: String) -> Result<(), anyhow::Error> {
        let state = self.state().await;

        let backup_id = BackupFileName::try_from(&backup_id)?;

        state.backup_service.delete_backup(&backup_id).await?;

        Ok(())
    }

    /// `PUT /backups/{backup_id}/restore`.
    async fn put_backup_restore(&self, backup_id: String) -> Result<(), anyhow::Error> {
        let state = self.state().await;

        todo!()
    }

    /// `GET /backups/{backup_id}/download-url`.
    async fn get_backup_download_url(
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

// MARK: Prose Pod API

/// Prose Pod API, as of early 2026. For more information, see
/// [“Prose Pod Server architecture: Server API vs XMPP server”](https://github.com/prose-im/prose-pod-server/blob/b881891e442d35ad6bfdf65ec164cc6911855ba3/api/docs/ARCHITECTURE.md).
pub struct ProsePodApiV2 {
    constants: ProsePodApiConstants,
    server_api: ProsePodServerApiV2,
}

/// This would be hard-coded as constants in the Prose Pod API code.
pub struct ProsePodApiConstants {
    backup_data_key_self: String,
}

impl ProsePodApiConstants {
    fn v2() -> Self {
        Self {
            backup_data_key_self: "self-data".to_owned(),
        }
    }
}

// MARK: Prose Pod Server

struct ProsePodServerApiV2 {
    constants: ProsePodServerApiConstants,
    state: RwLock<ProsePodServerState>,
}

impl ProsePodServerApiV2 {
    async fn state(&self) -> RwLockReadGuard<'_, ProsePodServerState> {
        self.state.read().await
    }
}

/// This would be hard-coded as constants in the Prose Pod Server API code.
pub struct ProsePodServerApiConstants {
    backups_version: u8,
    backup_blueprints: HashMap<u8, ArchiveBlueprint>,
}

impl ProsePodServerApiConstants {
    fn v2() -> Self {
        let prose_pod_api_dir = env_required!("PROSE_POD_API_DIR");
        let src_root = Path::new(&prose_pod_api_dir).join("local-run/scenarios/demo");
        let a = env_required!(EXAMPLE_TMPDIR_VAR_NAME);

        Self {
            backups_version: 1,
            backup_blueprints: [(1, blueprint_v2(&src_root))].into_iter().collect(),
        }
    }
}

fn blueprint_v2(root: impl AsRef<Path>) -> ArchiveBlueprint {
    let root = root.as_ref();
    ArchiveBlueprint::from_iter(
        [
            ("prose-pod-server-data", "var/lib/prose-pod-server"),
            ("prose-pod-api-data", "var/lib/prose-pod-api"),
            ("prosody-data", "var/lib/prosody"),
            ("prose-config", "etc/prose"),
            ("prosody-config", "etc/prosody"),
        ]
        .into_iter()
        .map(|(dst, src)| (dst, root.join(src))),
    )
}

// MARK: API config

#[derive(Debug, serde::Deserialize)]
struct ProsePodServerConfig {
    backups: BackupConfig,
}

fn load_config() -> Result<ProsePodServerConfig, anyhow::Error> {
    todo!()
}

// MARK: API state

pub struct ProsePodServerState {
    backup_service: BackupService,
}

impl ProsePodServerState {
    fn new_v2() -> Result<Self, anyhow::Error> {
        let api_config = load_config()?;

        let prose_pod_api_dir = env_required!("PROSE_POD_API_DIR");

        todo!()
    }
}
