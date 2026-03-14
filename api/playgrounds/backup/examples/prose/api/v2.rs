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
impl ProsePodApi for ApiV2 {
    /// `POST /backups`.
    async fn post_backups(
        &self,
        description: String,
    ) -> Result<CreateBackupSuccess, anyhow::Error> {
        let state = self.state().await;

        let backups_version = self.constants.backups_version;
        let ref blueprint = self.constants.backup_blueprints[&backups_version];

        let response = state
            .backup_service
            .create_backup(prose_backup::CreateBackupCommand {
                prefix: concat!("example-", env!("CARGO_CRATE_NAME")),
                description: &description,
                version: backups_version,
                blueprint,
                // Just to make rust-analyzer happy…
                #[cfg(feature = "test")]
                created_at: std::time::SystemTime::now(),
            })
            .await?;

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

pub fn start_v2() -> Result<ApiV2, anyhow::Error> {
    let state = ApiState::new_v2()?;

    Ok(ApiV2 {
        constants: ApiConstants::v2(),
        state: RwLock::new(state),
    })
}

// MARK: - Implementation details

// MARK: API

pub struct ApiV2 {
    constants: ApiConstants,
    state: RwLock<ApiState>,
}

impl ApiV2 {
    async fn state(&self) -> RwLockReadGuard<'_, ApiState> {
        self.state.read().await
    }
}

/// This would be hard-coded as constants in the Prose Pod API code.
pub struct ApiConstants {
    backups_version: u8,
    backup_blueprints: HashMap<u8, ArchiveBlueprint>,
}

impl ApiConstants {
    fn v2() -> Self {
        let prose_pod_api_dir = env_required!("PROSE_POD_API_DIR");
        let src_root = Path::new(&prose_pod_api_dir).join("local-run/scenarios/demo");
        let a = env_required!(EXAMPLE_TMPDIR_VAR_NAME);

        Self {
            backups_version: 1,
            backup_blueprints: [(1, Self::blueprint_v2(&src_root))].into_iter().collect(),
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
    fn new_v2() -> Result<Self, anyhow::Error> {
        let api_config = load_config()?;

        let prose_pod_api_dir = env_required!("PROSE_POD_API_DIR");

        todo!()
    }
}

// MARK: Setup

/// Creates a temporary filesystem root where backup operations will be
/// performed (avoids TODO).
pub fn init_fake_fs_root(
    example_context: &crate::common::lifecycle::ExampleContext,
) -> Result<(), anyhow::Error> {
    use crate::common::util::env_required;
    use std::process::Command;

    let tmpdir = example_context.tmpdir.path();

    let pod_api_commit_hash = todo!();
    let temp_api_path = tempfile::TempPath::from_path(tmpdir.join("prose-pod-api"));

    // Checkout the Prose Pod API at the desired version.
    {
        let prose_pod_api_dir = env_required!("PROSE_POD_API_DIR");

        _ = Command::new("git")
            .args([
                "-C",
                prose_pod_api_dir.as_str(),
                "worktree",
                "add",
                // SAFETY: `EXAMPLE_TMPDIR` has already been parsed to valid UTF-8.
                temp_api_path.to_str().unwrap(),
                pod_api_commit_hash,
            ])
            .status()?;
    }

    {}
    let fake_fsroot_path = tmpdir.join("fs-root");

    // std::fs::rename(temp_api_path.join("local-run/scenarios/demo"), to)

    todo!()
}
