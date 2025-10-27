// prose-pod-server-api
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use axum::extract::State;

use crate::responders::Error;
use crate::state::prelude::*;
use crate::util::{NoContext as _, ResultPanic as _};

impl AppState<f::Running, b::Running> {
    pub(in crate::router) async fn backend_reload_route(
        State(app_state): State<Self>,
    ) -> Result<(), Error> {
        _ = app_state.do_reload_backend().await?;
        Ok(())
    }
}

impl AppState<f::Running, b::Running> {
    #[inline]
    pub async fn do_reload_backend(self) -> Result<Self, Error> {
        use anyhow::Context as _;

        // Reload Prosody itself.
        {
            let mut prosody = self.backend.prosody.write().await;
            prosody.reload().await
        }
        .context("Could not reload Prosody")
        .debug_panic_or_log_error()
        .no_context()?;

        // Reload Prosody modules (not done automatically).
        let main_host = self.frontend.config.server.domain.as_str();
        {
            let mut prosodyctl = self.backend.prosodyctl.write().await;
            // TODO: Impact of runnning every time?
            prosodyctl.module_load_modules_for_host(main_host).await
        }
        .context(format!("Could not load Prosody modules for `{main_host}`"))
        .debug_panic_or_log_error()
        .no_context()?;

        Ok(self)
    }
}
