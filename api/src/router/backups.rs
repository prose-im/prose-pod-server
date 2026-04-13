// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use anyhow::Context as _;
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::sse::{self, Sse};
use axum_extra::either::Either;
use json::json;
use prose_backup::archiving::{AdditionalData, ArchiveBlueprint, TarSizeCalculator};
use prose_backup::dtos::{BackupDto, BackupMetadataFullDto, BackupMetadataPartialDto};
use prose_backup::event_handlers::NoopEventHandler;
use prose_backup::{
    BackupId, BackupService, CreateBackupCommand, CreateBackupError, CreateBackupEventHandler,
    CreateBackupSuccess, ExtractBackupEventHandler, ExtractionSuccess, RestoreBackupEventHandler,
};
use reqwest::header::ACCEPT;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::bytes::Buf;

use crate::errors;
use crate::models::{AuthToken, CallerInfo};
use crate::prose_pod_api::ProsePodApi;
use crate::state::prelude::*;
use crate::util::{NoContext as _, debug_panic_or_log_error};

const BACKUPS_VERSION: u8 = 1;

pub static BACKUP_BLUEPRINTS: LazyLock<HashMap<u8, ArchiveBlueprint>> = LazyLock::new(|| {
    let mut hash_map = HashMap::with_capacity(1);

    hash_map.insert(
        1,
        ArchiveBlueprint::new(
            1,
            [
                ("prose-pod-server-data", "/var/lib/prose-pod-server"),
                ("prosody-data", "/var/lib/prosody"),
                ("prose-config", "/etc/prose"),
                ("prosody-config", "/etc/prosody"),
            ],
        ),
    );

    hash_map
});

async fn post_backups_<F: frontend::State>(
    app_state: AppState<F, b::Running>,
    description: String,
    prose_pod_api_data: Bytes,
    blueprint: &ArchiveBlueprint,
    event_handler: &mut impl CreateBackupEventHandler,
) -> Result<CreateBackupSuccess, crate::responders::Error>
where
    F: AsRef<f::Running>,
    for<'a> F: From<(F, &'a crate::responders::Error)>,
    AppState<F, b::Running>: AppStateTrait,
    AppState<F, b::UndergoingBackup>: AppStateTrait,
    AppState<F, b::Restarting>: AppStateTrait,
    AppState<F, b::RestartFailed>: AppStateTrait,
{
    let Some(backup_service) = app_state.backend.backup_service.as_ref().map(Arc::clone) else {
        return Err(crate::errors::configuration_error(
            "MISSING_CONFIG",
            "Missing configuration",
            "Backups configuration not initialized.",
        ));
    };

    // Stop Prosody.
    {
        let mut prosody = app_state.backend.prosody.write().await;
        prosody.stop().await.unwrap();
    }

    let app_state = app_state.with_backend(b::UndergoingBackup {});

    let command = CreateBackupCommand {
        prefix: "prose-backup",
        description: &description,
        blueprint,
        additional_archive_data: Some(ProsePodApiData(prose_pod_api_data)),
    };

    let response = backup_service.create_backup(command, event_handler).await?;

    let _app_state = app_state.do_restart_backend().await;

    Ok(response)
}

pub(super) const PROSE_POD_API_ARCHIVE_KEY: &str = "prose-pod-api-data";

#[repr(transparent)]
struct ProsePodApiData(Bytes);

impl AdditionalData for ProsePodApiData {
    fn expected_size(&self) -> Result<u64, anyhow::Error> {
        let archive_len = self.0.len() as u64;
        Ok(TarSizeCalculator::archive_contents_size(archive_len))
    }

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

#[derive(serde::Deserialize)]
pub struct CreateBackupRequest {
    pub description: String,
}

pub(super) async fn post_backups_all(
    headers: HeaderMap,
    state: State<AppState<f::Running, b::Running>>,
    caller_info: CallerInfo,
    query: Query<CreateBackupRequest>,
    prose_pod_api_data: Bytes,
) -> Either<
    Result<Json<CreateBackupSuccess>, crate::responders::Error>,
    Result<Sse<ReceiverStream<Result<sse::Event, anyhow::Error>>>, crate::responders::Error>,
> {
    match headers.get(ACCEPT) {
        Some(val) if val.as_bytes() == b"text/event-stream" => {
            Either::E2(post_backups_stream(state, caller_info, query, prose_pod_api_data).await)
        }
        _ => Either::E1(post_backups(state, caller_info, query, prose_pod_api_data).await),
    }
}

/// `POST /backups`.
async fn post_backups(
    State(app_state): State<AppState<f::Running, b::Running>>,
    caller_info: CallerInfo,
    // NOTE: We pass that in the query because the body is already used to pass
    //   a byte stream. It’s internal and only temporary so let’s ignore it.
    Query(req): Query<CreateBackupRequest>,
    prose_pod_api_data: Bytes,
) -> Result<Json<CreateBackupSuccess>, crate::responders::Error> {
    // Ensure the caller is an admin.
    // FIXME: Make this more flexible by checking rights instead of roles
    //   (which can be extended).
    match caller_info.primary_role.as_str() {
        "prosody:admin" | "prosody:operator" => {}
        _ => return Err(errors::forbidden("Only admins can do that.")),
    }

    let blueprint = BACKUP_BLUEPRINTS.get(&BACKUPS_VERSION).unwrap();

    post_backups_(
        app_state,
        req.description,
        prose_pod_api_data,
        blueprint,
        &mut NoopEventHandler,
    )
    .await
    .map(Json)
}

enum CreateBackupEvent {}

impl CreateBackupEvent {
    fn progress(backup_id: &str, progress: u64, total: u64) -> Result<sse::Event, anyhow::Error> {
        sse::Event::default()
            .event("backup-create-progress")
            .id(backup_id)
            .json_data(json!({
                "progress": progress,
                "total": total,
            }))
            .map_err(|err| {
                debug_panic_or_log_error!("{err:#}");
                anyhow::Error::from(err)
            })
    }

    fn end(
        backup_id: &str,
        result: Result<CreateBackupSuccess, crate::responders::Error>,
    ) -> Result<sse::Event, anyhow::Error> {
        match result {
            Ok(data) => sse::Event::default()
                .event("backup-create-success")
                .id(backup_id)
                .json_data(data),
            Err(err) => sse::Event::default()
                .event("backup-create-error")
                .id(backup_id)
                .json_data(err.into_json()),
        }
        .map_err(|err| {
            debug_panic_or_log_error!("{err:#}");
            anyhow::Error::from(err)
        })
    }
}

/// `POST /backups Accept:text/event-stream`.
async fn post_backups_stream(
    State(app_state): State<AppState<f::Running, b::Running>>,
    caller_info: CallerInfo,
    // NOTE: We pass that in the query because the body is already used to pass
    //   a byte stream. It’s internal and only temporary so let’s ignore it.
    Query(req): Query<CreateBackupRequest>,
    prose_pod_api_data: Bytes,
) -> Result<Sse<ReceiverStream<Result<sse::Event, anyhow::Error>>>, crate::responders::Error> {
    // Ensure the caller is an admin.
    // FIXME: Make this more flexible by checking rights instead of roles
    //   (which can be extended).
    match caller_info.primary_role.as_str() {
        "prosody:admin" | "prosody:operator" => {}
        _ => return Err(errors::forbidden("Only admins can do that.")),
    }

    // Stream backup progress.
    let (mut event_handler, sender, receiver) = {
        let (sender, receiver) = mpsc::channel(8);

        struct EventHandler {
            backup_id: Option<String>,
            expected_archive_size: u64,
            effective_archive_size: u64,
            progress_sender: Arc<mpsc::Sender<Result<sse::Event, anyhow::Error>>>,
        }

        impl CreateBackupEventHandler for EventHandler {
            fn on_archive_start(&mut self, backup_id: &BackupId, expected_archive_size: u64) {
                debug_assert_eq!(self.effective_archive_size, 0);

                self.backup_id = Some(backup_id.to_string());
                self.expected_archive_size = expected_archive_size;

                tokio::task::block_in_place(move || {
                    tokio::runtime::Handle::current().block_on(async move {
                        self.progress_sender
                            .send(CreateBackupEvent::progress(
                                &backup_id.to_string(),
                                0,
                                expected_archive_size,
                            ))
                            .await
                            .unwrap_or_else(|err| {
                                debug_panic_or_log_error!("Progress init error: {err:#}")
                            });
                    })
                })
            }

            fn on_archive_progress(&mut self, backup_id: &BackupId, archived_bytes: usize) {
                debug_assert_ne!(self.expected_archive_size, 0);

                self.effective_archive_size += archived_bytes as u64;

                tokio::task::block_in_place(move || {
                    tokio::runtime::Handle::current().block_on(async move {
                        self.progress_sender
                            .send(CreateBackupEvent::progress(
                                &backup_id.to_string(),
                                self.effective_archive_size,
                                self.expected_archive_size,
                            ))
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
                backup_id: None,
                expected_archive_size: 0,
                effective_archive_size: 0,
                progress_sender: Arc::clone(&sender),
            },
            sender,
            receiver,
        )
    };

    // NOTE: No need to get the `JoinHandle`, we can fire-and-forget this.
    tokio::task::spawn({
        let blueprint = BACKUP_BLUEPRINTS.get(&BACKUPS_VERSION).unwrap();

        async move {
            let result = post_backups_(
                app_state,
                req.description,
                prose_pod_api_data,
                &blueprint,
                &mut event_handler,
            )
            .await;

            let backup_id = event_handler.backup_id.unwrap_or_else(|| {
                debug_panic_or_log_error!("`backup_id` should be assigned by now.");
                String::new()
            });

            sender
                .send(CreateBackupEvent::end(&backup_id, result))
                .await
                .unwrap_or_else(|err| debug_panic_or_log_error!("End event send error: {err:#}"));
        }
    });

    Ok(Sse::new(ReceiverStream::new(receiver)))
}

/// `GET /backups`.
pub(super) async fn get_backups(
    State(AppState { ref backend, .. }): State<AppState>,
    caller_info: CallerInfo,
) -> Result<Json<Vec<BackupDto<BackupMetadataPartialDto>>>, crate::responders::Error> {
    // Ensure the caller is an admin.
    // FIXME: Make this more flexible by checking rights instead of roles
    //   (which can be extended).
    match caller_info.primary_role.as_str() {
        "prosody:admin" | "prosody:operator" => {}
        _ => return Err(errors::forbidden("Only admins can do that.")),
    }

    let backup_service = backend.backup_service()?;

    let backups = backup_service.list_backups().await.no_context()?;

    Ok(Json(backups))
}

/// `GET /backups/{backup_id}`.
pub(super) async fn get_backup(
    State(AppState { ref backend, .. }): State<AppState>,
    caller_info: CallerInfo,
    Path(backup_id): Path<BackupId>,
) -> Result<Json<BackupDto<BackupMetadataFullDto>>, crate::responders::Error> {
    // Ensure the caller is an admin.
    // FIXME: Make this more flexible by checking rights instead of roles
    //   (which can be extended).
    match caller_info.primary_role.as_str() {
        "prosody:admin" | "prosody:operator" => {}
        _ => return Err(errors::forbidden("Only admins can do that.")),
    }

    let backup_service = backend.backup_service()?;

    let backup = backup_service.get_details(&backup_id).await.no_context()?;

    Ok(Json(backup))
}

/// `DELETE /backups/{backup_id}`.
pub(super) async fn delete_backup(
    State(AppState { ref backend, .. }): State<AppState>,
    caller_info: CallerInfo,
    Path(backup_id): Path<BackupId>,
) -> Result<(), crate::responders::Error> {
    // Ensure the caller is an admin.
    // FIXME: Make this more flexible by checking rights instead of roles
    //   (which can be extended).
    match caller_info.primary_role.as_str() {
        "prosody:admin" | "prosody:operator" => {}
        _ => return Err(errors::forbidden("Only admins can do that.")),
    }

    let backup_service = backend.backup_service()?;

    backup_service
        .delete_backup(&backup_id)
        .await
        .no_context()?;

    Ok(())
}

async fn put_backup_restore_<EventHandler>(
    backup_service: &BackupService,
    token: &AuthToken,
    backup_id: BackupId,
    blueprint: &ArchiveBlueprint,
    event_handler: &mut EventHandler,
    prose_pod_api: &ProsePodApi,
) -> Result<(), crate::responders::Error>
where
    EventHandler: ExtractBackupEventHandler + RestoreBackupEventHandler,
{
    let ExtractionSuccess {
        extraction_output, ..
    } = backup_service
        .extract_backup(&backup_id, event_handler)
        .await
        .map_err(|error| {
            crate::errors::internal_server_error(
                &anyhow::Error::from(error),
                "BACKUP_RESTORE_FAILED",
                "Something went wrong while restoring the backup.",
            )
        })?;

    let prose_pod_api_data_path = extraction_output
        .tmp_dir
        .path()
        .join(PROSE_POD_API_ARCHIVE_KEY);

    let prose_pod_api_data = {
        let mut tar = tar::Builder::new(Vec::<u8>::new());
        tar.append_dir_all(PROSE_POD_API_ARCHIVE_KEY, &prose_pod_api_data_path)
            .no_context()?;
        tar.into_inner().no_context()?
    };

    () = prose_pod_api
        .put_restore(token, std::io::Cursor::new(prose_pod_api_data))
        .await
        .context("Prose Pod API restoration failed.")
        .map_err(|error| {
            crate::errors::internal_server_error(
                &error,
                "BACKUP_RESTORE_FAILED",
                "Something went wrong while restoring the backup.",
            )
        })?;

    std::fs::remove_dir_all(prose_pod_api_data_path).no_context()?;

    let _response = backup_service
        .restore_extracted_backup(&backup_id, extraction_output, blueprint, event_handler)
        .await
        .map_err(|error| {
            crate::errors::internal_server_error(
                &anyhow::Error::from(error),
                "BACKUP_RESTORE_FAILED",
                "Something went wrong while restoring the backup.",
            )
        })?;

    Ok(())
}

pub(super) async fn put_backup_restore_all(
    headers: HeaderMap,
    state: State<AppState>,
    caller_info: CallerInfo,
    auth_token: AuthToken,
    path: Path<BackupId>,
) -> Either<
    Result<(), crate::responders::Error>,
    Result<Sse<ReceiverStream<Result<sse::Event, anyhow::Error>>>, crate::responders::Error>,
> {
    match headers.get(ACCEPT) {
        Some(val) if val.as_bytes() == b"text/event-stream" => {
            Either::E2(put_backup_restore_stream(state, caller_info, auth_token, path).await)
        }
        _ => Either::E1(put_backup_restore(state, caller_info, auth_token, path).await),
    }
}

/// `PUT /backups/{backup_id}/restore`.
async fn put_backup_restore(
    State(AppState { ref backend, .. }): State<AppState>,
    caller_info: CallerInfo,
    auth_token: AuthToken,
    Path(backup_id): Path<BackupId>,
) -> Result<(), crate::responders::Error> {
    // Ensure the caller is an admin.
    // FIXME: Make this more flexible by checking rights instead of roles
    //   (which can be extended).
    match caller_info.primary_role.as_str() {
        "prosody:admin" | "prosody:operator" => {}
        _ => return Err(errors::forbidden("Only admins can do that.")),
    }

    let backup_service = backend.backup_service()?;

    let blueprint = BACKUP_BLUEPRINTS.get(&BACKUPS_VERSION).unwrap();

    put_backup_restore_(
        backup_service,
        &auth_token,
        backup_id,
        blueprint,
        &mut NoopEventHandler,
        &backend.prose_pod_api,
    )
    .await
}

enum RestoreBackupEvent {}

impl RestoreBackupEvent {
    fn progress(backup_id: &str, progress: u64, total: u64) -> Result<sse::Event, anyhow::Error> {
        sse::Event::default()
            .event("backup-restore-progress")
            .id(backup_id)
            .json_data(json!({
                "progress": progress,
                "total": total,
            }))
            .map_err(|err| {
                debug_panic_or_log_error!("{err:#}");
                anyhow::Error::from(err)
            })
    }

    fn end(
        backup_id: &str,
        result: Result<(), crate::responders::Error>,
    ) -> Result<sse::Event, anyhow::Error> {
        match result {
            Ok(data) => sse::Event::default()
                .event("backup-restore-success")
                .id(backup_id)
                .json_data(data),
            Err(err) => sse::Event::default()
                .event("backup-restore-error")
                .id(backup_id)
                .json_data(err.into_json()),
        }
        .map_err(|err| {
            debug_panic_or_log_error!("{err:#}");
            anyhow::Error::from(err)
        })
    }
}

/// `PUT /backups/{backup_id}/restore Accept:text/event-stream`.
async fn put_backup_restore_stream(
    State(AppState { ref backend, .. }): State<AppState>,
    caller_info: CallerInfo,
    auth_token: AuthToken,
    Path(backup_id): Path<BackupId>,
) -> Result<Sse<ReceiverStream<Result<sse::Event, anyhow::Error>>>, crate::responders::Error> {
    // Ensure the caller is an admin.
    // FIXME: Make this more flexible by checking rights instead of roles
    //   (which can be extended).
    match caller_info.primary_role.as_str() {
        "prosody:admin" | "prosody:operator" => {}
        _ => return Err(errors::forbidden("Only admins can do that.")),
    }

    let backup_service = backend.backup_service()?;

    // Stream restore progress.
    let (mut event_handler, sender, receiver) = {
        let (sender, receiver) = mpsc::channel(8);

        struct EventHandler {
            backup_size: u64,
            progress: u64,
            progress_sender: Arc<mpsc::Sender<Result<sse::Event, anyhow::Error>>>,
        }

        impl ExtractBackupEventHandler for EventHandler {
            fn on_restoration_start(&mut self, backup_id: &BackupId, backup_size: u64) {
                assert_eq!(self.progress, 0);

                self.backup_size = backup_size;

                tokio::task::block_in_place(move || {
                    tokio::runtime::Handle::current().block_on(async move {
                        self.progress_sender
                            .send(RestoreBackupEvent::progress(
                                &backup_id.to_string(),
                                0,
                                backup_size,
                            ))
                            .await
                            .unwrap_or_else(|err| {
                                debug_panic_or_log_error!("Progress init error: {err:#}")
                            });
                    })
                })
            }

            fn on_raw_read(&mut self, backup_id: &BackupId, len: usize) {
                assert_ne!(self.backup_size, 0);

                if len == 0 {
                    return;
                }

                self.progress += len as u64;

                tokio::task::block_in_place(move || {
                    tokio::runtime::Handle::current().block_on(async move {
                        self.progress_sender
                            .send(RestoreBackupEvent::progress(
                                &backup_id.to_string(),
                                self.progress,
                                self.backup_size,
                            ))
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

    // NOTE: No need to get the `JoinHandle`, we can fire-and-forget this.
    tokio::task::spawn({
        let backup_service = Arc::clone(backup_service);

        let blueprint = BACKUP_BLUEPRINTS.get(&BACKUPS_VERSION).unwrap().to_owned();

        let prose_pod_api = backend.prose_pod_api.to_owned();

        let backup_id_str = backup_id.to_string();

        async move {
            let result = put_backup_restore_(
                &backup_service,
                &auth_token,
                backup_id,
                &blueprint,
                &mut event_handler,
                &prose_pod_api,
            )
            .await;

            sender
                .send(RestoreBackupEvent::end(&backup_id_str, result))
                .await
                .unwrap_or_else(|err| debug_panic_or_log_error!("End event send error: {err:#}"));
        }
    });

    Ok(Sse::new(ReceiverStream::new(receiver)))
}

#[derive(Debug)]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GetBackupDownloadUrlRequest {
    pub ttl: Option<std::time::Duration>,
}

/// `GET /backups/{backup_id}/download-url`.
pub(super) async fn get_backup_download_url(
    State(AppState { ref backend, .. }): State<AppState>,
    caller_info: CallerInfo,
    Path(backup_id): Path<BackupId>,
    Json(req): Json<GetBackupDownloadUrlRequest>,
) -> Result<String, crate::responders::Error> {
    // Ensure the caller is an admin.
    // FIXME: Make this more flexible by checking rights instead of roles
    //   (which can be extended).
    match caller_info.primary_role.as_str() {
        "prosody:admin" | "prosody:operator" => {}
        _ => return Err(errors::forbidden("Only admins can do that.")),
    }

    let backup_service = backend.backup_service()?;

    let ttl = req.ttl.unwrap_or(std::time::Duration::from_mins(5));

    let backup = backup_service
        .get_download_url(&backup_id, ttl)
        .await
        .no_context()?;

    Ok(backup)
}

// MARK: - Boilerplate

impl From<CreateBackupError> for crate::responders::Error {
    fn from(error: CreateBackupError) -> Self {
        match error {
            CreateBackupError::CannotCreateSink(error) => todo!(),
            CreateBackupError::CannotArchive(cannot_archive) => todo!(),
            CreateBackupError::ArchivingFailed(error) => todo!(),
            CreateBackupError::CannotCompress(error) => todo!(),
            CreateBackupError::CompressionFailed(error) => todo!(),
            CreateBackupError::CannotEncrypt(error) => todo!(),
            CreateBackupError::EncryptionFailed(error) => todo!(),
            CreateBackupError::HashingFailed(error) => todo!(),
            CreateBackupError::CannotSign(error) => todo!(),
            CreateBackupError::SigningFailed(error) => todo!(),
            CreateBackupError::UploadFailed(error) => todo!(),
            CreateBackupError::IntegrityCheckUploadFailed(error) => todo!(),
            CreateBackupError::Other(error) => todo!(),
        }
    }
}
