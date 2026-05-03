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
use axum::http::{HeaderMap, HeaderValue};
use axum::response::sse::{self, Sse};
use axum_extra::either::Either;
use prose_backup::archiving::{AdditionalData, ArchiveBlueprint, TarSizeCalculator};
use prose_backup::dtos::{BackupDto, BackupMetadataFullDto, BackupMetadataPartialDto};
use prose_backup::event_handlers::NoopEventHandler;
use prose_backup::{
    BackupId, BackupService, CreateBackupCommand, CreateBackupError, CreateBackupEventHandler,
    CreateBackupSuccess, RestoreBackupEventHandler, RestoreBackupPartialSuccess, tar,
};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

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

#[derive(serde::Deserialize)]
pub struct CreateBackupRequest {
    pub description: String,
}

pub(super) async fn post_backups_all(
    headers: HeaderMap,
    State(app_state): State<AppState<f::Running, b::Running>>,
    caller_info: CallerInfo,
    // NOTE: We pass that in the query because the body is already used to pass
    //   a byte stream. It’s internal and only temporary so let’s ignore it.
    Query(req): Query<CreateBackupRequest>,
    prose_pod_api_data: Bytes,
) -> Either<
    Result<Json<CreateBackupSuccess>, crate::responders::Error>,
    Result<Sse<ReceiverStream<Result<sse::Event, anyhow::Error>>>, crate::responders::Error>,
> {
    if let Err(err) = caller_info.check_is_admin() {
        return Either::E1(Err(err));
    };

    match headers.get(axum::http::header::CONTENT_TYPE) {
        Some(value) if value == "application/x-tar" => {}
        _ => {
            return Either::E1(Err(errors::unsupported_media_type(
                "BAD_REQUEST",
                "Bad request",
                "Body should be `application/x-tar`.",
            )));
        }
    }

    if prose_pod_api_data.is_empty() {
        return Either::E1(Err(errors::validation_error(
            "BAD_REQUEST",
            "Bad request",
            "Missing Prose Pod API data.",
        )));
    }

    match headers.get(reqwest::header::ACCEPT) {
        Some(val) if val.as_bytes() == b"text/event-stream" => {
            Either::E2(post_backups_stream(app_state, req, prose_pod_api_data).await)
        }
        _ => Either::E1(post_backups(app_state, req, prose_pod_api_data).await),
    }
}

/// `POST /v1/backups`.
async fn post_backups(
    app_state: AppState<f::Running, b::Running>,
    req: CreateBackupRequest,
    prose_pod_api_data: Bytes,
) -> Result<Json<CreateBackupSuccess>, crate::responders::Error> {
    post_backups_(
        app_state,
        req.description,
        prose_pod_api_data,
        &mut NoopEventHandler,
    )
    .await
    .map(Json)
}

/// `POST /v1/backups Accept:text/event-stream`.
async fn post_backups_stream(
    app_state: AppState<f::Running, b::Running>,
    req: CreateBackupRequest,
    prose_pod_api_data: Bytes,
) -> Result<Sse<ReceiverStream<Result<sse::Event, anyhow::Error>>>, crate::responders::Error> {
    // Stream backup progress.
    let (mut event_handler, sender, receiver) = {
        let (sender, receiver) = mpsc::channel(8);

        let sender = Arc::new(sender);

        (
            StreamingCreateBackupEventHandler {
                backup_id: None,
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
    tokio::task::spawn(async move {
        let result = post_backups_(
            app_state,
            req.description,
            prose_pod_api_data,
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
    });

    Ok(Sse::new(ReceiverStream::new(receiver)))
}

async fn post_backups_<F: frontend::State>(
    app_state: AppState<F, b::Running>,
    description: String,
    prose_pod_api_data: Bytes,
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
    let blueprint = (BACKUP_BLUEPRINTS.get(&BACKUPS_VERSION))
        .expect("A blueprint should always exist for BACKUPS_VERSION");

    let backup_service = Arc::clone(app_state.backend.backup_service()?);

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

    let response = backup_service.create_backup(command, event_handler).await;

    let _app_state = app_state.do_restart_backend().await;

    response.map_err(crate::responders::Error::from)
}

/// `GET /v1/backups`.
pub(super) async fn get_backups(
    State(AppState { ref backend, .. }): State<AppState>,
    caller_info: CallerInfo,
) -> Result<Json<Vec<BackupDto<BackupMetadataPartialDto>>>, crate::responders::Error> {
    caller_info.check_is_admin()?;

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
    caller_info.check_is_admin()?;

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
    caller_info.check_is_admin()?;

    let backup_service = backend.backup_service()?;

    backup_service
        .delete_backup(&backup_id)
        .await
        .no_context()?;

    Ok(())
}

pub(super) async fn put_backup_restore_all(
    headers: HeaderMap,
    State(app_state): State<AppState>,
    caller_info: CallerInfo,
    Path(backup_id): Path<BackupId>,
) -> Either<
    Result<(), crate::responders::Error>,
    Result<Sse<ReceiverStream<Result<sse::Event, axum::Error>>>, crate::responders::Error>,
> {
    if let Err(err) = caller_info.check_is_admin() {
        return Either::E1(Err(err));
    };

    let Some(prose_token) = headers.get("x-prose-token") else {
        return Either::E1(Err(errors::validation_error(
            "BAD_REQUEST",
            "Bad request",
            "Missing Prose token.",
        )));
    };

    match headers.get(reqwest::header::ACCEPT) {
        Some(val) if val.as_bytes() == b"text/event-stream" => {
            Either::E2(put_backup_restore_stream(app_state, prose_token, backup_id).await)
        }
        _ => Either::E1(put_backup_restore(app_state, prose_token, backup_id).await),
    }
}

/// `PUT /v1/backups/{backup_id}/restore`.
async fn put_backup_restore(
    app_state: AppState<f::Running, b::Running>,
    prose_token: &HeaderValue,
    backup_id: BackupId,
) -> Result<(), crate::responders::Error> {
    let backup_service = Arc::clone(app_state.backend.backup_service()?);
    let prose_pod_api = Arc::clone(&app_state.backend.prose_pod_api);

    put_backup_restore_(
        app_state,
        &backup_service,
        prose_token,
        backup_id,
        &mut NoopEventHandler,
        &prose_pod_api,
    )
    .await
}

/// `PUT /v1/backups/{backup_id}/restore Accept:text/event-stream`.
async fn put_backup_restore_stream(
    app_state: AppState<f::Running, b::Running>,
    prose_token: &HeaderValue,
    backup_id: BackupId,
) -> Result<Sse<ReceiverStream<Result<sse::Event, axum::Error>>>, crate::responders::Error> {
    let backup_service = Arc::clone(app_state.backend.backup_service()?);
    let prose_pod_api = Arc::clone(&app_state.backend.prose_pod_api);

    // Stream restore progress.
    let (mut event_handler, sender, receiver) = {
        let (sender, receiver) = mpsc::channel(8);

        let sender = Arc::new(sender);

        (
            StreamingRestoreBackupEventHandler {
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
        let prose_token = prose_token.to_owned();

        let backup_id_str = backup_id.to_string();

        async move {
            let result = put_backup_restore_(
                app_state,
                &backup_service,
                &prose_token,
                backup_id,
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

async fn put_backup_restore_<EventHandler>(
    app_state: AppState<f::Running, b::Running>,
    backup_service: &BackupService,
    prose_token: &HeaderValue,
    backup_id: BackupId,
    event_handler: &mut EventHandler,
    prose_pod_api: &ProsePodApi,
) -> Result<(), crate::responders::Error>
where
    EventHandler: RestoreBackupEventHandler + RestoreBackupEventHandler,
{
    // Stop Prosody.
    {
        let mut prosody = app_state.backend.prosody.write().await;
        prosody.stop().await.unwrap();
    }

    let app_state = app_state.with_backend(b::UndergoingRestore {});

    let res = put_backup_restore_inner(
        backup_service,
        prose_token,
        backup_id,
        event_handler,
        prose_pod_api,
    )
    .await;

    let _app_state = app_state.do_restart_backend().await;

    res
}

async fn put_backup_restore_inner<EventHandler>(
    backup_service: &BackupService,
    prose_token: &HeaderValue,
    backup_id: BackupId,
    event_handler: &mut EventHandler,
    prose_pod_api: &ProsePodApi,
) -> Result<(), crate::responders::Error>
where
    EventHandler: RestoreBackupEventHandler + RestoreBackupEventHandler,
{
    let blueprint = (BACKUP_BLUEPRINTS.get(&BACKUPS_VERSION))
        .expect("A blueprint should always exist for BACKUPS_VERSION");

    let mut event_handler = WithAdditionalData {
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

    Ok(())
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
    Query(req): Query<GetBackupDownloadUrlRequest>,
) -> Result<String, crate::responders::Error> {
    caller_info.check_is_admin()?;

    let backup_service = backend.backup_service()?;

    let ttl = req.ttl.unwrap_or(std::time::Duration::from_mins(5));

    let backup = backup_service
        .get_download_url(&backup_id, ttl)
        .await
        .no_context()?;

    Ok(backup)
}

// MARK: - Boilerplate

pub(super) const PROSE_POD_API_ARCHIVE_KEY: &str = "prose-pod-api-data";

#[repr(transparent)]
struct ProsePodApiData(Bytes);

impl AdditionalData for ProsePodApiData {
    fn expected_size(&self) -> Result<u64, anyhow::Error> {
        let archive_len = self.0.len() as u64;
        Ok(TarSizeCalculator::archive_contents_size(archive_len))
    }

    fn append<W: std::io::Write>(self, builder: &mut tar::Builder<W>) -> Result<(), anyhow::Error> {
        use tokio_util::bytes::Buf as _;

        let mut archive = tar::Archive::new(self.0.reader());
        let entries = archive.entries()?;

        for entry in entries {
            let entry = entry?;

            builder.append(&entry.header().clone(), entry)?;
        }

        Ok(())
    }
}

enum CreateBackupEvent {}

impl CreateBackupEvent {
    fn progress(backup_id: &str, progress: u64, total: u64) -> Result<sse::Event, anyhow::Error> {
        sse::Event::default()
            .event("backup-create-progress")
            .id(backup_id)
            .json_data(json::json!({
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

/// This [`CreateBackupEventHandler`] sends a [`sse::Event`] on
/// progress, throttling them while ensuring one still receives the
/// last event (100% progress).
///
/// NOTE: The throttle is subject to drift, but we don’t care.
///   It’s simple and effective, just what we want.
struct StreamingCreateBackupEventHandler {
    backup_id: Option<String>,
    total: u64,
    progress: u64,
    interval: tokio::time::Duration,
    last_event_sent: (u64, tokio::time::Instant),
    progress_sender: Arc<mpsc::Sender<Result<sse::Event, anyhow::Error>>>,
}

impl CreateBackupEventHandler for StreamingCreateBackupEventHandler {
    fn on_archive_start(&mut self, backup_id: &BackupId, expected_archive_size: u64) {
        debug_assert_eq!(self.progress, 0);

        self.backup_id = Some(backup_id.to_string());
        self.total = expected_archive_size;

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
        debug_assert_ne!(self.total, 0);

        self.progress = self.progress.saturating_add(archived_bytes as u64);
        debug_assert!(self.progress <= self.total);

        if self.last_event_sent.1.elapsed() > self.interval {
            self.last_event_sent = (self.progress, tokio::time::Instant::now());

            tokio::task::block_in_place(move || {
                tokio::runtime::Handle::current().block_on(async move {
                    self.progress_sender
                        .send(CreateBackupEvent::progress(
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

    fn on_backup_uploaded(
        &mut self,
        backup_id: &BackupId,
        _size_bytes: u64,
        _duration: std::time::Duration,
    ) {
        if self.last_event_sent.0 < self.total {
            self.last_event_sent = (self.total, tokio::time::Instant::now());

            tokio::task::block_in_place(move || {
                tokio::runtime::Handle::current().block_on(async move {
                    self.progress_sender
                        .send(CreateBackupEvent::progress(
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

// A wrapper which takes into account the fact that we also
// have to restore the Prose Pod API’s data.
struct WithAdditionalData<'a, Inner> {
    inner: &'a mut Inner,
    additional_data_size: usize,
}

impl<'a, Inner> RestoreBackupEventHandler for WithAdditionalData<'a, Inner>
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

enum RestoreBackupEvent {}

impl RestoreBackupEvent {
    fn progress(backup_id: &str, progress: u64, total: u64) -> Result<sse::Event, axum::Error> {
        sse::Event::default()
            .event("backup-restore-progress")
            .id(backup_id)
            .json_data(json::json!({
                "progress": progress,
                "total": total,
            }))
            .inspect_err(|e| debug_panic_or_log_error!("Restore progress send error: {e:#}"))
    }

    fn end(
        backup_id: &str,
        result: Result<(), crate::responders::Error>,
    ) -> Result<sse::Event, axum::Error> {
        match result {
            Ok(data) => sse::Event::default()
                .event("backup-restore-success")
                .id(backup_id)
                .json_data(data)
                .inspect_err(|e| debug_panic_or_log_error!("Restore success send error: {e:#}")),
            Err(err) => sse::Event::default()
                .event("backup-restore-error")
                .id(backup_id)
                .json_data(err.into_json())
                .inspect_err(|e| debug_panic_or_log_error!("Restore error send error: {e:#}")),
        }
    }
}

/// This [`RestoreBackupEventHandler`] sends a [`sse::Event`] on
/// progress, throttling them while ensuring one still receives the
/// last event (100% progress).
///
/// NOTE: The throttle is subject to drift, but we don’t care.
///   It’s simple and effective, just what we want.
struct StreamingRestoreBackupEventHandler {
    total: u64,
    progress: u64,
    interval: tokio::time::Duration,
    last_event_sent: (u64, tokio::time::Instant),
    progress_sender: Arc<mpsc::Sender<Result<sse::Event, axum::Error>>>,
}

impl RestoreBackupEventHandler for StreamingRestoreBackupEventHandler {
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

impl From<CreateBackupError> for crate::responders::Error {
    fn from(error: CreateBackupError) -> Self {
        errors::internal_server_error(
            &anyhow::Error::new(error),
            "BACKUP_CREATE_FAILED",
            "Something went wrong while creating the backup. Contact an administrator to fix this.",
        )
    }
}
