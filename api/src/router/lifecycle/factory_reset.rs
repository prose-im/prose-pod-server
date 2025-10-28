// prose-pod-server-api
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;

use crate::errors::ERROR_CODE_INTERNAL;
use crate::responders::Error;
use crate::state::prelude::*;
use crate::util::{NoContext as _, empty_dir};
use crate::{AppConfig, startup};

/// **Undergoing factory reset** (during a factory reset).
impl AppStateTrait for AppState<f::UndergoingFactoryReset, b::UndergoingFactoryReset> {
    fn state_name() -> &'static str {
        "Undergoing factory reset"
    }

    fn into_router(self) -> axum::Router {
        Router::new()
    }
}

pub async fn factory_reset(
    State(app_state): State<AppState<f::Running, b::Running>>,
) -> Result<StatusCode, Error> {
    self::factory_reset_(app_state).await
}

async fn factory_reset_<F, B>(app_state: AppState<F, B>) -> Result<StatusCode, Error>
where
    B: Clone + AsRef<b::Operational>,
    f::UndergoingFactoryReset: From<F>,
    b::UndergoingFactoryReset: From<B>,
{
    use tokio::time::Instant;

    let start = Instant::now();

    let substate = app_state.backend.clone();
    let mut prosody = substate.as_ref().prosody.write().await;
    let mut prosodyctl = substate.as_ref().prosodyctl.write().await;

    // Read Prosody paths early to abort before doing anything non-recoverable.
    let config_path = prosodyctl.prosody_paths_config().await.no_context()?;
    let data_path = prosodyctl.prosody_paths_data().await.no_context()?;

    let app_state =
        app_state.with_auto_transition::<f::UndergoingFactoryReset, b::UndergoingFactoryReset>();

    prosody.stop().await.no_context()?;

    tracing::warn!("Emptying `{config_path}`…");
    empty_dir(&config_path)
        .map_err(|err| anyhow::Error::new(err).context(format!("Emptying `{config_path}`")))
        .no_context()?;
    tracing::warn!("Emptying `{data_path}`…");
    empty_dir(&data_path)
        .map_err(|err| anyhow::Error::new(err).context(format!("Emptying `{data_path}`")))
        .no_context()?;

    reset_config_file().await?;

    match AppConfig::from_default_figment() {
        Ok(app_config) => {
            // NOTE: After a factory reset, the default configuration is,
            //   at least, missing the Server domain. However, in some cases
            //   like for Prose Cloud instances, the required values will be
            //   set via environment variables. This allows seamless factory
            //   resets, without requireing one to edit the configuration
            //   file manually.

            // Transition app to “Starting”.
            let app_state = app_state.with_transition(|state| {
                state
                    .with_frontend(f::Running {
                        state: Arc::new(f::Operational {}),
                        config: Arc::new(app_config),
                    })
                    .with_backend(b::Starting {
                        state: Arc::new(b::NotInitialized {}),
                    })
            });

            // Try to bootstrap the backend.
            _ = startup::bootstrap(app_state)
                .await
                .map_err(|err| err.context("Initialization failed after factory reset"))
                .no_context()?;
        }
        Err(error) => {
            let error = Arc::new(
                anyhow::Error::new(error).context("Factory reset done, configuration needed"),
            );

            app_state.transition_with(|state| {
                state
                    .with_frontend(f::Misconfigured {
                        error: Arc::clone(&error),
                    })
                    .with_backend(b::Stopped {
                        state: Arc::new(b::NotInitialized {}),
                    })
            });

            // Log debug info.
            tracing::warn!("{error:?}");

            // NOTE: Do not return a failure status code
            //   as this is expected behavior.
        }
    };

    tracing::info!("Performed factory reset in {:.0?}.", start.elapsed());

    Ok(StatusCode::RESET_CONTENT)
}

async fn reset_config_file() -> Result<(), Error> {
    use crate::app_config::CONFIG_FILE_PATH;
    use crate::util::Context as _;
    use std::fs;
    use std::io::Write as _;

    tracing::info!("Resetting the Prose configuration file…");

    let config_file_path = CONFIG_FILE_PATH.as_path();
    let mut file = fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(config_file_path)
        .context(
            ERROR_CODE_INTERNAL,
            &format!(
                "Could not reset API config file at <{path}>: Cannot open",
                path = config_file_path.display(),
            ),
        )?;
    let bootstrap_config = r#"# Prose Pod configuration file
# Template: https://github.com/prose-im/prose-pod-system/blob/master/templates/prose.toml
# All keys: https://github.com/prose-im/prose-pod-api/blob/master/src/service/src/features/app_config.rs
"#;
    file.write_all(bootstrap_config.as_bytes()).context(
        ERROR_CODE_INTERNAL,
        &format!(
            "Could not reset API config file at <{path}>: Cannot write",
            path = config_file_path.display(),
        ),
    )?;

    Ok(())
}
