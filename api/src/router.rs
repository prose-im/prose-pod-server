// prose-pod-server-api
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::{io::Write, path::PathBuf};

use axum::{
    Router,
    http::StatusCode,
    response::Response,
    routing::{get, put},
};

pub fn startup_router() -> Router {
    Router::new()
        .route("/health", get(starting_up))
        .layer(axum::middleware::from_fn(log_request))
}

pub fn router() -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/lifecycle/reload", put(reload))
        .route("/lifecycle/restart", put(restart))
        .layer(axum::middleware::from_fn(log_request))
}

async fn starting_up() -> Response {
    Response::builder()
        .status(StatusCode::SERVICE_UNAVAILABLE)
        // FIXME: Check if 1 second is enough.
        .header("Retry-After", 1)
        .body("Starting and initializing the Server…".into())
        .unwrap()
}

async fn health() -> &'static str {
    "OK"
}

const PROSODY_SIGNALS_DIR: &'static str = "/var/run/prosody/signals";

async fn reload() -> StatusCode {
    use std::fs::File;

    let path = PathBuf::from(format!(
        "{PROSODY_SIGNALS_DIR}/reload-{timestamp}",
        timestamp = unix_timestamp(),
    ));

    let mut file = File::options().create_new(true).open(path).unwrap();
    file.write("prose::orchestrator/graceful".as_bytes())
        .unwrap();

    StatusCode::NOT_IMPLEMENTED
}

async fn restart() -> StatusCode {
    StatusCode::NOT_IMPLEMENTED
}

async fn log_request(
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
        "/health" => tracing::trace!(method = %method, route = %matched_path, "Incoming request"),
        _ => tracing::debug!(method = %method, route = %matched_path, "Incoming request"),
    }

    next.run(req).await
}

// MARK: - Helpers

fn unix_timestamp() -> u64 {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}
