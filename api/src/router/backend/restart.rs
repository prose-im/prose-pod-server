// prose-pod-server-api
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::routing::post;
use prosody_child_process::ProsodyChildProcess;
use tokio::sync::RwLockWriteGuard;

use crate::errors;
use crate::responders::Error;
use crate::router::util::backend_health;
use crate::state::prelude::*;

impl AppState<f::Running, b::Running> {
    pub(crate) fn backend_restart_routes() -> axum::Router<Self> {
        Router::<Self>::new().route("/backend/restart", post(Self::backend_restart_route))
    }
}

/// During a startup.
impl AppStateTrait for AppState<f::Running, b::Starting<b::NotInitialized>> {
    fn state_name() -> &'static str {
        "Starting"
    }

    fn into_router(self) -> axum::Router {
        let todo = "Keep one route for safety?";
        Router::<Self>::new()
            .fallback(backend_health)
            .with_state(self)
    }
}

/// During a restart.
impl AppStateTrait for AppState<f::Running, b::Starting> {
    fn state_name() -> &'static str {
        "Restarting"
    }

    fn into_router(self) -> axum::Router {
        Router::<Self>::new()
            // NOTE: Keep `/backend/restart` available just in case an internal
            //   error happened and we ended up stuck in this state.
            .route("/backend/restart", post(Self::backend_restart_route))
            .fallback(backend_health)
            .with_state(self)
    }
}

/// During a failed restart.
impl AppStateTrait for AppState<f::Running, b::StartFailed> {
    fn state_name() -> &'static str {
        "Start failed"
    }

    fn into_router(self) -> axum::Router {
        Router::<Self>::new()
            .route("/backend/restart", post(Self::backend_start_route))
            .fallback(backend_health)
            .with_state(self)
    }
}

// impl AppStateTrait for AppState<f::Running, b::Stopped> {
//     fn into_router(self) -> axum::Router {
//         Router::<Self>::new()
//             .route("/backend/restart", post(Self::backend_start_route))
//             .fallback(backend_health)
//             .with_state(self)
//     }
// }

impl AppState<f::Running, b::Running> {
    #[inline]
    async fn do_stop_backend<'a, B2>(
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
    #[inline]
    async fn do_start_backend<'a>(
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

impl AppState<f::Running, b::Running> {
    async fn backend_restart_route(State(app_state): State<Self>) -> Result<(), Error> {
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
    async fn backend_restart_route(State(app_state): State<Self>) -> Result<(), Error> {
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
    async fn backend_start_route(State(app_state): State<Self>) -> Result<(), Error> {
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
