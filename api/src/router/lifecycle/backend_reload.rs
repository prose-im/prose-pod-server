// prose-pod-server-api
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::sync::Arc;

use axum::extract::State;

use crate::responders::Error;
use crate::state::prelude::*;
use crate::util::{NoContext as _, debug_panic_or_log_error};

// MARK: - Routes

pub(in crate::router) async fn backend_reload(
    State(app_state): State<AppState<f::Running, b::Running>>,
) -> Result<(), Error> {
    match app_state.do_reload_backend().await {
        Ok(_new_state) => Ok(()),
        Err(FailState { error, .. }) => Err(error.no_context()),
    }
}

// MARK: - State transitions

impl AppState<f::Running, b::Running> {
    /// ```txt
    /// AppState<Running, Running>
    /// -------------------------- (Reload backend)
    /// AppState<Running, Running>
    /// ```
    ///
    /// NOTE: This method **does** log errors.
    pub(crate) async fn do_reload_backend(
        self,
    ) -> Result<AppState<f::Running, b::Running>, FailState<f::Running, b::Running>> {
        // Reload Prosody itself.
        let mut prosody = self.backend.prosody.write().await;
        match prosody.reload().await {
            Ok(()) => drop(prosody),
            Err(error) => {
                drop(prosody);

                let error = error.context("Could not reload Prosody");

                debug_panic_or_log_error!("{error:?}");

                return Err(self.with_error(Arc::new(error)));
            }
        }

        // Reload Prosody modules (not done automatically).
        let main_host = self.frontend.config.server.domain.as_str();
        let mut prosodyctl = self.backend.prosodyctl.write().await;
        // TODO: Impact of runnning every time?
        match prosodyctl.module_load_modules_for_host(main_host).await {
            Ok(()) => drop(prosodyctl),
            Err(error) => {
                drop(prosodyctl);

                let error =
                    error.context(format!("Could not load Prosody modules for `{main_host}`"));

                debug_panic_or_log_error!("{error:?}");

                return Err(self.with_error(Arc::new(error)));
            }
        }

        Ok(self)
    }
}
