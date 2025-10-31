// prose-pod-server-api
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::sync::Arc;

use axum::extract::State;

use crate::errors;
use crate::responders::Error;
use crate::state::prelude::*;

impl<FrontendSubstate> AppState<f::Running<FrontendSubstate>, b::Running>
where
    FrontendSubstate: FrontendRunningState,
{
    pub(in crate::router) async fn lifecycle_reload_route(
        State(app_state): State<Self>,
    ) -> Result<(), Error> {
        match app_state.try_reload_frontend() {
            Ok(app_state) => {
                _ = app_state.do_reload_backend().await?;
                Ok(())
            }

            Err((_, error)) => {
                tracing::warn!("{error:?}");
                Err(errors::bad_configuration(&error))
            }
        }
    }
}

impl AppState<f::Misconfigured, b::Stopped<b::NotInitialized>> {
    pub(in crate::router) async fn lifecycle_reload_route(
        State(app_state): State<Self>,
    ) -> Result<(), Error> {
        match app_state.try_reload_frontend::<b::Starting<b::NotInitialized>>() {
            Ok(app_state) => match app_state.try_bootstrapping().await {
                Ok(app_state) => {
                    let fixme = "That shouldn’t be here";
                    _ = app_state.do_reload_backend().await?;
                    Ok(())
                }
                Err(err) => {
                    let todo = "Handle error";
                    panic!("{err:?}")
                }
            },

            // Transition state if the reload failed.
            Err((app_state, error)) => {
                let error = Arc::new(error);
                tracing::warn!("{error:?}");

                // Update stored error (for better health diagnostics).
                app_state.transition_with::<f::Misconfigured, b::Stopped<b::NotInitialized>>(
                    |state| {
                        state.with_frontend(f::Misconfigured {
                            error: Arc::clone(&error),
                        })
                    },
                );

                Err(errors::bad_configuration(&error))
            }
        }
    }
}
