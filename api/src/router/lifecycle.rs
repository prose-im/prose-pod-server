// prose-pod-server-api
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;

use crate::errors::ERROR_CODE_INTERNAL;
use crate::responders::Error;
use crate::router::{main_router, startup_router};
use crate::startup::bootstrap;
use crate::state::{AppStatus, Layer0AppState, Layer1AppState};
use crate::util::{NoContext as _, ResultPanic as _, debug_panic_or_log_error, empty_dir};
use crate::{AppConfig, Layer2AppState, errors, startup};

pub fn router() -> Router<Layer2AppState> {
    Router::new()
        .route("/lifecycle/reload", post(reload))
        .route("/lifecycle/factory-reset", post(factory_reset))
        .route("/prosody/reload", post(prosody_reload))
        .route("/prosody/restart", post(prosody_restart))
}

fn app_misconfigured_router() -> Router<Layer1AppState> {
    Router::new()
        .route("/lifecycle/reload", post(reload_while_misconfigured))
        .fallback(crate::app_status_if_matching!(AppStatus::Misconfigured(_)))
}

fn restart_failed_router() -> Router<Layer2AppState> {
    Router::new()
        .route("/prosody/restart", post(prosody_restart))
        .fallback(crate::app_status_if_matching!(AppStatus::RestartFailed(_)))
}

pub fn factory_reset_router() -> Router {
    Router::new()
}

async fn factory_reset(
    State(layer1_app_state): State<Layer1AppState>,
) -> Result<StatusCode, Error> {
    use crate::state::AppStatus;
    use tokio::time::Instant;

    let start = Instant::now();

    let mut prosody = layer1_app_state.prosody.write().await;
    let mut prosodyctl = layer1_app_state.prosodyctl.write().await;

    // Read Prosody paths early to abort before doing anything non-recoverable.
    let config_path = prosodyctl.prosody_paths_config().await.no_context()?;
    let data_path = prosodyctl.prosody_paths_data().await.no_context()?;

    let layer0_app_state = layer1_app_state.layer0.clone();
    layer0_app_state.set_state(AppStatus::UndergoingFactoryReset, factory_reset_router());

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

    // Initialize Prosody.
    // NOTE: Previous `layer1_app_state` will only be dropped when the function
    //   ends. That’s desired. We want to keep `prosody` and `prosodyctl`
    //   exclusive locks open as long as possible to avoid race conditions.
    let layer1_app_state = startup::init(layer0_app_state.clone())
        .await
        .map_err(|err| err.context("Initialization failed after factory reset"))
        .no_context()?;

    match AppConfig::from_default_figment() {
        Ok(_app_config) => {
            let todo = "That’s not quite right, it can be defined using \
                an environment variable — which would make it survive factory resets.";
            unreachable!(
                "After a factory reset, the default configuration is, at least, missing the Server domain."
            )
        }
        Err(err) => {
            drop(prosody);

            // Start Prosody to allow interacting with `prosodyctl`.
            // NOTE: Not doing it before reading the app config, as in the
            //   non-error case we run the bootstrapping phase which starts
            //   Prosody.
            {
                let mut prosody = layer1_app_state.prosody.write().await;
                prosody.start().await.no_context()?;
            }

            // Log debug info.
            let err = anyhow::Error::new(err);
            tracing::warn!("{err:?}");

            // NOTE: Set status as “Misconfigured” _after_ starting Prosody as
            //   Prosody failing to start would indicate we’re stuck in the
            //   “UndergoingFactoryReset” phase.
            layer0_app_state.set_state(
                AppStatus::Misconfigured(err),
                app_misconfigured_router().with_state(layer1_app_state),
            );

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
            format!(
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
        format!(
            "Could not reset API config file at <{path}>: Cannot write",
            path = config_file_path.display(),
        ),
    )?;

    Ok(())
}

fn read_app_config_(app_state: Layer1AppState) -> Result<(AppConfig, Layer1AppState), Error> {
    match AppConfig::from_default_figment() {
        Ok(app_config) => Ok((app_config, app_state)),
        Err(err) => {
            // Log debug info.
            let err = anyhow::Error::new(err);
            tracing::warn!("{err:?}");

            let res = errors::bad_configuration(&err);

            let layer0_app_state = app_state.layer0.clone();
            // NOTE: Set status as “Misconfigured” _after_ starting Prosody as
            //   Prosody failing to start would indicate we’re stuck in the
            //   “UndergoingFactoryReset” phase.
            layer0_app_state.set_state(
                AppStatus::Misconfigured(err),
                app_misconfigured_router().with_state(app_state),
            );

            Err(res)
        }
    }
}

async fn reload_while_misconfigured(State(app_state): State<Layer1AppState>) -> Result<(), Error> {
    let layer0_app_state = app_state.layer0.clone();

    let (app_config, app_state) = read_app_config_(app_state)?;

    let todo = "Check app_state.is_server_bootstrapping_done";
    let layer2_app_state = bootstrap(app_config, app_state).await.no_context()?;

    layer0_app_state.set_state(
        AppStatus::Running,
        main_router().with_state(layer2_app_state),
    );

    Ok(())
}

async fn reload(State(app_state): State<Layer2AppState>) -> Result<(), Error> {
    let todo = "Log error if config changed and needs a restart (e.g. server address/port).";

    let layer0_app_state = app_state.layer0.clone();

    let (app_config, layer1_app_state) = read_app_config_(app_state.layer1.clone())?;

    let layer2_app_state = Layer2AppState {
        layer1: layer1_app_state,
        config: Arc::new(app_config),
        ..app_state
    };

    layer0_app_state.set_state(
        AppStatus::Running,
        main_router().with_state(layer2_app_state),
    );

    Ok(())
}

async fn prosody_reload(State(ref app_state): State<Layer2AppState>) -> Result<(), Error> {
    use anyhow::Context as _;

    // Reload Prosody itself.
    {
        let mut prosody = app_state.prosody.write().await;
        prosody.reload().await
    }
    .context("Could not reload Prosody")
    .debug_panic_or_log_error()
    .no_context()?;

    // Reload Prosody modules (not done automatically).
    let main_host = app_state.config.server.domain.as_str();
    {
        let mut prosodyctl = app_state.prosodyctl.write().await;
        // TODO: Impact of runnning every time?
        prosodyctl.module_load_modules_for_host(main_host).await
    }
    .context(format!("Could not load Prosody modules for `{main_host}`"))
    .debug_panic_or_log_error()
    .no_context()?;

    Ok(())
}

async fn prosody_restart(State(app_state): State<Layer2AppState>) -> StatusCode {
    let layer0_app_state = app_state.layer0.clone();

    app_state.set_state(AppStatus::Restarting, startup_router());

    let restart_res = {
        let mut prosody = app_state.prosody.write().await;
        prosody.restart().await
    };

    match restart_res {
        Ok(()) => {
            layer0_app_state.set_state(AppStatus::Running, main_router().with_state(app_state));

            StatusCode::OK
        }
        Err(err) => {
            let err = err.context("Could not restart Prosody");

            // Log debug info.
            debug_panic_or_log_error(format!("{err:?}"));

            layer0_app_state.set_state(
                AppStatus::RestartFailed(err),
                restart_failed_router().with_state(app_state),
            );

            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}
