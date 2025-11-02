// prose-pod-server-api
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::sync::Arc;

use anyhow::Context as _;
use axum::extract::State;
use axum::http::StatusCode;
use tokio::time::Instant;

use crate::errors;
use crate::responders::Error;
use crate::state::prelude::*;
use crate::util::either::Either;

// MARK: - Routes

impl AppState<f::Running, b::Running> {
    pub(crate) async fn lifecycle_factory_reset_route(
        State(app_state): State<Self>,
    ) -> Result<StatusCode, Error> {
        match app_state.do_factory_reset().await {
            Ok(_new_state) => Ok(StatusCode::RESET_CONTENT),
            Err((_new_state, error)) => Err(errors::factory_reset_failed(&error)),
        }
    }
}

// MARK: - State transitions

impl<F, B> AppState<F, B> {
    /// NOTE: This method does **not** log errors.
    async fn factory_reset(backend: impl AsRef<b::Operational>) -> Result<(), anyhow::Error> {
        use crate::util::empty_dir;

        let mut prosody = backend.as_ref().prosody.write().await;
        let mut prosodyctl = backend.as_ref().prosodyctl.write().await;

        // Read Prosody paths early to abort before doing anything non-recoverable.
        let config_path = prosodyctl.prosody_paths_config().await?;
        let data_path = prosodyctl.prosody_paths_data().await?;

        prosody.stop().await?;

        tracing::warn!("Emptying `{config_path}`…");
        empty_dir(&config_path).context(format!("Emptying `{config_path}`"))?;

        tracing::warn!("Emptying `{data_path}`…");
        empty_dir(&data_path).context(format!("Emptying `{data_path}`"))?;

        reset_config_file().await?;

        Ok(())
    }

    /// ```txt
    /// AppState<_, _>
    /// -------------------------------------------------------- (Factory reset)
    /// AppState<Misconfigured, Stopped<NotInitialized>>
    ///   if success
    /// AppState<Running, Running>
    ///   if success and minimal config in env
    /// AppState<UndergoingFactoryReset, UndergoingFactoryReset>
    ///   if failure
    /// AppState<f::Running, b::StartFailed<b::NotInitialized>>
    ///   if failure and minimal config in env
    /// ```
    ///
    /// NOTE: This method **does** log errors.
    pub(crate) async fn do_factory_reset(
        self,
    ) -> Result<
        Either<
            AppState<f::Misconfigured, b::Stopped<b::NotInitialized>>,
            AppState<f::Running, b::Running>,
        >,
        (
            Either<
                AppState<f::UndergoingFactoryReset, b::UndergoingFactoryReset>,
                AppState<f::Running, b::StartFailed<b::NotInitialized>>,
            >,
            Arc<anyhow::Error>,
        ),
    >
    where
        B: Clone + AsRef<b::Operational>,
    {
        tracing::info!("Performing factory reset…");
        let start = Instant::now();

        let backend = self.backend.clone();

        let app_state = self.with_transition(|state| {
            state
                .with_frontend(f::UndergoingFactoryReset {})
                .with_backend(b::UndergoingFactoryReset {})
        });

        if let Err(error) = Self::factory_reset(backend).await {
            tracing::error!("Factory reset failed: {error:?}");
            return Err((Either::E1(app_state), Arc::new(error)));
        }

        // Transition app to “Starting”.
        match app_state.try_reload_frontend::<b::Starting<b::NotInitialized>>() {
            Ok(new_state) => {
                // NOTE: After a factory reset, the default configuration is,
                //   at least, missing the Server domain. However, in some cases
                //   like for Prose Cloud instances, the required values will be
                //   set via environment variables. This allows seamless factory
                //   resets, without requiring one to edit the configuration
                //   file manually.

                // Try to bootstrap the backend.
                match new_state.do_bootstrapping().await {
                    Ok(new_state) => Ok(Either::E2(new_state)),

                    Err((new_state, error)) => {
                        tracing::debug!("Bootstrapping failed after factory reset: {error:?}");
                        Err((Either::E2(new_state), error))
                    }
                }
            }
            Err((new_state, error)) => {
                let error = Arc::new(error.context("Factory reset done, configuration needed"));

                // Log debug info.
                tracing::warn!("{error:?}");

                let new_state = new_state.with_transition(|state| {
                    state
                        .with_frontend(f::Misconfigured {
                            error: Arc::clone(&error),
                        })
                        .with_backend(b::Stopped {
                            state: Arc::new(b::NotInitialized {}),
                        })
                });

                tracing::info!("Performed factory reset in {:.0?}.", start.elapsed());
                // NOTE: Do not return a failure as this is expected behavior.
                Ok(Either::E1(new_state))
            }
        }
    }
}

// MARK: - Steps

async fn reset_config_file() -> Result<(), anyhow::Error> {
    use crate::app_config::CONFIG_FILE_PATH;
    use std::fs;
    use std::io::Write as _;

    tracing::warn!("Resetting the Prose configuration file…");

    let config_file_path = CONFIG_FILE_PATH.as_path();

    // Open file in overwrite mode.
    let mut file = fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(config_file_path)
        .context(format!(
            "Could not reset API config file at <{path}>: Cannot open",
            path = config_file_path.display(),
        ))?;

    // Write the placeholder configuration.
    let empty_config = include_str!("../../prose-empty.toml");
    file.write_all(empty_config.as_bytes()).context(format!(
        "Could not reset API config file at <{path}>: Cannot write",
        path = config_file_path.display(),
    ))?;

    Ok(())
}
