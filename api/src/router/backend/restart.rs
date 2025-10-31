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
use crate::state::prelude::*;

// MARK: - Routes

impl AppState<f::Running, b::Running> {
    pub(in crate::router) async fn backend_restart_route(
        State(app_state): State<Self>,
    ) -> Result<(), Error> {
        let backend_state = Arc::clone(&app_state.backend.state);
        let mut prosody = backend_state.prosody.write().await;

        match app_state.do_stop_backend::<b::Starting>(&mut prosody).await {
            Ok(app_state) => match app_state.do_start_backend(&mut prosody).await {
                Ok(_) => Ok(()),

                Err((_, error)) => {
                    tracing::error!("Backend restart failed: {error:?}");
                    Err(errors::restart_failed(&error))
                }
            },

            Err((_, error)) => {
                tracing::error!("Backend restart failed: {error:?}");
                Err(errors::restart_failed(&error))
            }
        }
    }
}

impl AppState<f::Running, b::Starting> {
    pub(in crate::router) async fn backend_restart_route(
        State(app_state): State<Self>,
    ) -> Result<(), Error> {
        let backend_state = Arc::clone(&app_state.backend.state);
        let mut prosody = backend_state.prosody.write().await;

        match app_state.do_start_backend(&mut prosody).await {
            Ok(_) => Ok(()),

            Err((_, error)) => {
                tracing::error!("{error:?}");
                Err(errors::restart_failed(&error))
            }
        }
    }
}

impl AppState<f::Running, b::StartFailed> {
    pub(in crate::router) async fn backend_start_route(
        State(app_state): State<Self>,
    ) -> Result<(), Error> {
        let backend_state = Arc::clone(&app_state.backend.state);
        let mut prosody = backend_state.prosody.write().await;

        match app_state.do_start_backend(&mut prosody).await {
            Ok(_) => Ok(()),

            Err((_, error)) => {
                tracing::error!("{error:?}");
                Err(errors::restart_failed(&error))
            }
        }
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
        (AppState<f::Running, b::StartFailed>, Arc<anyhow::Error>),
    >
    where
        b::Running: From<B>,
        Arc<b::Operational>: From<B>,
    {
        match prosody.start().await {
            Ok(()) => Ok(self.with_auto_transition::<f::Running, b::Running>()),

            Err(err) => {
                let error = err
                    .context("Could not start Prosody")
                    .context("Backend start failed");
                let error = Arc::new(error);

                // Log debug info.
                tracing::debug!("{error:?}");

                let new_state = self.with_transition::<f::Running, b::StartFailed>(|state| {
                    state.with_backend_transition(|substate| b::StartFailed {
                        state: substate.into(),
                        error: Arc::clone(&error),
                    })
                });

                Err((new_state, error))
            }
        }
    }
}
