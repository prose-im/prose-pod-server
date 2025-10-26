// prose-pod-server-api
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

mod app_config;
mod errors;
mod extractors;
mod models;
mod responders;
mod router;
mod secrets_service;
mod secrets_store;
mod startup;
mod state;
mod util;

use std::sync::Arc;

use anyhow::Context as _;
use axum::Router;
use tokio::net::TcpListener;

use crate::state::prelude::*;

pub(crate) use self::app_config::AppConfig;
use self::startup::startup;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let todo = "Migrate to Prosody 13";
    let todo = "Listen to SIGHUP";

    init_tracing();

    let app_config = AppConfig::from_default_figment()?;

    tokio::select! {
        res = main_inner(app_config) => res.inspect_err(|err| tracing::error!("{err:?}")),

        // Listen for graceful shutdown signals.
        () = listen_for_graceful_shutdown() => Ok(()),
    }
}

async fn main_inner(app_config: AppConfig) -> anyhow::Result<()> {
    // Bind to the API address to exit early if not available.
    let address = app_config.server_api.address();
    let listener = TcpListener::bind(address).await?;

    let app_state = AppState::<f::Running, b::Starting<b::NotInitialized>>::new(
        frontend::Running {
            state: Arc::new(f::Operational {}),
            config: Arc::new(app_config),
        },
        backend::Starting {
            state: Arc::new(b::NotInitialized {}),
        },
    );

    // Serve a minimal HTTP API while the startup actions run.
    let mut main_tasks = tokio::task::JoinSet::<anyhow::Result<()>>::new();
    main_tasks.spawn({
        let app_context = app_state.context().clone();
        async move {
            let app: Router = Router::new()
                .fallback_service(app_context.router())
                .layer(axum::middleware::from_fn(router::util::log_request))
                .with_state(app_context);

            tracing::info!("Serving the Prose Pod Server API on {address}…");
            axum::serve(listener, app).await.context("Serve error")
        }
    });
    main_tasks.spawn(async move { startup(app_state).await.context("Startup error") });

    // Wait for both tasks to finish, or abort if one fails.
    let mut main_res: anyhow::Result<()> = Err(anyhow::Error::msg("No task ran."));
    while let Some(join_res) = main_tasks.join_next().await {
        match join_res {
            Ok(ok @ Ok(())) => main_res = ok,
            Ok(Err(task_err)) => {
                main_tasks.abort_all();
                main_res = Err(task_err)
            }
            Err(join_err) => {
                main_tasks.abort_all();
                main_res = Err(anyhow::Error::new(join_err).context("Join error"))
            }
        }
    }

    main_res
}

async fn listen_for_graceful_shutdown() {
    use tokio::signal;
    use tracing::warn;

    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
        warn!("Received SIGTERM.")
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            warn!("Received Ctrl+C.")
        },
        _ = terminate => {
            warn!("Process terminated.")
        },
    }
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::fmt()
        .with_ansi(true)
        .with_env_filter(env_filter)
        .init();
}
