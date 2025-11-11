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

use std::sync::{Arc, atomic::AtomicBool};

use anyhow::{Context as _, anyhow};
use axum::Router;
use tokio::{net::TcpListener, time::Instant};

use crate::{
    state::prelude::*,
    util::tracing_subscriber_ext::{TracingReloadHandles, init_tracing},
};

pub(crate) use self::app_config::AppConfig;

static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // TODO: Allow logging at startup for debugging purposes.
    let app_config = AppConfig::from_default_figment()?;

    // Initialize tracing subscribers.
    let (otel_tracer, tracing_reload_handles) = {
        init_tracing(&app_config.log, &app_config.server.log_level)
            .unwrap_or_else(|err| panic!("Failed to init tracing for OpenTelemetry: {err}"))
    };

    let app_context = Arc::new(AppContext::new());

    let reload_callback = {
        let app_context = Arc::clone(&app_context);
        move || app_context.reload()
    };

    let res = tokio::select! {
        res = main_inner(Arc::clone(&app_context), app_config, tracing_reload_handles) => res,

        // Listen for graceful shutdown signals.
        () = listen_for_graceful_shutdown() => Ok(()),

        () = listen_for_reload(reload_callback) => Err(anyhow!("Reload watcher ended unexpectedly.")),
    };

    SHUTTING_DOWN.store(true, std::sync::atomic::Ordering::Relaxed);

    // Wait for Prosody to shutdown gracefully.
    app_context.cleanup().await?;
    // Drop OTel provider to ensure all remaining spans are exported.
    drop(otel_tracer);

    drop(app_context);

    res
}

async fn main_inner(
    app_context: Arc<AppContext>,
    app_config: AppConfig,
    tracing_reload_handles: TracingReloadHandles,
) -> anyhow::Result<()> {
    // Bind to the API address to exit early if not available.
    let address = app_config.server_api.address();
    let listener = TcpListener::bind(address).await?;

    let startup_app_state = AppState::<f::Running, b::Starting<b::NotInitialized>>::new(
        Arc::clone(&app_context),
        frontend::Running {
            state: Arc::new(f::Operational {}),
            config: Arc::new(app_config),
            tracing_reload_handles: Arc::new(tracing_reload_handles),
        },
        backend::Starting {
            state: Arc::new(b::NotInitialized {}),
        },
    );

    // Serve a minimal HTTP API while the startup actions run.
    let mut main_tasks = tokio::task::JoinSet::<anyhow::Result<()>>::new();
    main_tasks.spawn({
        // NOTE: Looks like `Router::with_state` stores clones of the app state
        //   for each route, but nothing keeps a strong reference to it if the
        //   router is empty. Our main router has no route, which means the app
        //   context gets dropped immediately! While we might add static routes
        //   (e.g. `/version`) in the future, it’s better not to rely on such
        //   implementation details and pass a strong reference to the app
        //   context to `main_inner` (ensuring it will never get droppped early)
        //   Doing this also gives us a reference to the app context in `main`
        //   to do some cleanup tasks during graceful shutdowns if we ever need
        //   to, which is good.
        let app_context = Arc::clone(&app_context);

        async move {
            let app: Router = Router::new()
                .fallback_service(app_context.router())
                .layer(axum::middleware::from_fn(router::util::log_request))
                .with_state(app_context);

            tracing::info!("Serving the Prose Pod Server API on {address}…");
            axum::serve(listener, app).await.context("Serve error")
        }
    });
    main_tasks.spawn(async move { startup(startup_app_state).await.context("Startup error") });

    // Wait for both tasks to finish, or abort if one fails.
    let mut main_res: Option<anyhow::Result<()>> = None;
    while let Some(join_res) = main_tasks.join_next().await {
        match join_res {
            Ok(ok @ Ok(())) => main_res = Some(ok),
            Ok(Err(task_err)) => {
                main_tasks.abort_all();
                if main_res.is_none() {
                    main_res = Some(Err(task_err));
                }
            }
            Err(join_err) => {
                main_tasks.abort_all();
                if main_res.is_none() {
                    main_res = Some(Err(anyhow::Error::new(join_err).context("Join error")));
                }
            }
        }
    }

    main_res.unwrap_or(Err(anyhow::Error::msg("No task ran.")))
}

async fn startup(
    app_state: AppState<f::Running, b::Starting<b::NotInitialized>>,
) -> Result<(), anyhow::Error> {
    tracing::info!("Running startup actions…");
    let start = Instant::now();

    match app_state.try_bootstrapping().await {
        Ok(_new_state) => {
            tracing::info!("Startup took {:.0?}.", start.elapsed());
            Ok(())
        }
        Err((_new_state, error)) => {
            let error = error.context("Startup failed");

            // Log debug info.
            tracing::error!("{error:?}");

            tracing::info!("Startup failed in {:.0?}.", start.elapsed());
            Err(error)
        }
    }
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

pub async fn listen_for_reload(callback: impl Fn() -> ()) {
    use tokio::signal;
    use tracing::warn;

    #[cfg(unix)]
    let mut sighup = signal::unix::signal(signal::unix::SignalKind::hangup())
        .expect("failed to install SIGHUP handler");

    async fn listen_for_one_reload(#[cfg(unix)] sighup: &mut signal::unix::Signal) {
        #[cfg(unix)]
        let sighup = async {
            sighup.recv().await;
        };

        #[cfg(not(unix))]
        let sighup = std::future::pending::<()>();

        tokio::select! {
            _ = sighup => {
                warn!("Received SIGHUP.")
            },
        }
    }

    loop {
        listen_for_one_reload(
            #[cfg(unix)]
            &mut sighup,
        )
        .await;
        callback()
    }
}
