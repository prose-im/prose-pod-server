// prose-pod-server-api
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use axum::extract::State;

use crate::responders::Error;
use crate::state::prelude::*;
use crate::util::{NoContext as _, debug_panic_or_log_error};

// MARK: - Routes

pub(in crate::router) async fn backend_reload(
    State(app_state): State<AppState<f::Running, b::Running>>,
) -> Result<(), Error> {
    match app_state.do_reload_backend().await {
        Ok(_) => Ok(()),
        Err(err) => Err(err),
    }
}

// MARK: - State transitions

impl AppState<f::Running, b::Running> {
    /// ```txt
    /// AppState<Running, Running>
    /// -------------------------- (Reload backend)
    /// AppState<Running, Running>
    /// ```
    pub(crate) async fn do_reload_backend(self) -> Result<Self, Error> {
        use anyhow::Context as _;

        // Reload Prosody itself.
        {
            let mut prosody = self.backend.prosody.write().await;
            prosody.reload().await
        }
        .context("Could not reload Prosody")
        .inspect_err(|err| debug_panic_or_log_error!("{err:?}"))
        .no_context()?;

        // Reload Prosody modules (not done automatically).
        let main_host = self.frontend.config.server.domain.as_str();
        {
            let mut prosodyctl = self.backend.prosodyctl.write().await;
            // TODO: Impact of runnning every time?
            prosodyctl.module_load_modules_for_host(main_host).await
        }
        .context(format!("Could not load Prosody modules for `{main_host}`"))
        .inspect_err(|err| debug_panic_or_log_error!("{err:?}"))
        .no_context()?;

        Ok(self)
    }
}
