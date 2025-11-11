// prose-pod-server-api
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::sync::Arc;

use axum::extract::State;
use prosody_child_process::ProsodyChildProcess;
use tokio::sync::RwLockWriteGuard;

use crate::errors;
use crate::responders::Error;
use crate::state::{FailState, prelude::*};

// MARK: - Routes

pub(in crate::router) async fn backend_restart(
    State(app_state): State<AppState<f::Running, b::Running>>,
) -> Result<(), Error> {
    let backend_state = Arc::clone(&app_state.backend.state);
    let mut prosody = backend_state.prosody.write().await;

    match app_state.do_stop_backend::<b::Starting>(&mut prosody).await {
        Ok(app_state) => match app_state.do_start_backend(&mut prosody).await {
            Ok(_) => Ok(()),

            Err(FailState { error, .. }) => {
                tracing::error!("Backend restart failed: {error:?}");
                Err(errors::restart_failed(&error))
            }
        },

        Err((_state, error)) => {
            tracing::error!("Backend restart failed: {error:?}");
            Err(errors::restart_failed(&error))
        }
    }
}

pub(in crate::router) async fn backend_start(
    State(app_state): State<AppState<f::Running, b::Starting>>,
) -> Result<(), Error> {
    let backend_state = Arc::clone(&app_state.backend.state);
    let mut prosody = backend_state.prosody.write().await;

    match app_state.do_start_backend(&mut prosody).await {
        Ok(_) => Ok(()),

        Err(FailState { error, .. }) => {
            tracing::error!("{error:?}");
            Err(errors::restart_failed(&error))
        }
    }
}

pub(in crate::router) async fn backend_start_retry(
    State(app_state): State<AppState<f::Running, b::StartFailed<b::Operational>>>,
) -> Result<(), Error> {
    let backend_state = Arc::clone(&app_state.backend.state);
    let mut prosody = backend_state.prosody.write().await;

    match app_state.do_start_backend(&mut prosody).await {
        Ok(_new_state) => Ok(()),

        Err(FailState { error, .. }) => {
            tracing::error!("{error:?}");
            Err(errors::restart_failed(&error))
        }
    }
}

pub(in crate::router) async fn backend_init_retry<B>(
    State(app_state): State<AppState<f::Running, B>>,
) -> Result<(), Error>
where
    B: Into<Arc<b::NotInitialized>>,
{
    match app_state.do_bootstrapping().await {
        Ok(_new_state) => Ok(()),
        Err(FailState { error, .. }) => Err(errors::restart_failed(&error)),
    }
}

// MARK: - State transitions

impl AppState<f::Running, b::Running> {
    /// ```txt
    /// AppState<Running, Running>
    /// -------------------------------------- (Stop backend)
    /// AppState<Running, B>  B ∈ { Starting }
    /// ```
    pub(crate) async fn do_stop_backend<'a, B2>(
        self,
        prosody: &mut RwLockWriteGuard<'a, ProsodyChildProcess>,
    ) -> Result<AppState<f::Running, B2>, (Self, Arc<anyhow::Error>)>
    where
        B2: crate::router::HealthTrait + Send + Sync + 'static + Clone,
        B2: From<b::Running>,
        AppState<f::Running, B2>: AppStateTrait,
    {
        match prosody.stop().await {
            Ok(()) => Ok(self.with_auto_transition::<f::Running, B2>()),

            Err(err) => {
                let error = err.context("Could not stop Prosody");
                let error = Arc::new(error);

                // Log debug info.
                tracing::debug!("{error:?}");

                // Do not transition state if the backend failed to stop. It means
                // it’s still running. There could be some edge cases where it’s in
                // fact an internal error that is thrown after the backend has stopped
                // but in that case we’d have to fix that code so it doesn’t happen.

                Err((self, error))
            }
        }
    }
}

impl<B> AppState<f::Running, B> {
    /// ```txt
    /// AppState<Running, B>  B ∈ { Starting, StartFailed }
    /// --------------------------------------------------- (Start backend)
    /// AppState<Running, Running>      if success
    /// AppState<Running, StartFailed>  if failure
    /// ```
    pub(crate) async fn do_start_backend<'a>(
        self,
        prosody: &mut RwLockWriteGuard<'a, ProsodyChildProcess>,
    ) -> Result<
        AppState<f::Running, b::Running>,
        FailState<f::Running, b::StartFailed<b::Operational>>,
    >
    where
        b::Running: From<B>,
        Arc<b::Operational>: From<B>,
    {
        match prosody.start().await {
            Ok(()) => Ok(self.with_auto_transition()),

            Err(err) => {
                let error = err
                    .context("Could not start Prosody")
                    .context("Backend start failed");

                // Log debug info.
                tracing::debug!("{error:?}");

                Err(self.transition_failed(error))
            }
        }
    }
}
