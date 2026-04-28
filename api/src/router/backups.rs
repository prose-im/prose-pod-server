// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::usize;

use anyhow::Context as _;
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue};
use axum::response::sse::{self, Sse};
use axum_extra::either::Either;
use json::json;
use prose_backup::archiving::{AdditionalData, ArchiveBlueprint, TarSizeCalculator};
use prose_backup::dtos::{BackupDto, BackupMetadataFullDto, BackupMetadataPartialDto};
use prose_backup::event_handlers::NoopEventHandler;
use prose_backup::{
    BackupId, BackupService, CreateBackupCommand, CreateBackupError, CreateBackupEventHandler,
    CreateBackupSuccess, RestoreBackupEventHandler, RestoreBackupPartialSuccess, tar,
};
use prosody_child_process::ProsodyChildProcess;
use reqwest::header::ACCEPT;
use tokio::sync::{RwLock, RwLockWriteGuard, mpsc};
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::bytes::Buf;

use crate::errors;
use crate::models::CallerInfo;
use crate::prose_pod_api::ProsePodApi;
use crate::state::prelude::*;
use crate::util::{NoContext as _, debug_panic_or_log_error};

pub const BACKUPS_VERSION: u8 = 1;

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
        Some(val) if val.as_bytes() == b"text/event-stream" => Either::E2(
            post_backups_stream(state, caller_info, headers, query, prose_pod_api_data).await,
        ),
        _ => Either::E1(post_backups(state, caller_info, headers, query, prose_pod_api_data).await),
    }
}

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
        prefix: "prose_backup",
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

/// `POST /v1/backups`.
async fn post_backups(
    State(app_state): State<AppState<f::Running, b::Running>>,
    caller_info: CallerInfo,
    headers: HeaderMap,
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

    match headers.get(axum::http::header::CONTENT_TYPE) {
        Some(value) if value == "application/x-tar" => {}
        _ => {
            return Err(errors::unsupported_media_type(
                "BAD_REQUEST",
                "Bad request",
                "Body should be `application/x-tar`.",
            ));
        }
    }

    if prose_pod_api_data.is_empty() {
        return Err(errors::validation_error(
            "BAD_REQUEST",
            "Bad request",
            "Missing Prose Pod API data.",
        ));
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

/// `POST /v1/backups Accept:text/event-stream`.
async fn post_backups_stream(
    State(app_state): State<AppState<f::Running, b::Running>>,
    caller_info: CallerInfo,
    headers: HeaderMap,
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

    match headers.get(axum::http::header::CONTENT_TYPE) {
        Some(value) if value == "application/x-tar" => {}
        _ => {
            return Err(errors::unsupported_media_type(
                "BAD_REQUEST",
                "Bad request",
                "Body should be `application/x-tar`.",
            ));
        }
    }

    if prose_pod_api_data.is_empty() {
        return Err(errors::validation_error(
            "BAD_REQUEST",
            "Bad request",
            "Missing Prose Pod API data.",
        ));
    }

    // Stream backup progress.
    let (mut event_handler, sender, receiver) = {
        let (sender, receiver) = mpsc::channel(8);

        /// This [`CreateBackupEventHandler`] sends a [`sse::Event`] on
        /// progress, throttling them while ensuring one still receives the
        /// last event (100% progress).
        ///
        /// NOTE: The throttle is subject to drift, but we don’t care.
        ///   It’s simple and effective, just what we want.
        struct EventHandler {
            backup_id: Option<String>,
            expected_archive_size: u64,
            effective_archive_size: u64,
            interval: tokio::time::Duration,
            last_event_sent: (u64, tokio::time::Instant),
            progress_sender: Arc<mpsc::Sender<Result<sse::Event, anyhow::Error>>>,
        }

        impl CreateBackupEventHandler for EventHandler {
            fn on_archive_start(&mut self, backup_id: &BackupId, expected_archive_size: u64) {
                debug_assert_eq!(self.effective_archive_size, 0);

                self.backup_id = Some(backup_id.to_string());
                self.expected_archive_size = expected_archive_size;

                self.last_event_sent = (0, tokio::time::Instant::now());

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

                self.effective_archive_size = self
                    .effective_archive_size
                    .saturating_add(archived_bytes as u64);
                debug_assert!(self.effective_archive_size <= self.expected_archive_size);

                if self.last_event_sent.1.elapsed() > self.interval {
                    self.last_event_sent =
                        (self.effective_archive_size, tokio::time::Instant::now());

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

            fn on_backup_uploaded(
                &mut self,
                backup_id: &BackupId,
                _size_bytes: u64,
                _duration: std::time::Duration,
            ) {
                if self.last_event_sent.0 < self.expected_archive_size {
                    self.last_event_sent =
                        (self.expected_archive_size, tokio::time::Instant::now());

                    tokio::task::block_in_place(move || {
                        tokio::runtime::Handle::current().block_on(async move {
                            self.progress_sender
                                .send(CreateBackupEvent::progress(
                                    &backup_id.to_string(),
                                    self.expected_archive_size,
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
        }

        let sender = Arc::new(sender);

        (
            EventHandler {
                backup_id: None,
                expected_archive_size: 0,
                effective_archive_size: 0,
                // TODO: Parameterize this?
                interval: tokio::time::Duration::from_millis(100),
                last_event_sent: (0, tokio::time::Instant::now()),
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

/// `GET /v1/backups`.
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

/// `GET /v1/backups/{backup_id}`.
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

/// `DELETE /v1/backups/{backup_id}`.
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

pub(super) async fn put_backup_restore_all(
    headers: HeaderMap,
    state: State<AppState>,
    caller_info: CallerInfo,
    path: Path<BackupId>,
) -> Either<
    Result<(), crate::responders::Error>,
    Result<Sse<ReceiverStream<Result<sse::Event, anyhow::Error>>>, crate::responders::Error>,
> {
    let Some(prose_token) = headers.get("x-prose-token") else {
        return Either::E1(Err(errors::validation_error(
            "BAD_REQUEST",
            "Bad request",
            "Missing Prose token.",
        )));
    };

    let fixme = "Change app state so health status isn’t success (Pod API needs an error)";

    match headers.get(ACCEPT) {
        Some(val) if val.as_bytes() == b"text/event-stream" => {
            Either::E2(put_backup_restore_stream(state, caller_info, prose_token, path).await)
        }
        _ => Either::E1(put_backup_restore(state, caller_info, prose_token, path).await),
    }
}

async fn put_backup_restore_<EventHandler>(
    prosody: &RwLock<ProsodyChildProcess>,
    backup_service: &BackupService,
    prose_token: &HeaderValue,
    backup_id: BackupId,
    blueprint: &ArchiveBlueprint,
    event_handler: &mut EventHandler,
    prose_pod_api: &ProsePodApi,
) -> Result<(), crate::responders::Error>
where
    EventHandler: RestoreBackupEventHandler + RestoreBackupEventHandler,
{
    // A wrapper which takes into account the fact that we also
    // have to restore the Prose Pod API’s data.
    struct EventHandler<'a, Inner> {
        inner: &'a mut Inner,
        additional_data_size: usize,
    }

    impl<'a, Inner> RestoreBackupEventHandler for EventHandler<'a, Inner>
    where
        Inner: RestoreBackupEventHandler,
    {
        fn on_restoration_start(&mut self, backup_id: &BackupId, mut total: u64) {
            // This is just an estimate. It doesn’t have to be exact, just
            // to be there so the progress bar doesn’t reach 100% before
            // the Prose Pod API data is restored.
            let additional_data_size = total / 10;

            self.additional_data_size = usize::try_from(additional_data_size).unwrap_or(usize::MAX);

            total = total.saturating_add(additional_data_size);

            self.inner.on_restoration_start(backup_id, total);
        }

        fn on_restoration_progress(&mut self, backup_id: &BackupId, len: usize) {
            self.inner.on_restoration_progress(backup_id, len);
        }

        fn on_decryption_finished(
            &mut self,
            backup_id: &BackupId,
            stats: prose_backup::stats::ReadStats,
            report: prose_backup::decryption::DecryptionReport,
        ) {
            self.inner.on_decryption_finished(backup_id, stats, report);
        }

        fn on_decompression_finished(
            &mut self,
            backup_id: &BackupId,
            stats: prose_backup::stats::ReadStats,
        ) {
            self.inner.on_decompression_finished(backup_id, stats);
        }

        fn on_extraction_finished(
            &mut self,
            backup_id: &BackupId,
            report: prose_backup::archiving::ExtractionReport,
        ) {
            self.inner.on_extraction_finished(backup_id, report);
        }

        fn on_restoration_finished(&mut self, backup_id: &BackupId) {
            self.inner.on_restoration_finished(backup_id);
        }
    }

    let mut prosody = prosody.write().await;
    prosody.stop().await.map_err(|error| {
        crate::errors::internal_server_error(
            &anyhow::Error::from(error),
            "BACKUP_RESTORE_FAILED",
            "Something went wrong while restoring the backup.",
        )
    })?;

    struct RestoreGuard<'a> {
        prosody: Option<RwLockWriteGuard<'a, ProsodyChildProcess>>,
    }

    impl<'a> Drop for RestoreGuard<'a> {
        fn drop(&mut self) {
            if let Some(mut prosody) = self.prosody.take() {
                tokio::task::block_in_place(move || {
                    tokio::runtime::Handle::current().block_on(async move {
                        if let Err(err) = prosody.start().await {
                            tracing::error!("[Drop] Failed restarting Prosody: {err:#}");
                        };
                    })
                })
            }
        }
    }

    let mut restore_guard = RestoreGuard {
        prosody: Some(prosody),
    };

    let mut event_handler = EventHandler {
        inner: event_handler,
        additional_data_size: 0,
    };

    let RestoreBackupPartialSuccess {
        mut restoration_output,
        ..
    } = backup_service
        .restore_backup_partial(&backup_id, blueprint, &mut event_handler)
        .await
        .map_err(|error| {
            crate::errors::internal_server_error(
                &anyhow::Error::from(error),
                "BACKUP_RESTORE_FAILED",
                "Something went wrong while restoring the backup.",
            )
        })?;

    let Some((tmp_dir, revert_guard)) = restoration_output.additional_data.as_mut() else {
        tracing::error!("Invalid backup: Missing Prose Pod API data.");
        return Err(crate::errors::validation_error(
            "INVALID_BACKUP",
            "Invalid backup",
            "This backup is missing data. It can’t be restored.",
        ));
    };

    {
        let prose_pod_api_data_path = tmp_dir.path().join(PROSE_POD_API_ARCHIVE_KEY);

        if !prose_pod_api_data_path.is_dir() {
            tracing::error!(
                "Invalid backup: `{path}` is missing.",
                path = prose_pod_api_data_path.display()
            );
            return Err(crate::errors::validation_error(
                "INVALID_BACKUP",
                "Invalid backup",
                "This backup is missing data. It can’t be restored.",
            ));
        }

        let prose_pod_api_data = {
            let mut tar = tar::Builder::new(Vec::<u8>::new());
            tar.append_dir_all(PROSE_POD_API_ARCHIVE_KEY, &prose_pod_api_data_path)
                .no_context()?;
            tar.into_inner().no_context()?
        };

        () = prose_pod_api
            .put_restore(prose_token, std::io::Cursor::new(prose_pod_api_data))
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

        event_handler.on_restoration_progress(&backup_id, event_handler.additional_data_size);
    }

    revert_guard.defuse();

    event_handler.on_restoration_finished(&backup_id);

    match std::fs::read_dir(tmp_dir) {
        Ok(entries) => {
            for entry in entries {
                match entry {
                    Ok(entry) => {
                        let file_name = entry.file_name();
                        tracing::warn!("Extracted unknown entry {file_name:?}.")
                    }
                    Err(err) => debug_panic_or_log_error!("{err:?}"),
                }
            }
        }
        Err(err) => debug_panic_or_log_error!("Error reading temporary directory: {err:?}"),
    }

    let mut prosody = restore_guard.prosody.take().unwrap();

    prosody.start().await.map_err(|error| {
        crate::errors::internal_server_error(
            &anyhow::Error::from(error),
            "RESTART_FAILED",
            "Something went wrong while restarting your Prose Server. \
            Contact an administrator to fix this.",
        )
    })?;

    Ok(())
}

/// `PUT /v1/backups/{backup_id}/restore`.
async fn put_backup_restore(
    State(AppState { ref backend, .. }): State<AppState>,
    caller_info: CallerInfo,
    prose_token: &HeaderValue,
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
        &backend.prosody,
        backup_service,
        prose_token,
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

/// `PUT /v1/backups/{backup_id}/restore Accept:text/event-stream`.
async fn put_backup_restore_stream(
    State(AppState { ref backend, .. }): State<AppState>,
    caller_info: CallerInfo,
    prose_token: &HeaderValue,
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

        /// This [`RestoreBackupEventHandler`] sends a [`sse::Event`] on
        /// progress, throttling them while ensuring one still receives the
        /// last event (100% progress).
        ///
        /// NOTE: The throttle is subject to drift, but we don’t care.
        ///   It’s simple and effective, just what we want.
        struct EventHandler {
            total: u64,
            progress: u64,
            interval: tokio::time::Duration,
            last_event_sent: (u64, tokio::time::Instant),
            progress_sender: Arc<mpsc::Sender<Result<sse::Event, anyhow::Error>>>,
        }

        impl RestoreBackupEventHandler for EventHandler {
            fn on_restoration_start(&mut self, backup_id: &BackupId, total: u64) {
                assert_eq!(self.progress, 0);

                self.total = total;

                self.last_event_sent = (0, tokio::time::Instant::now());

                tokio::task::block_in_place(move || {
                    tokio::runtime::Handle::current().block_on(async move {
                        self.progress_sender
                            .send(RestoreBackupEvent::progress(
                                &backup_id.to_string(),
                                0,
                                total,
                            ))
                            .await
                            .unwrap_or_else(|err| {
                                debug_panic_or_log_error!("Progress init error: {err:#}")
                            });
                    })
                })
            }

            fn on_restoration_progress(&mut self, backup_id: &BackupId, len: usize) {
                assert_ne!(self.total, 0);

                if len == 0 {
                    return;
                }

                self.progress = self.progress.saturating_add(len as u64);
                debug_assert!(self.progress <= self.total);

                if self.last_event_sent.1.elapsed() > self.interval {
                    self.last_event_sent = (self.progress, tokio::time::Instant::now());

                    tokio::task::block_in_place(move || {
                        tokio::runtime::Handle::current().block_on(async move {
                            self.progress_sender
                                .send(RestoreBackupEvent::progress(
                                    &backup_id.to_string(),
                                    self.progress,
                                    self.total,
                                ))
                                .await
                                .unwrap_or_else(|err| {
                                    debug_panic_or_log_error!("Progress send error: {err:#}")
                                });
                        })
                    })
                }
            }

            fn on_restoration_finished(&mut self, backup_id: &BackupId) {
                if self.last_event_sent.0 < self.total {
                    self.last_event_sent = (self.total, tokio::time::Instant::now());

                    tokio::task::block_in_place(move || {
                        tokio::runtime::Handle::current().block_on(async move {
                            self.progress_sender
                                .send(RestoreBackupEvent::progress(
                                    &backup_id.to_string(),
                                    self.total,
                                    self.total,
                                ))
                                .await
                                .unwrap_or_else(|err| {
                                    debug_panic_or_log_error!("Progress send error: {err:#}")
                                });
                        })
                    })
                }
            }
        }

        let sender = Arc::new(sender);

        (
            EventHandler {
                total: 0,
                progress: 0,
                // TODO: Parameterize this?
                interval: tokio::time::Duration::from_millis(100),
                last_event_sent: (0, tokio::time::Instant::now()),
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

        let prosody = Arc::clone(&backend.prosody);
        let prose_pod_api = Arc::clone(&backend.prose_pod_api);
        let prose_token = prose_token.to_owned();

        let backup_id_str = backup_id.to_string();

        async move {
            let result = put_backup_restore_(
                &prosody,
                &backup_service,
                &prose_token,
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

/// `GET /v1/backups/{backup_id}/download-url`.
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
        errors::internal_server_error(
            &anyhow::Error::new(error),
            "BACKUP_CREATE_FAILED",
            "Something went wrong while creating the backup. Contact an administrator to fix this.",
        )
    }
}
