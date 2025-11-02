// prose-pod-server-api
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

mod health;
mod init;
mod invitations_util;
mod lifecycle;
mod users_util;
mod workspace;

use axum::Router;
use axum::routing::{get, post, put};

use crate::AppConfig;
use crate::router::util::{backend_health, frontend_health};
use crate::state::prelude::*;

pub(crate) use self::health::HealthTrait;

/// Base router that’s always active. Routes defined here will always be available.
pub fn with_base_routes(
    frontend: impl HealthTrait + Send + Sync + 'static + Clone,
    backend: impl HealthTrait + Send + Sync + 'static + Clone,
    router: Router,
) -> Router {
    let backend_health_route = async move || backend.health();
    let frontend_health_route = async move || frontend.health();

    // TODO: /version
    router
        // Health routes.
        .route("/frontend/health", get(frontend_health_route.clone()))
        .route("/backend/health", get(backend_health_route.clone()))
        // Convenient health route aliases.
        .route("/config-health", get(frontend_health_route))
        // Note that `/health` should report the backend health, not the
        // Server API as a whole. The reason for it is that the frontend
        // can be in a fail state because it became misconfigured after
        // a `SIGHUP`, but is keeping the correctly-configured backend
        // running to keep a high XMPP server availability (i.e. reduce
        // XMPP downtime). If we reported “unhealthy”, the orchestrator
        // could kill the whole Prose Pod Server just because an admin
        // broke the configuration, before they even had the chance to
        // fix it. If needed, `/config-health` can be used by the
        // orchestrator to show a warning on bad configuration change.
        .route("/health", get(backend_health_route))
}

/// **Operational** (under normal conditions).
impl AppStateTrait for AppState<f::Running, b::Running> {
    fn state_name() -> &'static str {
        "Operational"
    }

    fn into_router(self) -> axum::Router {
        Router::new()
            .route("/init/first-account", put(init::init_first_account))
            .route("/users-util/stats", get(users_util::users_stats))
            .route("/users-util/admin-jids", get(users_util::list_admin_jids))
            .route("/users-util/self", get(users_util::self_user_info))
            .route(
                "/invitations-util/stats",
                get(invitations_util::invitations_stats),
            )
            .route(
                "/lifecycle/frontend-reload",
                post(lifecycle::frontend_reload),
            )
            .route("/lifecycle/backend-reload", post(lifecycle::backend_reload))
            .route(
                "/lifecycle/backend-restart",
                post(lifecycle::backend_restart),
            )
            .route("/lifecycle/reload", post(lifecycle::reload))
            .route("/lifecycle/factory-reset", post(lifecycle::factory_reset))
            .merge(workspace::router())
            .with_state(self)
    }

    fn validate_config_changes(&self, new_config: &AppConfig) -> Result<(), anyhow::Error> {
        AppConfig::validate_config_changes(&self.frontend.config, new_config)
    }
}

/// **Starting** (during a startup and after a factory reset).
impl AppStateTrait for AppState<f::Running, b::Starting<b::NotInitialized>> {
    fn state_name() -> &'static str {
        "Starting"
    }

    fn into_router(self) -> axum::Router {
        let todo = "Keep one route for safety?";
        Router::<Self>::new()
            .fallback(backend_health)
            .with_state(self)
    }

    fn validate_config_changes(&self, new_config: &AppConfig) -> Result<(), anyhow::Error> {
        AppConfig::validate_config_changes(&self.frontend.config, new_config)
    }
}

/// **Restarting** (during a restart).
impl AppStateTrait for AppState<f::Running, b::Starting> {
    fn state_name() -> &'static str {
        "Restarting"
    }

    fn into_router(self) -> axum::Router {
        Router::<Self>::new()
            // NOTE: Keep `/lifecycle/backend-restart` available just in case
            //   an internal error happened and we ended up stuck in this state.
            .route("/lifecycle/backend-restart", post(lifecycle::backend_start))
            .fallback(backend_health)
            .with_state(self)
    }

    fn validate_config_changes(&self, new_config: &AppConfig) -> Result<(), anyhow::Error> {
        AppConfig::validate_config_changes(&self.frontend.config, new_config)
    }
}

/// **Restart failed**.
impl AppStateTrait for AppState<f::Running, b::StartFailed<b::Operational>> {
    fn state_name() -> &'static str {
        "Restart failed"
    }

    fn into_router(self) -> axum::Router {
        Router::<Self>::new()
            .route(
                "/lifecycle/backend-restart",
                post(lifecycle::backend_start_retry),
            )
            .fallback(backend_health)
            .with_state(self)
    }

    fn validate_config_changes(&self, new_config: &AppConfig) -> Result<(), anyhow::Error> {
        AppConfig::validate_config_changes(&self.frontend.config, new_config)
    }
}

/// **Running with misconfiguration** (after a `SIGHUP`
/// with a bad configuration).
impl AppStateTrait for AppState<f::Running<f::WithMisconfiguration>, b::Running> {
    fn state_name() -> &'static str {
        "Running with misconfiguration"
    }

    fn into_router(self) -> axum::Router {
        Router::new()
            .route("/lifecycle/reload", post(lifecycle::reload))
            .fallback(frontend_health)
            .with_state(self)
    }

    fn validate_config_changes(&self, new_config: &AppConfig) -> Result<(), anyhow::Error> {
        AppConfig::validate_config_changes(&self.frontend.config, new_config)
    }
}

/// **Undergoing factory reset** (during a factory reset).
impl AppStateTrait for AppState<f::UndergoingFactoryReset, b::UndergoingFactoryReset> {
    fn state_name() -> &'static str {
        "Undergoing factory reset"
    }

    fn into_router(self) -> axum::Router {
        Router::new().fallback(frontend_health).with_state(self)
    }

    fn validate_config_changes(&self, _new_config: &AppConfig) -> Result<(), anyhow::Error> {
        Ok(())
    }
}

/// **Configuration needed** (after a factory reset).
impl AppStateTrait for AppState<f::Misconfigured, b::Stopped<b::NotInitialized>> {
    fn state_name() -> &'static str {
        "Configuration needed"
    }

    fn into_router(self) -> axum::Router {
        Router::new()
            .route("/lifecycle/reload", post(lifecycle::init_config))
            .fallback(frontend_health)
            .with_state(self)
    }

    fn validate_config_changes(&self, _new_config: &AppConfig) -> Result<(), anyhow::Error> {
        Ok(())
    }
}

/// **Bootstrapping failed**.
impl AppStateTrait for AppState<f::Running, b::StartFailed<b::NotInitialized>> {
    fn state_name() -> &'static str {
        "Bootstrapping failed"
    }

    fn into_router(self) -> axum::Router {
        Router::<Self>::new()
            .route(
                "/lifecycle/backend-restart",
                post(lifecycle::backend_init_retry),
            )
            .fallback(backend_health)
            .with_state(self)
    }

    fn validate_config_changes(&self, new_config: &AppConfig) -> Result<(), anyhow::Error> {
        AppConfig::validate_config_changes(&self.frontend.config, new_config)
    }
}

// MARK: - Utilities

pub(crate) mod util {
    use axum::extract::State;

    use crate::state::AppState;

    pub async fn log_request(
        req: axum::http::Request<axum::body::Body>,
        next: axum::middleware::Next,
    ) -> axum::response::Response {
        let method = req.method().clone();
        let path = req.uri().path().to_string();

        let matched_path = req
            .extensions()
            .get::<axum::extract::MatchedPath>()
            .map(|mp| mp.as_str())
            .unwrap_or(&path);

        match matched_path {
            "/health" => {
                tracing::trace!(target: "router", method = %method, route = %matched_path, "Incoming request")
            }
            _ => {
                tracing::debug!(target: "router", method = %method, route = %matched_path, "Incoming request")
            }
        }

        next.run(req).await
    }

    pub async fn frontend_health<F: super::HealthTrait, B>(
        State(AppState { frontend, .. }): State<AppState<F, B>>,
    ) -> axum::response::Response {
        frontend.health()
    }

    pub async fn backend_health<F, B: super::HealthTrait>(
        State(AppState { backend, .. }): State<AppState<F, B>>,
    ) -> axum::response::Response {
        backend.health()
    }
}
