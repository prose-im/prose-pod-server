// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! The version 2 of the Prose Pod API, where the Prose Pod API has state
//! and it calls the Prose Pod Server API for some operations.

use std::{path::Path, str::FromStr as _, sync::Arc};

use anyhow::Context;
use prose_backup::archiving::ArchiveBlueprint;
use prose_backup::event_handlers::NoopEventHandler;
use prose_backup::restoration::ArchiveMigration;
use prose_backup::{
    BackupConfig, BackupId, BackupService, CreateBackupCommand, CreateBackupEventHandler,
    ExtractBackupEventHandler, ExtractionSuccess, RestoreBackupEventHandler,
};
use tokio::sync::RwLock;

use crate::common::{lifecycle::EXAMPLE_TMPDIR_VAR_NAME, util::*};
use crate::prose::api::v2::blueprint_v2;

use super::*;

// MARK: - Public API

#[async_trait::async_trait]
impl ProsePodApi for ProsePodApiV3 {
    async fn post_backups(
        &self,
        description: String,
    ) -> Result<CreateBackupSuccess, anyhow::Error> {
        let state = self.state().await;

        let backups_version = self.constants.backups_version;
        let blueprint = &self.constants.backup_blueprints[&backups_version];

        Self::post_backups_(
            &state.backup_service,
            description,
            blueprint,
            &mut NoopEventHandler,
        )
        .await
    }

    async fn post_backups_stream(
        &self,
        description: String,
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

    async fn get_backups(&self) -> Result<Vec<BackupDto<BackupMetadataPartialDto>>, anyhow::Error> {
        let state = self.state().await;

        let backups = state.backup_service.list_backups().await?;

        Ok(backups)
    }

    async fn get_backup(
        &self,
        backup_id: String,
    ) -> Result<BackupDto<BackupMetadataFullDto>, anyhow::Error> {
        let state = self.state().await;

        let backup_id = BackupId::from_str(&backup_id)?;

        let backup = state.backup_service.get_details(&backup_id).await?;

        Ok(backup)
    }

    async fn delete_backup(&self, backup_id: String) -> Result<(), anyhow::Error> {
        let state = self.state().await;

        let backup_id = BackupId::from_str(&backup_id)?;

        state.backup_service.delete_backup(&backup_id).await?;

        Ok(())
    }

    async fn put_backup_restore(&self, backup_id: String) -> Result<(), anyhow::Error> {
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
        )
        .await
    }

    async fn put_backup_restore_stream(
        &self,
        backup_id: String,
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

            async move {
                let result = Self::put_backup_restore_(
                    &backup_service,
                    backup_id,
                    &blueprint,
                    &mut event_handler,
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

pub fn start_v3() -> Result<ProsePodApiV3, anyhow::Error> {
    let constants = ProsePodApiConstants::v3();
    let state = ProsePodServerState::new_v3(&constants)?;

    Ok(ProsePodApiV3 {
        constants,
        state: RwLock::new(state),
    })
}

// MARK: - Internals

impl ProsePodApiV3 {
    async fn post_backups_(
        backup_service: &BackupService,
        description: String,
        blueprint: &ArchiveBlueprint,
        event_handler: &mut impl CreateBackupEventHandler,
    ) -> Result<CreateBackupSuccess, anyhow::Error> {
        let command = CreateBackupCommand::new(
            concat!("example-", env!("CARGO_CRATE_NAME")),
            &description,
            blueprint,
        );

        let response = backup_service.create_backup(command, event_handler).await?;

        Ok(response)
    }

    async fn put_backup_restore_<EventHandler>(
        backup_service: &BackupService,
        backup_id: String,
        blueprint: &ArchiveBlueprint,
        event_handler: &mut EventHandler,
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

        let _response = backup_service
            .restore_extracted_backup(&backup_id, extraction_output, blueprint, event_handler)
            .await?;

        Ok(())
    }
}

// MARK: - Implementation details

// MARK: Prose Pod API

/// Prose Pod API, when [“Prose Pod API as part of Prose Pod Server”](https://github.com/prose-im/prose-pod-api/discussions/368)
/// will have been implemented.
pub struct ProsePodApiV3 {
    constants: ProsePodApiConstants,
    state: RwLock<ProsePodServerState>,
}

impl ProsePodApiV3 {
    async fn state(&self) -> RwLockReadGuard<'_, ProsePodServerState> {
        self.state.read().await
    }
}

/// This would be hard-coded as constants in the Prose Pod API code.
pub struct ProsePodApiConstants {
    backups_version: u8,
    backup_blueprints: HashMap<u8, ArchiveBlueprint>,
    backup_migrations: Vec<ArchiveMigration>,
}

impl ProsePodApiConstants {
    fn v3() -> Self {
        let fs_root = env_required!(EXAMPLE_TMPDIR_VAR_NAME);

        let mut blueprints: HashMap<u8, ArchiveBlueprint> = HashMap::with_capacity(2);
        blueprints.insert(1, blueprint_v2(&fs_root));
        blueprints.insert(2, blueprint_v3(&fs_root));

        let migrations = vec![
            ArchiveMigration::new(
                2,
                [
                    ("prose-pod-server-data", "prose-data"),
                    (v2::PROSE_POD_API_ARCHIVE_KEY, "prose-data"),
                ],
            ),
        ];

        Self {
            backups_version: 2,
            backup_blueprints: blueprints,
            backup_migrations: migrations,
        }
    }
}

pub(super) fn blueprint_v3(root: impl AsRef<Path>) -> ArchiveBlueprint {
    ArchiveBlueprint::new(
        2,
        [
            ("prose-data", "var/lib/prose"),
            ("prosody-data", "var/lib/prosody"),
            ("prose-config", "etc/prose"),
            ("prosody-config", "etc/prosody"),
        ]
        .into_iter()
        .map(|(dst, src)| (dst, root.as_ref().join(src))),
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
    fn new_v3(constants: &ProsePodApiConstants) -> Result<Self, anyhow::Error> {
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

        let backup_service = BackupService::from_config(
            &config.backups,
            constants.backup_blueprints.clone(),
            constants.backup_migrations.clone(),
        )?;

        Ok(Self {
            config,
            backup_service: Arc::new(backup_service),
        })
    }
}
