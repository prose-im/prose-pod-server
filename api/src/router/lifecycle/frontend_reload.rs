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
    match app_state.do_reload_frontend() {
        Ok(_new_state) => Ok(()),
        Err(FailState { error, .. }) => Err(error),
    }
}

// MARK: - State transitions

impl<F, B> AppState<F, B>
where
    F: frontend::State,
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
        B2: crate::router::HealthTrait + Send + Sync + 'static + Clone,
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

impl<B> AppState<f::Running, B>
where
    AppState<f::Running, B>: AppStateTrait,
{
    /// ```txt
    /// AppState<Running, B>
    /// ---------------------------------------------------- (Reload frontend)
    /// AppState<Running, B>                      if success
    /// AppState<RunningWithMisconfiguration, B>  if failure
    /// ```
    ///
    /// NOTE: This method **does** log errors.
    pub(crate) fn do_reload_frontend(
        self,
    ) -> Result<AppState<f::Running, B>, FailState<f::RunningWithMisconfiguration, B>>
    where
        B: crate::router::HealthTrait + Send + Sync + 'static + Clone,
        for<'a> (B, &'a crate::responders::Error): Into<B>,
        AppState<f::RunningWithMisconfiguration, B>: AppStateTrait,
    {
        match self.try_reload_frontend() {
            Ok(new_state) => Ok(new_state),

            Err((app_state, error)) => {
                // Log debug info.
                tracing::error!("{error:?}");

                Err(app_state.transition_failed(errors::bad_configuration(&error)))
            }
        }
    }
}
