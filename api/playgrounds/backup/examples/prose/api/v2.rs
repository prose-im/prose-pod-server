// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! The version 2 of the Prose Pod API, where the Prose Pod API has state
//! and it calls the Prose Pod Server API for some operations.

use std::{path::Path, str::FromStr as _, sync::Arc};

use anyhow::Context;
use bytes::Buf;
use prose_backup::archiving::{AdditionalData, ArchiveBlueprint};
use prose_backup::event_handlers::NoopEventHandler;
use prose_backup::{
    BackupConfig, BackupId, BackupService, CreateBackupCommand, CreateBackupEventHandler,
    ExtractBackupEventHandler, ExtractionSuccess, RestoreBackupEventHandler,
};
use tokio::sync::RwLock;

use crate::common::{lifecycle::EXAMPLE_TMPDIR_VAR_NAME, util::*};

use super::*;

// MARK: - Public API

#[async_trait::async_trait]
impl ProsePodApi for ProsePodApiV2 {
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

    async fn post_backups_stream(
        &self,
        description: String,
    ) -> Result<mpsc::Receiver<CreateBackupEvent>, anyhow::Error> {
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
            .post_backups_stream(description, prose_pod_api_data.into())
            .await
    }

    async fn get_backups(&self) -> Result<Vec<BackupDto<BackupMetadataPartialDto>>, anyhow::Error> {
        self.server_api.get_backups().await
    }

    async fn get_backup(
        &self,
        backup_id: String,
    ) -> Result<BackupDto<BackupMetadataFullDto>, anyhow::Error> {
        self.server_api.get_backup(backup_id).await
    }

    async fn delete_backup(&self, backup_id: String) -> Result<(), anyhow::Error> {
        self.server_api.delete_backup(backup_id).await
    }

    async fn put_backup_restore(&self, backup_id: String) -> Result<(), anyhow::Error> {
        self.server_api.put_backup_restore(backup_id, self).await
    }

    async fn put_backup_restore_stream(
        &self,
        backup_id: String,
    ) -> Result<mpsc::Receiver<RestoreBackupEvent>, anyhow::Error> {
        self.server_api
            .put_backup_restore_stream(backup_id, self)
            .await
    }

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
        let constants = ProsePodServerApiConstants::v2();
        let state = ProsePodServerState::new_v2(&constants)?;

        ProsePodServerApiV2 {
            constants,
            state: RwLock::new(state),
        }
    };

    Ok(ProsePodApiV2 {
        constants: Arc::new(ProsePodApiConstants::v2()),
        server_api: Arc::new(server_api),
    })
}

// MARK: - Internals

pub(super) const PROSE_POD_API_ARCHIVE_KEY: &str = "prose-pod-api-data";

#[repr(transparent)]
struct ProsePodApiData(bytes::Bytes);

impl AdditionalData for ProsePodApiData {
    fn append<W: std::io::Write>(self, builder: &mut tar::Builder<W>) -> Result<(), anyhow::Error> {
        let mut archive = tar::Archive::new(self.0.reader());
        let entries = archive.entries()?;

        for entry in entries {
            let entry = entry?;

            builder.append(&entry.header().clone(), entry)?;
        }

        Ok(())
    }
}

impl ProsePodServerApiV2 {
    async fn post_backups_(
        backup_service: &BackupService,
        description: String,
        prose_pod_api_data: bytes::Bytes,
        backups_version: u8,
        blueprint: &ArchiveBlueprint,
        event_handler: &mut impl CreateBackupEventHandler,
    ) -> Result<CreateBackupSuccess, anyhow::Error> {
        let command = CreateBackupCommand {
            prefix: concat!("example-", env!("CARGO_CRATE_NAME")),
            description: &description,
            version: backups_version,
            blueprint,
            additional_archive_data: Some(ProsePodApiData(prose_pod_api_data)),
            #[cfg(feature = "test")]
            created_at: std::time::SystemTime::now(),
        };

        let response = backup_service.create_backup(command, event_handler).await?;

        Ok(response)
    }

    /// `POST /backups`.
    async fn post_backups(
        &self,
        description: String,
        prose_pod_api_data: bytes::Bytes,
    ) -> Result<CreateBackupSuccess, anyhow::Error> {
        let state = self.state().await;

        let backups_version = self.constants.backups_version;
        let blueprint = &self.constants.backup_blueprints[&backups_version];

        Self::post_backups_(
            &state.backup_service,
            description,
            prose_pod_api_data,
            backups_version,
            blueprint,
            &mut NoopEventHandler,
        )
        .await
    }

    /// `POST /backups Accept:text/event-stream`.
    async fn post_backups_stream(
        &self,
        description: String,
        prose_pod_api_data: bytes::Bytes,
    ) -> Result<mpsc::Receiver<CreateBackupEvent>, anyhow::Error> {
        // Stream backup progress.
        let (mut event_handler, sender, receiver) = {
            let (sender, receiver) = mpsc::channel(8);

            struct EventHandler {
                expected_archive_size: u64,
                effective_archive_size: u64,
                progress_sender: Arc<mpsc::Sender<CreateBackupEvent>>,
            }

            impl CreateBackupEventHandler for EventHandler {
                fn on_archive_start(&mut self, _backup_id: &BackupId, expected_archive_size: u64) {
                    assert_eq!(self.effective_archive_size, 0);

                    self.expected_archive_size = expected_archive_size;

                    tokio::task::block_in_place(move || {
                        tokio::runtime::Handle::current().block_on(async move {
                            self.progress_sender
                                .send(CreateBackupEvent::Progress {
                                    progress: 0,
                                    total: expected_archive_size,
                                })
                                .await
                                .unwrap_or_else(|err| {
                                    debug_panic_or_log_error!("Progress init error: {err:#}")
                                });
                        })
                    })
                }

                fn on_archive_progress(&mut self, _backup_id: &BackupId, archived_bytes: usize) {
                    assert_ne!(self.expected_archive_size, 0);

                    self.effective_archive_size += archived_bytes as u64;

                    tokio::task::block_in_place(move || {
                        tokio::runtime::Handle::current().block_on(async move {
                            self.progress_sender
                                .send(CreateBackupEvent::Progress {
                                    progress: self.effective_archive_size,
                                    total: self.expected_archive_size,
                                })
                                .await
                                .unwrap_or_else(|err| {
                                    debug_panic_or_log_error!("Progress send error: {err:#}")
                                });
                        })
                    })
                }
            }

            let sender = Arc::new(sender);

            (
                EventHandler {
                    expected_archive_size: 0,
                    effective_archive_size: 0,
                    progress_sender: Arc::clone(&sender),
                },
                sender,
                receiver,
            )
        };

        // NOTE: In a real API there would likely be a system for awaiting this
        //   handle. Here it’s not necessary because we wait for all events to
        //   arrive anyway.
        let _join_handle = tokio::task::spawn({
            let backup_service = {
                let state = self.state().await;
                Arc::clone(&state.backup_service)
            };

            let backups_version = self.constants.backups_version;
            let blueprint = self.constants.backup_blueprints[&backups_version].clone();

            async move {
                let result = Self::post_backups_(
                    &backup_service,
                    description,
                    prose_pod_api_data,
                    backups_version,
                    &blueprint,
                    &mut event_handler,
                )
                .await;

                sender
                    .send(CreateBackupEvent::End(result.map_err(anyhow::Error::from)))
                    .await
                    .unwrap_or_else(|err| {
                        debug_panic_or_log_error!("End event send error: {err:#}")
                    });
            }
        });

        Ok(receiver)
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

    async fn put_backup_restore_<EventHandler>(
        backup_service: &BackupService,
        backup_id: String,
        blueprint: &ArchiveBlueprint,
        event_handler: &mut EventHandler,
        prose_pod_api: &ProsePodApiV2,
    ) -> Result<(), anyhow::Error>
    where
        EventHandler: ExtractBackupEventHandler + RestoreBackupEventHandler,
    {
        let backup_id = BackupId::from_str(&backup_id)?;

        let ExtractionSuccess {
            extraction_output, ..
        } = backup_service
            .extract_backup(&backup_id, event_handler)
            .await?;

        let prose_pod_api_data_path = extraction_output
            .tmp_dir
            .path()
            .join(PROSE_POD_API_ARCHIVE_KEY);

        let prose_pod_api_data = {
            let mut tar = tar::Builder::new(Vec::<u8>::new());
            tar.append_dir_all(PROSE_POD_API_ARCHIVE_KEY, &prose_pod_api_data_path)?;
            tar.into_inner()?
        };

        () = prose_pod_api
            .put_restore(std::io::Cursor::new(prose_pod_api_data))
            .await?;

        std::fs::remove_dir_all(prose_pod_api_data_path)?;

        let _response = backup_service
            .restore_extracted_backup(&backup_id, extraction_output, blueprint, event_handler)
            .await?;

        Ok(())
    }

    /// `PUT /backups/{backup_id}/restore`.
    async fn put_backup_restore(
        &self,
        backup_id: String,
        prose_pod_api: &ProsePodApiV2,
    ) -> Result<(), anyhow::Error> {
        let state = self.state().await;

        let blueprint = self
            .constants
            .backup_blueprints
            .get(&self.constants.backups_version)
            .unwrap();

        Self::put_backup_restore_(
            &state.backup_service,
            backup_id,
            blueprint,
            &mut NoopEventHandler,
            prose_pod_api,
        )
        .await
    }

    /// `PUT /backups/{backup_id}/restore Accept:text/event-stream`.
    async fn put_backup_restore_stream(
        &self,
        backup_id: String,
        prose_pod_api: &ProsePodApiV2,
    ) -> Result<mpsc::Receiver<RestoreBackupEvent>, anyhow::Error> {
        // Stream restore progress.
        let (mut event_handler, sender, receiver) = {
            let (sender, receiver) = mpsc::channel(8);

            struct EventHandler {
                backup_size: u64,
                progress: u64,
                progress_sender: Arc<mpsc::Sender<RestoreBackupEvent>>,
            }

            impl ExtractBackupEventHandler for EventHandler {
                fn on_restoration_start(&mut self, _backup_id: &BackupId, backup_size: u64) {
                    assert_eq!(self.progress, 0);

                    self.backup_size = backup_size;

                    tokio::task::block_in_place(move || {
                        tokio::runtime::Handle::current().block_on(async move {
                            self.progress_sender
                                .send(RestoreBackupEvent::Progress {
                                    progress: 0,
                                    total: backup_size,
                                })
                                .await
                                .unwrap_or_else(|err| {
                                    debug_panic_or_log_error!("Progress init error: {err:#}")
                                });
                        })
                    })
                }

                fn on_raw_read(&mut self, _backup_id: &BackupId, len: usize) {
                    assert_ne!(self.backup_size, 0);

                    if len == 0 {
                        return;
                    }

                    self.progress += len as u64;

                    tokio::task::block_in_place(move || {
                        tokio::runtime::Handle::current().block_on(async move {
                            self.progress_sender
                                .send(RestoreBackupEvent::Progress {
                                    progress: self.progress,
                                    total: self.backup_size,
                                })
                                .await
                                .unwrap_or_else(|err| {
                                    debug_panic_or_log_error!("Progress send error: {err:#}")
                                });
                        })
                    })
                }
            }

            impl RestoreBackupEventHandler for EventHandler {}

            let sender = Arc::new(sender);

            (
                EventHandler {
                    backup_size: 0,
                    progress: 0,
                    progress_sender: Arc::clone(&sender),
                },
                sender,
                receiver,
            )
        };

        // NOTE: In a real API there would likely be a system for awaiting this
        //   handle. Here it’s not necessary because we wait for all events to
        //   arrive anyway.
        let _join_handle = tokio::task::spawn({
            let backup_service = {
                let state = self.state().await;
                Arc::clone(&state.backup_service)
            };

            let blueprint = self
                .constants
                .backup_blueprints
                .get(&self.constants.backups_version)
                .unwrap()
                .to_owned();

            let prose_pod_api = prose_pod_api.to_owned();

            async move {
                let result = Self::put_backup_restore_(
                    &backup_service,
                    backup_id,
                    &blueprint,
                    &mut event_handler,
                    &prose_pod_api,
                )
                .await;

                sender
                    .send(RestoreBackupEvent::End(result.map_err(anyhow::Error::from)))
                    .await
                    .unwrap_or_else(|err| {
                        debug_panic_or_log_error!("End event send error: {err:#}")
                    });
            }
        });

        Ok(receiver)
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
#[derive(Clone)]
pub struct ProsePodApiV2 {
    constants: Arc<ProsePodApiConstants>,
    server_api: Arc<ProsePodServerApiV2>,
}

/// This would be hard-coded as constants in the Prose Pod API code.
pub struct ProsePodApiConstants {
    backup_data_key_self: String,
}

impl ProsePodApiConstants {
    fn v2() -> Self {
        Self {
            backup_data_key_self: PROSE_POD_API_ARCHIVE_KEY.to_owned(),
        }
    }
}

impl ProsePodApiV2 {
    async fn put_restore(&self, data: impl std::io::Read) -> Result<(), anyhow::Error> {
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
        blueprints.insert(1, blueprint_v2(&fs_root));

        Self {
            backups_version: 1,
            backup_blueprints: blueprints,
        }
    }
}

pub(super) fn blueprint_v2(root: impl AsRef<Path>) -> ArchiveBlueprint {
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

// MARK: API state

pub struct ProsePodServerState {
    // NOTE: Just to highlight the fact that `BackupService::from_config`
    //   doesn’t consume its configuration.
    #[allow(dead_code)]
    config: ProsePodServerConfig,
    backup_service: Arc<BackupService>,
}

impl ProsePodServerState {
    fn new_v2(constants: &ProsePodServerApiConstants) -> Result<Self, anyhow::Error> {
        let fs_root = env_required!(EXAMPLE_TMPDIR_VAR_NAME);
        let config_path = Path::new(&fs_root).join("etc/prose/prose.toml");

        unsafe {
            map_env!("S3_BUCKET_NAME_BACKUPS" -> "PROSE_BACKUPS__STORAGE__BACKUPS__S3__BUCKET_NAME");
            map_env!("S3_BUCKET_NAME_CHECKS" -> "PROSE_BACKUPS__STORAGE__CHECKS__S3__BUCKET_NAME");
            map_env!("S3_REGION" -> "PROSE_BACKUPS__S3__REGION");
            map_env!("S3_ENDPOINT_URL" -> "PROSE_BACKUPS__S3__ENDPOINT_URL");
            map_env!("S3_ACCESS_KEY" -> "PROSE_BACKUPS__S3__ACCESS_KEY");
            map_env!("S3_SECRET_KEY" -> "PROSE_BACKUPS__S3__SECRET_KEY");
        };
        let config: ProsePodServerConfig = load_config(&config_path)?;
        // tracing::debug!("Parsed config: {api_config:#?}");

        let backup_service =
            BackupService::from_config(&config.backups, constants.backup_blueprints.clone())?;

        Ok(Self {
            config,
            backup_service: Arc::new(backup_service),
        })
    }
}
