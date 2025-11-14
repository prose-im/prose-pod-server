// prose-pod-server
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::sync::Arc;

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
    match app_state.do_restart_backend().await {
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

impl<B: backend::State> AppState<f::Running, B> {
    /// ```txt
    /// AppState<Running, B>  B ∈ { Running, Restarting }
    /// ------------------------------------------------- (Restart backend)
    /// AppState<Running, Running>        if success
    /// AppState<Running, Running>        if stop failed
    /// AppState<Running, RestartFailed>  if start failed
    /// ```
    ///
    /// NOTE: This method **does** log errors.
    pub(crate) async fn do_restart_backend<'a>(
        self,
    ) -> Result<
        AppState<f::Running, b::Running>,
        Either<FailState<f::Running, b::Running>, FailState<f::Running, b::RestartFailed>>,
    >
    where
        B: AsRef<Arc<b::Operational>> + Into<b::Restarting> + Into<b::Running>,
    {
        let backend_state = Arc::clone(self.backend.as_ref());
        let mut prosody = backend_state.prosody.write().await;

        if prosody.is_running().await {
            if let Err(err) = prosody.stop().await {
                let error = err
                    .context("Could not stop Prosody")
                    .context("Backend restart failed");

                // Log debug info.
                tracing::warn!("{error:?}");

                // Do not transition to a failure state if the backend failed
                // to stop. It means it’s still running. There could be some
                // edge cases where it’s in fact an internal error that is
                // thrown after the backend has stopped but in that case we’d
                // have to fix that code so it doesn’t happen.
                let new_state = self.set_backend_running();

                return Err(Either::E1(new_state.with_error(
                    errors::internal_server_error(
                        &error,
                        "RESTART_FAILED",
                        "Something went wrong while restarting your Prose Server. \
                        Contact an administrator to fix this.",
                    ),
                )));
            }
        }

        let app_state = self.set_backend_restarting();

        match prosody.start().await {
            Ok(()) => Ok(app_state.set_backend_running()),

            Err(err) => {
                let error = err
                    .context("Could not start Prosody")
                    .context("Backend restart failed");

                // Log debug info.
                tracing::error!("{error:?}");

                Err(Either::E2(app_state.transition_failed(
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
    /// AppState<Running, B>  B ∈ { Running, RestartFailed }
    /// ---------------------------------------------------- (Set backend restarting)
    /// AppState<Running, Restarting>
    /// ```
    pub(crate) fn set_backend_restarting<'a>(self) -> AppState<f::Running, b::Restarting>
    where
        B: Into<b::Restarting>,
    {
        self.with_auto_transition()
    }

    /// ```txt
    /// AppState<Running, B>
    ///   B ∈ { Stopped, StartFailed, UndergoingFactoryReset }
    /// ------------------------------------------------------ (Set backend starting)
    /// AppState<Running, Starting>
    /// ```
    pub(crate) fn set_backend_starting<'a>(self) -> AppState<f::Running, b::Starting>
    where
        B: Into<b::Starting>,
    {
        self.with_auto_transition()
    }

    /// ```txt
    /// AppState<Running, B>  B ∈ { Restarting }
    /// ---------------------------------------- (Set backend running)
    /// AppState<Running, Running>
    /// ```
    pub(crate) fn set_backend_running<'a>(self) -> AppState<f::Running, b::Running>
    where
        B: Into<b::Running>,
    {
        self.with_auto_transition()
    }
}
