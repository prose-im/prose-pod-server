// prose-pod-server-api
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

mod backend;
mod health;
mod init;
mod invitations_util;
mod lifecycle;
mod users_util;
mod workspace;

use axum::Router;
use axum::routing::{MethodRouter, get, post, put};

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

    let todo = "Fix /health comment";
    // TODO: /version
    router
        // Health routes.
        .route("/frontend/health", get(frontend_health_route.clone()))
        .route("/backend/health", get(backend_health_route.clone()))
        // Convenient health route aliases.
        .route("/config-health", get(frontend_health_route))
        // Note that `/health` should report the health of the Server API itself, not
        // the underlying XMPP server. The reason for it is that the Server API
        // can be in a fail state because it’s misconfigured, but keeping the
        // correctly-configured Prosody running in the background to keep a high
        // XMPP server availability (i.e. reduce XMPP downtime).
        .route("/health", get(backend_health_route))
}

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
            .route("/lifecycle/reload", post(Self::frontend_reload_route))
            .route("/lifecycle/factory-reset", post(lifecycle::factory_reset))
            .route("/backend/reload", post(backend::backend_reload))
            .merge(Self::backend_restart_routes())
            .merge(Self::workspace_routes())
            .with_state(self)
    }
}

pub fn startup_router() -> Router {
    Router::new()
}

// MARK: - Utilities

pub(crate) mod util {
    use axum::extract::State;

    use crate::state::AppState;

    pub async fn log_request(
        req: axum::http::Request<axum::body::Body>,
        next: axum::middleware::Next,
    ) -> impl axum::response::IntoResponse {
        let method = req.method().clone();
        let path = req.uri().path().to_string();

        let matched_path = req
            .extensions()
            .get::<axum::extract::MatchedPath>()
            .map(|mp| mp.as_str())
            .unwrap_or(&path);

        match matched_path {
            "/health" => {
                tracing::trace!(method = %method, route = %matched_path, "Incoming request")
            }
            _ => tracing::debug!(method = %method, route = %matched_path, "Incoming request"),
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
