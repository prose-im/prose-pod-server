// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! The version 2 of the Prose Pod API, where the Prose Pod API has state
//! and it calls the Prose Pod Server API for some operations.

use std::{path::Path, str::FromStr as _};

use anyhow::Context;
use prose_backup::{
    BackupConfig, BackupId, BackupService, CreateBackupCommand, ExtractionSuccess,
    archiving::ArchiveBlueprint, event_handlers::NoopEventHandler,
};
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
        let fs_root = env_required!(EXAMPLE_TMPDIR_VAR_NAME);
        let api_data_path = Path::new(&fs_root).join("var/lib/prose-pod-api");
        builder
            .append_dir_all(&self.constants.backup_data_key_self, &api_data_path)
            .context(format!("Dir: {api_data_path:?}"))?;

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
        self.server_api.put_backup_restore(backup_id, self).await
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
    let tmpdir_path = env_required!(EXAMPLE_TMPDIR_VAR_NAME);

    init_tsks(&tmpdir_path).context("Failed creating tsks")?;
    init_prose_config(&tmpdir_path).context("Failed creating prose.toml")?;

    let server_api = {
        let constants = ProsePodServerApiConstants::v2();
        let state = ProsePodServerState::new_v2(&constants)?;

        ProsePodServerApiV2 {
            constants,
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

        let mut command = CreateBackupCommand::new(
            concat!("example-", env!("CARGO_CRATE_NAME")),
            &description,
            backups_version,
            blueprint,
        );
        command.additional_archive_data = vec![(
            PROSE_POD_API_ARCHIVE_KEY.to_owned(),
            prose_pod_api_data.len() as u64,
            Box::new(std::io::Cursor::new(prose_pod_api_data)),
        )];

        let todo = "Stream backup progress";
        let response = state
            .backup_service
            .create_backup(command, &mut NoopEventHandler)
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

        let backup_id = BackupId::from_str(&backup_id)?;

        let backup = state.backup_service.get_details(&backup_id).await?;

        Ok(backup)
    }

    /// `DELETE /backups/{backup_id}`.
    async fn delete_backup(&self, backup_id: String) -> Result<(), anyhow::Error> {
        let state = self.state().await;

        let backup_id = BackupId::from_str(&backup_id)?;

        state.backup_service.delete_backup(&backup_id).await?;

        Ok(())
    }

    /// `PUT /backups/{backup_id}/restore`.
    async fn put_backup_restore(
        &self,
        backup_id: String,
        prose_pod_api: &ProsePodApiV2,
    ) -> Result<(), anyhow::Error> {
        let state = self.state().await;

        let backup_id = BackupId::from_str(&backup_id)?;

        let blueprint = self
            .constants
            .backup_blueprints
            .get(&self.constants.backups_version)
            .unwrap();

        let ExtractionSuccess {
            extraction_output, ..
        } = state.backup_service.extract_backup(&backup_id).await?;

        let prose_pod_api_data_path = extraction_output
            .tmp_dir
            .path()
            .join(PROSE_POD_API_ARCHIVE_KEY);
        let prose_pod_api_data_file = std::fs::File::open(&prose_pod_api_data_path)?;

        () = prose_pod_api.put_restore(prose_pod_api_data_file).await?;

        std::fs::remove_file(prose_pod_api_data_path)?;

        let _response = state
            .backup_service
            .restore_extracted_backup(extraction_output, blueprint)
            .await?;

        Ok(())
    }

    /// `GET /backups/{backup_id}/download-url`.
    async fn get_backup_download_url(
        &self,
        backup_id: String,
        ttl: std::time::Duration,
    ) -> Result<String, anyhow::Error> {
        let state = self.state().await;

        let backup_id = BackupId::from_str(&backup_id)?;

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

impl ProsePodApiV2 {
    async fn put_restore(&self, data: std::fs::File) -> Result<(), anyhow::Error> {
        let mut archive = tar::Archive::new(data);

        let tmpdir = tempfile::TempDir::with_prefix(env!("CARGO_CRATE_NAME"))?;

        archive.unpack(tmpdir.path())?;

        let fs_root = env_required!(EXAMPLE_TMPDIR_VAR_NAME);
        let api_data_path = Path::new(&fs_root).join("var/lib/prose-pod-api");

        // NOTE: In the real codebase we’d do this atomically.
        std::fs::remove_dir_all(&api_data_path)?;

        std::fs::rename(
            tmpdir.path().join(&self.constants.backup_data_key_self),
            &api_data_path,
        )?;

        Ok(())
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
        let fs_root = env_required!(EXAMPLE_TMPDIR_VAR_NAME);

        let mut blueprints: HashMap<u8, ArchiveBlueprint> = HashMap::with_capacity(1);
        blueprints.insert(1, blueprint_v2(fs_root));

        Self {
            backups_version: 1,
            backup_blueprints: blueprints,
        }
    }
}

fn blueprint_v2(root: impl AsRef<Path>) -> ArchiveBlueprint {
    let root = root.as_ref();
    ArchiveBlueprint::from_iter(
        [
            ("prose-pod-server-data", "var/lib/prose-pod-server"),
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

fn load_config(path: impl AsRef<Path>) -> Result<ProsePodServerConfig, anyhow::Error> {
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

    pub fn figment_at_path(path: impl AsRef<Path>) -> Figment {
        use figment::providers::*;

        default_figment()
            .merge(Toml::file(path))
            .merge(Env::prefixed("PROSE_").split("__"))
    }

    fn try_from(figment: figment::Figment) -> Result<ProsePodServerConfig, anyhow::Error> {
        with_dynamic_defaults(figment)?
            .extract::<ProsePodServerConfig>()
            .map_err(anyhow::Error::from)
    }

    // Map env to simulate real configuration.
    macro_rules! map_env {
        ($from:literal -> $to:literal) => {
            let val = env_required!($from);
            std::env::set_var($to, val);
        };
    }
    unsafe {
        map_env!("S3_BUCKET_NAME_BACKUPS" -> "PROSE_BACKUPS__STORAGE__BACKUPS__S3__BUCKET_NAME");
        map_env!("S3_BUCKET_NAME_CHECKS" -> "PROSE_BACKUPS__STORAGE__CHECKS__S3__BUCKET_NAME");
        map_env!("S3_REGION" -> "PROSE_BACKUPS__S3__REGION");
        map_env!("S3_ENDPOINT_URL" -> "PROSE_BACKUPS__S3__ENDPOINT_URL");
        map_env!("S3_ACCESS_KEY" -> "PROSE_BACKUPS__S3__ACCESS_KEY");
        map_env!("S3_SECRET_KEY" -> "PROSE_BACKUPS__S3__SECRET_KEY");
    };

    try_from(figment_at_path(path))
}

// MARK: API state

pub struct ProsePodServerState {
    // NOTE: Just to highlight the fact that `BackupService::from_config`
    //   doesn’t consume its configuration.
    #[allow(dead_code)]
    config: ProsePodServerConfig,
    backup_service: BackupService,
}

impl ProsePodServerState {
    fn new_v2(constants: &ProsePodServerApiConstants) -> Result<Self, anyhow::Error> {
        let fs_root = env_required!(EXAMPLE_TMPDIR_VAR_NAME);
        let config_path = Path::new(&fs_root).join("etc/prose/prose.toml");

        let config = load_config(&config_path)?;
        // tracing::debug!("Parsed config: {api_config:#?}");

        let backup_service =
            BackupService::from_config(&config.backups, constants.backup_blueprints.clone())?;

        Ok(Self {
            config,
            backup_service,
        })
    }
}
