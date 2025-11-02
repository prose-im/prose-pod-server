// prose-pod-server-api
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::sync::Arc;

use axum::extract::State;

use crate::responders::Error;
use crate::state::prelude::*;
use crate::{AppConfig, errors};

// MARK: - Routes

impl<FrontendSubstate> AppState<f::Running<FrontendSubstate>, b::Running>
where
    FrontendSubstate: FrontendRunningState,
{
    pub(in crate::router) async fn frontend_reload_route(
        State(app_state): State<Self>,
    ) -> Result<(), Error> {
        match app_state.try_reload_frontend::<b::Running>() {
            Ok(_) => Ok(()),

            Err((_, error)) => {
                tracing::warn!("{error:?}");
                Err(errors::bad_configuration(&error))
            }
        }
    }
}

// MARK: - State transitions

impl<F, B> AppState<F, B>
where
    B: crate::router::HealthTrait + Send + Sync + 'static + Clone,
{
    /// Try reloading the frontend, but do not transition if an error occurs.
    ///
    /// NOTE: This method does not log errors.
    ///
    /// ```txt
    /// AppState<F, B1>
    /// ----------------------------------
    /// AppState<Running<Operational>, B2>
    /// ```
    pub(crate) fn try_reload_frontend<B2>(
        self,
    ) -> Result<AppState<f::Running<f::Operational>, B2>, (Self, anyhow::Error)>
    where
        B2: From<B>,
        B2: crate::router::HealthTrait + Send + Sync + 'static + Clone,
        AppState<f::Running, B2>: AppStateTrait,
    {
        match AppConfig::from_default_figment() {
            Ok(app_config) => {
                let todo =
                    "Log warn if config changed and needs a restart (e.g. server address/port).";

                let new_state = self.with_transition::<f::Running<f::Operational>, B2>(|state| {
                    state
                        .with_frontend(f::Running {
                            state: Arc::new(f::Operational {}),
                            config: Arc::new(app_config),
                        })
                        .with_backend_transition(From::from)
                });

                Ok(new_state)
            }

            Err(err) => {
                let error = anyhow::Error::new(err).context("Frontend reload failed");
                Err((self, error))
            }
        }
    }
}

impl<FrontendSubstate, B> AppState<f::Running<FrontendSubstate>, B>
where
    FrontendSubstate: FrontendRunningState,
    B: crate::router::HealthTrait + Send + Sync + 'static + Clone,
    AppState<f::Running, B>: AppStateTrait,
    AppState<f::Running<f::WithMisconfiguration>, B>: AppStateTrait,
{
    /// ```txt
    /// AppState<Running<_>, B>
    /// --------------------------------- (Reload frontend)
    /// AppState<Running<Operational>, B>
    /// ```
    pub(crate) fn do_reload_frontend(
        self,
    ) -> Result<
        AppState<f::Running<f::Operational>, B>,
        (
            AppState<f::Running<f::WithMisconfiguration>, B>,
            Arc<anyhow::Error>,
        ),
    > {
        match self.try_reload_frontend() {
            Ok(new_state) => Ok(new_state),

            Err((app_state, error)) => {
                let error = Arc::new(error);

                // Log debug info.
                tracing::warn!("{error:?}");

                let new_state = app_state
                    .with_transition::<f::Running<f::WithMisconfiguration>, B>(|state| {
                        state.with_frontend_transition(|state| f::Running {
                            state: Arc::new(f::WithMisconfiguration {
                                error: Arc::clone(&error),
                            }),
                            config: state.config,
                        })
                    });

                Err((new_state, error))
            }
        }
    }
}
