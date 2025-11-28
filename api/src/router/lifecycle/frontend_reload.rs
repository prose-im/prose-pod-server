// prose-pod-server
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::sync::Arc;

use anyhow::Context as _;
use axum::extract::State;

use crate::responders::Error;
use crate::state::prelude::*;
use crate::util::tracing_subscriber_ext::update_tracing_config;
use crate::{AppConfig, errors};

// MARK: - Routes

pub(in crate::router) async fn frontend_reload(
    State(app_state): State<AppState<f::Running, b::Running>>,
) -> Result<(), Error> {
    match app_state.do_reload_frontend::<f::RunningWithMisconfiguration, b::Running, b::Running>() {
        Ok(_new_state) => Ok(()),
        Err(FailState { error, .. }) => Err(error),
    }
}

// MARK: - State transitions

impl<F: frontend::State, B: backend::State> AppState<F, B>
where
    AppState<F, B>: AppStateTrait,
{
    /// NOTE: This method does **not** log errors.
    fn reload_frontend(app_state: &Self) -> Result<f::Running, anyhow::Error> {
        let app_config = AppConfig::from_default_figment()?;

        app_state.validate_config_changes(&app_config)?;

        let tracing_reload_handles = app_state.frontend.tracing_reload_handles();
        update_tracing_config(
            &app_config.log,
            &app_config.server.log_level,
            &tracing_reload_handles,
        )
        .context("Could not update tracing config")?;

        Ok(f::Running {
            config: Arc::new(app_config),
            tracing_reload_handles: Arc::clone(app_state.frontend.tracing_reload_handles()),
        })
    }

    /// Try reloading the frontend, but do not transition if an error occurs.
    ///
    /// ```txt
    /// AppState<F, B1>
    /// --------------------------------- (Try reloading frontend)
    /// AppState<Running, B2>  if success
    /// AppState<F, B1>        if failure
    /// ```
    ///
    /// NOTE: This method does **not** log errors.
    pub(crate) fn try_reload_frontend<B2>(
        self,
    ) -> Result<AppState<f::Running, B2>, (Self, anyhow::Error)>
    where
        B: Into<B2>,
        B2: backend::State,
        AppState<f::Running, B2>: AppStateTrait,
    {
        match Self::reload_frontend(&self) {
            Ok(frontend) => Ok(self.with_frontend(frontend).with_auto_transition()),

            Err(err) => {
                let error = err.context("Frontend reload failed");

                Err((self, error))
            }
        }
    }
}

impl<F: frontend::State, B: backend::State> AppState<F, B>
where
    AppState<F, B>: AppStateTrait,
{
    /// ```txt
    /// AppState<F, B>
    /// ---------------------- (Reload frontend)
    /// AppState<Running, B>  if success
    /// AppState<F, B>        if failure
    /// ```
    ///
    /// NOTE: This method **does** log errors.
    pub(crate) fn do_reload_frontend<FrontendFailure, BackendFailure, BackendSuccess>(
        self,
    ) -> Result<AppState<f::Running, BackendSuccess>, FailState<FrontendFailure, BackendFailure>>
    where
        BackendSuccess: backend::State,
        FrontendFailure: frontend::State,
        BackendFailure: backend::State,
        B: Into<BackendSuccess>,
        for<'a> (F, &'a crate::responders::Error): Into<FrontendFailure>,
        for<'a> (B, &'a crate::responders::Error): Into<BackendFailure>,
        AppState<f::Running, BackendSuccess>: AppStateTrait,
        AppState<FrontendFailure, BackendFailure>: AppStateTrait,
    {
        match self.try_reload_frontend() {
            Ok(new_state) => Ok(new_state.with_auto_transition()),

            Err((app_state, error)) => {
                // Log debug info.
                tracing::error!("{error:?}");

                Err(app_state.transition_failed(errors::service_unavailable_err(
                    &error,
                    "BAD_CONFIGURATION",
                    "Bad configuration",
                    "Your Prose Server configuration is incorrect. \
                    Contact an administrator to fix this.",
                )))
            }
        }
    }
}
