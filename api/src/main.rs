// prose-pod-server-api
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

mod config;
mod models;
mod router;
mod startup;
mod util;

use std::{
    net::{Ipv4Addr, SocketAddrV4},
    str::FromStr,
};

use anyhow::anyhow;
use axum::Router;
use tokio::net::TcpListener;

use crate::{config::AppConfig, router::startup_router, startup::startup};

use self::router::router;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let app_config = {
        use crate::config::*;
        use crate::models::jid::*;

        AppConfig {
            server: ServerConfig {
                domain: JidDomain::from_str("example.org").unwrap(),
            },
            service_accounts: Default::default(),
            teams: Default::default(),
        }
    };

    main_inner(&app_config)
        .await
        .inspect_err(|err| tracing::error!("{err:#}"))
}

async fn main_inner(app_config: &AppConfig) -> anyhow::Result<()> {
    // Bind to the API address to exit early if not available.
    let address = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 8080);
    let mut listener = TcpListener::bind(address).await?;

    // Run startup tasks.
    let startup_res = tokio::select! {
        startup_res = startup(app_config) => match startup_res {
            Ok(prosody_handle) => Ok(Some(prosody_handle)),
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
    let Some(prosody_handle) = startup_res else {
        return Ok(());
    };

    // Then serve all routes once Prosody has started.
    listener = TcpListener::bind(address).await?;
    tokio::select! {
        res = {
            tracing::info!("Now serving all routes on {address}…");
            serve(listener, router())
        } => res,

        // Keep Prosody running in the back.
        join_res = prosody_handle => match join_res {
            Ok(res) => res,
            Err(join_err) => Err(anyhow!(join_err)),
        },

        // Listen for graceful shutdown signals.
        () = listen_for_graceful_shutdown() => Ok(()),
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
