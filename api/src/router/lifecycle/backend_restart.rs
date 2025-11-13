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

        Err(FailState { error, .. }) => Err(errors::start_failed(&error)),
    }
}

pub(in crate::router) async fn backend_start_retry(
    State(app_state): State<AppState<f::Running, b::StartFailed>>,
) -> Result<(), Error> {
    let app_state = app_state.with_auto_transition::<_, b::Starting>();
    match app_state.do_bootstrapping().await {
        Ok(_new_state) => Ok(()),

        Err(FailState { error, .. }) => Err(errors::start_failed(&error)),
    }
}

pub(in crate::router) async fn backend_restart(
    State(app_state): State<AppState<f::Running, b::Running>>,
) -> Result<(), Error> {
    match app_state.do_restart_backend().await {
        Ok(_) => Ok(()),

        Err(Either::E1(FailState { error, .. }) | Either::E2(FailState { error, .. })) => {
            Err(errors::restart_failed(&error))
        }
    }
}

pub(in crate::router) async fn backend_restart_again(
    State(app_state): State<AppState<f::Running, b::Restarting>>,
) -> Result<(), Error> {
    match app_state.do_restart_backend().await {
        Ok(_new_state) => Ok(()),

        Err(Either::E1(FailState { error, .. }) | Either::E2(FailState { error, .. })) => {
            Err(errors::restart_failed(&error))
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
            Err(errors::restart_failed(&error))
        }
    }
}

// MARK: - State transitions

impl AppState<f::Running, b::Running> {
    /// ```txt
    /// AppState<Running, Running>
    /// -------------------------------------------- (Restart backend)
    /// AppState<Running, Running>        if success
    /// AppState<Running, RestartFailed>  if failure
    /// ```
    ///
    /// NOTE: This method **does** log errors.
    pub(crate) async fn do_restart_backend<'a>(
        self,
    ) -> Result<
        AppState<f::Running, b::Running>,
        Either<FailState<f::Running, b::Running>, FailState<f::Running, b::RestartFailed>>,
    > {
        let backend_state = Arc::clone(&self.backend.state);
        let mut prosody = backend_state.prosody.write().await;

        match prosody.stop().await {
            Ok(()) => {
                let app_state = self.with_auto_transition::<_, b::Restarting>();

                match prosody.start().await {
                    Ok(()) => Ok(app_state.with_auto_transition()),

                    Err(err) => {
                        let error = err
                            .context("Could not start Prosody")
                            .context("Backend restart failed");

                        // Log debug info.
                        tracing::error!("{error:?}");

                        Err(Either::E2(app_state.transition_failed(error)))
                    }
                }
            }

            Err(err) => {
                let error = err
                    .context("Could not stop Prosody")
                    .context("Backend restart failed");
                let error = Arc::new(error);

                // Log debug info.
                tracing::warn!("{error:?}");

                // Do not transition state if the backend failed to stop. It means
                // it’s still running. There could be some edge cases where it’s in
                // fact an internal error that is thrown after the backend has stopped
                // but in that case we’d have to fix that code so it doesn’t happen.

                Err(Either::E1(self.with_error(error)))
            }
        }
    }
}

impl AppState<f::Running, b::Restarting> {
    /// ```txt
    /// AppState<Running, Restarting>
    /// -------------------------------------------- (Retry backend restart)
    /// AppState<Running, Running>        if success
    /// AppState<Running, RestartFailed>  if failure
    /// ```
    ///
    /// NOTE: This method **does** log errors.
    pub(crate) async fn do_restart_backend<'a>(
        self,
    ) -> Result<
        AppState<f::Running, b::Running>,
        Either<FailState<f::Running, b::Running>, FailState<f::Running, b::RestartFailed>>,
    > {
        let backend_state = Arc::clone(&self.backend.state);
        let mut prosody = backend_state.prosody.write().await;

        if prosody.is_running().await {
            if let Err(err) = prosody.stop().await {
                let error = err
                    .context("Could not stop Prosody")
                    .context("Backend restart failed");
                let error = Arc::new(error);

                // Log debug info.
                tracing::warn!("{error:?}");

                // Do not transition state if the backend failed to stop. It means
                // it’s still running. There could be some edge cases where it’s in
                // fact an internal error that is thrown after the backend has stopped
                // but in that case we’d have to fix that code so it doesn’t happen.

                return Err(Either::E1(self.with_auto_transition().with_error(error)));
            }
        }

        let app_state = self.with_auto_transition::<_, b::Restarting>();

        match prosody.start().await {
            Ok(()) => Ok(app_state.with_auto_transition()),

            Err(err) => {
                let error = err
                    .context("Could not start Prosody")
                    .context("Backend restart failed");

                // Log debug info.
                tracing::error!("{error:?}");

                Err(Either::E2(app_state.transition_failed(error)))
            }
        }
    }
}

impl AppState<f::Running, b::RestartFailed> {
    /// ```txt
    /// AppState<Running, RestartFailed>
    /// -------------------------------- (Set backend retrying restart)
    /// AppState<Running, Restarting>
    /// ```
    pub(crate) fn set_backend_restarting<'a>(self) -> AppState<f::Running, b::Restarting> {
        self.with_auto_transition()
    }
}
