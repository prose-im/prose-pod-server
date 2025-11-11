// prose-pod-server-api
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use axum::extract::State;

use crate::errors;
use crate::responders::Error;
use crate::state::{FailState, prelude::*};
use crate::util::either::Either;

pub(in crate::router) async fn reload<FrontendSubstate>(
    State(app_state): State<AppState<f::Running<FrontendSubstate>, b::Running>>,
) -> Result<(), Error>
where
    FrontendSubstate: FrontendRunningState,
    AppState<f::Running<FrontendSubstate>, b::Running>: AppStateTrait,
{
    match app_state.try_reload_frontend() {
        Ok(new_state) => {
            let _new_state = new_state.do_reload_backend().await?;
            Ok(())
        }

        Err((_, error)) => {
            // Log debug info.
            tracing::error!("{error:?}");

            Err(errors::bad_configuration(&error))
        }
    }
}

impl AppState<f::Misconfigured, b::Stopped<b::NotInitialized>> {
    pub async fn do_init_config(
        self,
    ) -> Result<
        AppState<f::Running, b::Running>,
        Either<
            FailState<f::Misconfigured, b::Stopped<b::NotInitialized>>,
            FailState<f::Running, b::StartFailed<b::NotInitialized>>,
        >,
    > {
        match self.try_reload_frontend::<b::Starting<b::NotInitialized>>() {
            Ok(app_state) => app_state.do_bootstrapping().await.map_err(Either::E2),

            // Transition state if the reload failed.
            Err((app_state, error)) => {
                // Log debug info.
                tracing::error!("{error:?}");

                // Update stored error (for better health diagnostics).
                Err(Either::E1(app_state.transition_failed(error)))
            }
        }
    }
}

pub(in crate::router) async fn init_config(
    State(app_state): State<AppState<f::Misconfigured, b::Stopped<b::NotInitialized>>>,
) -> Result<(), Error> {
    match app_state.do_init_config().await {
        Ok(_new_state) => Ok(()),
        Err(Either::E1(FailState { error, .. })) => Err(errors::bad_configuration(&error)),
        Err(Either::E2(FailState { error, .. })) => Err(errors::restart_failed(&error)),
    }
}
