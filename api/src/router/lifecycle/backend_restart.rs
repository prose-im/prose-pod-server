// prose-pod-server
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use axum::extract::State;

use crate::errors;
use crate::responders::Error;
use crate::state::prelude::*;
use crate::util::either::Either;

// MARK: - Routes

pub(in crate::router) async fn backend_start_again(
    State(app_state): State<AppState<f::Running, b::Starting>>,
) -> Result<(), Error> {
    match app_state.do_bootstrapping().await {
        Ok(_) => Ok(()),
        Err(FailState { error, .. }) => Err(error),
    }
}

pub(in crate::router) async fn backend_start_retry(
    State(app_state): State<AppState<f::Running, b::StartFailed>>,
) -> Result<(), Error> {
    let app_state = app_state.with_auto_transition::<_, b::Starting>();
    match app_state.do_bootstrapping().await {
        Ok(_new_state) => Ok(()),
        Err(FailState { error, .. }) => Err(error),
    }
}

pub(in crate::router) async fn backend_restart(
    State(app_state): State<AppState<f::Running, b::Running>>,
) -> Result<(), Error> {
    // Stop Prosody.
    {
        let mut prosody = app_state.backend.prosody.write().await;
        prosody.stop().await.unwrap();
    }

    match app_state
        .with_backend(b::Restarting {})
        .do_restart_backend()
        .await
    {
        Ok(_) => Ok(()),

        Err(Either::E1(FailState { error, .. }) | Either::E2(FailState { error, .. })) => {
            Err(error)
        }
    }
}

pub(in crate::router) async fn backend_restart_again(
    State(app_state): State<AppState<f::Running, b::Restarting>>,
) -> Result<(), Error> {
    match app_state.do_restart_backend().await {
        Ok(_new_state) => Ok(()),

        Err(Either::E1(FailState { error, .. }) | Either::E2(FailState { error, .. })) => {
            Err(error)
        }
    }
}

pub(in crate::router) async fn backend_restart_retry(
    State(app_state): State<AppState<f::Running, b::RestartFailed>>,
) -> Result<(), Error> {
    match app_state
        .set_backend_restarting()
        .do_restart_backend()
        .await
    {
        Ok(_new_state) => Ok(()),

        Err(Either::E1(FailState { error, .. }) | Either::E2(FailState { error, .. })) => {
            Err(error)
        }
    }
}

// MARK: - State transitions

impl<F: frontend::State, B: backend::State> AppState<F, B> {
    /// ```txt
    /// AppState<F, B>
    ///   F ∈ { Running, RunningWithMisconfiguration }
    ///   B ∈ { Running, Restarting }
    /// ---------------------------------------------- (Restart backend)
    /// AppState<F, Running>        if success
    /// AppState<F, Running>        if stop failed
    /// AppState<F, RestartFailed>  if start failed
    /// ```
    ///
    /// NOTE: This method **does** log errors.
    pub(crate) async fn do_restart_backend(
        self,
    ) -> Result<
        AppState<F, b::Running>,
        Either<FailState<F, b::Running>, FailState<F, b::RestartFailed>>,
    >
    where
        F: AsRef<f::Running>,
        for<'a> F: From<(F, &'a crate::responders::Error)>,
        B: Into<b::Restarting>,
        AppState<F, b::Running>: AppStateTrait,
        AppState<F, b::Restarting>: AppStateTrait,
        AppState<F, b::RestartFailed>: AppStateTrait,
    {
        let app_state = self.set_backend_restarting();

        match app_state.try_bootstrapping().await {
            Ok(new_state) => Ok(new_state),

            Err((new_state, err)) => {
                let error = err.context("Backend restart failed");

                // Log debug info.
                tracing::error!("{error:?}");

                Err(Either::E2(new_state.transition_failed(
                    errors::internal_server_error(
                        &error,
                        "RESTART_FAILED",
                        "Something went wrong while restarting your Prose Server. \
                        Contact an administrator to fix this.",
                    ),
                )))
            }
        }
    }

    /// ```txt
    /// AppState<F, B>
    ///   F ∈ { Running, RunningWithMisconfiguration }
    ///   B ∈ { Running, RestartFailed, UndergoingBackup }
    /// -------------------------------------------------- (Set backend restarting)
    /// AppState<F, Restarting>
    /// ```
    pub(crate) fn set_backend_restarting<'a>(self) -> AppState<F, b::Restarting>
    where
        B: Into<b::Restarting>,
        AppState<F, b::Restarting>: AppStateTrait,
    {
        self.with_auto_transition::<F, b::Restarting>()
    }

    /// ```txt
    /// AppState<F, B>
    ///   F ∈ { Running, RunningWithMisconfiguration }
    ///   B ∈ { Stopped, StartFailed, UndergoingFactoryReset }
    /// ------------------------------------------------------ (Set backend starting)
    /// AppState<F, Starting>
    /// ```
    pub(crate) fn set_backend_starting<'a>(self) -> AppState<F, b::Starting>
    where
        B: Into<b::Starting>,
        AppState<F, b::Starting>: AppStateTrait,
    {
        self.with_auto_transition::<F, b::Starting>()
    }
}
