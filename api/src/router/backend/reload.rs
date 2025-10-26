// prose-pod-server-api
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use axum::extract::State;

use crate::responders::Error;
use crate::state::prelude::*;
use crate::util::{NoContext as _, ResultPanic as _};

pub async fn backend_reload(
    State(AppState {
        ref frontend,
        ref backend,
        ..
    }): State<AppState<f::Running, b::Running>>,
) -> Result<(), Error> {
    use anyhow::Context as _;
    tracing::debug!("AAAAAA");

    // Reload Prosody itself.
    {
        let mut prosody = backend.prosody.write().await;
        prosody.reload().await
    }
    .context("Could not reload Prosody")
    .debug_panic_or_log_error()
    .no_context()?;

    // Reload Prosody modules (not done automatically).
    let main_host = frontend.config.server.domain.as_str();
    {
        let mut prosodyctl = backend.prosodyctl.write().await;
        // TODO: Impact of runnning every time?
        prosodyctl.module_load_modules_for_host(main_host).await
    }
    .context(format!("Could not load Prosody modules for `{main_host}`"))
    .debug_panic_or_log_error()
    .no_context()?;

    Ok(())
}
