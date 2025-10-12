// prose-pod-server-api
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

mod config;
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

use std::net::{Ipv4Addr, SocketAddrV4};

use anyhow::anyhow;
use axum::Router;
use tokio::net::TcpListener;

pub(crate) use self::config::AppConfig;
use self::router::{router, startup_router};
use self::startup::startup;
pub(crate) use self::state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let todo = "Read app config file";

    let app_config = {
        use crate::config::*;
        use crate::models::jid::*;
        use std::str::FromStr as _;
        use tokio::time::Duration;

        AppConfig {
            server: ServerConfig {
                domain: JidDomain::from_str("example.org").unwrap(),
                local_hostname: "localhost".to_owned(),
                http_port: 5280,
            },
            auth: AuthConfig {
                token_ttl: Duration::from_secs(3 * 3600),
            },
            service_accounts: Default::default(),
            teams: Default::default(),
        }
    };

    main_inner(app_config)
        .await
        .inspect_err(|err| tracing::error!("{err:#}"))
}

async fn main_inner(app_config: AppConfig) -> anyhow::Result<()> {
    // Bind to the API address to exit early if not available.
    let address = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 8080);
    let mut listener = TcpListener::bind(address).await?;

    // Run startup tasks.
    let startup_res = tokio::select! {
        startup_res = startup(app_config) => match startup_res {
            Ok(res) => Ok(Some(res)),
            Err(err) => Err(err),
        },

        // Serve a subset of routes during startup.
        res = {
            tracing::info!("Serving startup routes on {address}…");
            serve(listener, startup_router())
        } => match res {
            Ok(()) => Err(anyhow!("Startup router ended too soon.")),
            Err(err) => Err(err),
        },

        // Listen for graceful shutdown signals.
        () = listen_for_graceful_shutdown() => Ok(None),
    }?;

    // Stop now if we should shutdown gracefully.
    let Some(app_state) = startup_res else {
        return Ok(());
    };

    let secrets_service = app_state.secrets_service.clone();

    // Then serve all routes once Prosody has started.
    listener = TcpListener::bind(address).await?;
    tokio::select! {
        res = {
            tracing::info!("Now serving all routes on {address}…");
            serve(listener, router(app_state))
        } => res,

        // Listen for graceful shutdown signals.
        () = listen_for_graceful_shutdown() => Ok(()),

        // Run cache purge tasks in the background.
        () = secrets_service.run_purge_tasks() => unreachable!(),
    }
}

async fn serve(listener: TcpListener, router: Router) -> anyhow::Result<()> {
    axum::serve(listener, router).await?;
    Ok(())
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

    tracing_subscriber::fmt().with_env_filter(env_filter).init();
}
