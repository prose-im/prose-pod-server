// prose-pod-server-api
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::sync::{Arc, atomic::AtomicBool};

use arc_swap::ArcSwap;
use axum_hot_swappable_router::HotSwappableRouter;
use prosody_child_process::ProsodyChildProcess;
use prosody_http::mod_http_oauth2::ProsodyOAuth2Client;
use prosody_rest::ProsodyRest;
use prosodyctl::Prosodyctl;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use crate::{AppConfig, secrets_service::SecretsService};

/// The most minimal app state, that’s conserved across restarts and reloads.
#[derive(Debug, Clone)]
pub struct Layer0AppState {
    router: HotSwappableRouter,
    status: Arc<ArcSwap<AppStatus>>,
}

impl Layer0AppState {
    pub(crate) fn new(status: AppStatus, router: axum::Router) -> Self {
        Self {
            router: HotSwappableRouter::new(router),
            status: Arc::new(ArcSwap::from_pointee(status)),
        }
    }

    pub(crate) fn set_state(&self, new_status: AppStatus, new_router: axum::Router) {
        self.router.set(new_router);
        self.status.store(Arc::new(new_status));
    }

    pub(crate) fn router(&self) -> HotSwappableRouter {
        self.router.clone()
    }

    pub(crate) fn status(&self) -> Arc<AppStatus> {
        self.status.load_full()
    }
}

/// Some state that survives reloads, but not restarts.
#[derive(Debug, Clone)]
pub struct Layer1AppState {
    pub layer0: Layer0AppState,
    /// A cancellation token used to cancel tasks on restarts.
    pub restart_bound_cancellation_token: CancellationToken,
    pub prosody: Arc<RwLock<ProsodyChildProcess>>,
    pub prosodyctl: Arc<RwLock<Prosodyctl>>,
    pub is_server_bootstrapping_done: Arc<AtomicBool>,
}

/// Some state that is refreshed on every reload.
#[derive(Debug, Clone)]
pub struct Layer2AppState {
    pub layer1: Layer1AppState,
    pub(crate) config: Arc<AppConfig>,
    /// A cancellation token used to cancel tasks on reloads.
    pub reload_bound_cancellation_token: CancellationToken,
    pub prosody_rest: ProsodyRest,
    pub oauth2_client: Arc<ProsodyOAuth2Client>,
    pub secrets_service: SecretsService,
}

#[derive(Debug)]
pub(crate) enum AppStatus {
    Starting,
    Running,
    Restarting,
    RestartFailed(anyhow::Error),
    Misconfigured(anyhow::Error),
    UndergoingFactoryReset,
}

impl std::fmt::Display for AppStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Starting => write!(f, "Server starting…"),
            Self::Running => write!(f, "Running."),
            Self::Restarting => write!(f, "Server restarting…"),
            Self::RestartFailed(_) => write!(f, "Restart failed."),
            Self::Misconfigured(_) => write!(f, "Incorrect configuration."),
            Self::UndergoingFactoryReset => write!(f, "Factory reset in progress…"),
        }
    }
}

// MARK: - Boilerplate

impl axum::extract::FromRef<Layer2AppState> for Arc<AppConfig> {
    #[inline]
    fn from_ref(app_state: &Layer2AppState) -> Self {
        app_state.config.clone()
    }
}

impl axum::extract::FromRef<Layer2AppState> for Layer1AppState {
    #[inline]
    fn from_ref(app_state: &Layer2AppState) -> Self {
        app_state.layer1.clone()
    }
}

impl axum::extract::FromRef<Layer2AppState> for Layer0AppState {
    #[inline]
    fn from_ref(app_state: &Layer2AppState) -> Self {
        Self::from_ref(&app_state.layer1)
    }
}

impl axum::extract::FromRef<Layer1AppState> for Layer0AppState {
    #[inline]
    fn from_ref(app_state: &Layer1AppState) -> Self {
        app_state.layer0.clone()
    }
}

impl std::ops::Deref for Layer2AppState {
    type Target = Layer1AppState;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.layer1
    }
}

impl std::ops::Deref for Layer1AppState {
    type Target = Layer0AppState;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.layer0
    }
}
