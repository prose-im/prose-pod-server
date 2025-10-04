// prose-pod-server-api
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::{get, put};
use axum::{Json, Router};
use prosodyctl::UserCreateError;
use serde::{Deserialize, Serialize};

use crate::AppConfig;
use crate::errors::{self, ERROR_KIND_CONFLICT};
use crate::models::{BareJid, CallerInfo, JidNode, Password};
use crate::responders::Error;
use crate::state::AppState;
use crate::util::{NoContext, PROSODY_JIDS_ARE_VALID};

pub fn startup_router() -> Router {
    Router::new()
        .route("/health", get(starting_up))
        .layer(axum::middleware::from_fn(log_request))
}

pub fn router(app_state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/lifecycle/reload", put(reload))
        .route("/lifecycle/restart", put(restart))
        .route("/init/first-account", put(init_first_account))
        .route("/users-util/stats", get(users_stats))
        .route("/users-util/admin-jids", get(list_admin_jids))
        .route("/users-util/self", get(self_user_info))
        .route("/invitations-util/stats", get(invitations_stats))
        .route(
            "/service-accounts/{username}/password",
            get(get_service_account_password),
        )
        .layer(axum::middleware::from_fn(log_request))
        .with_state(app_state)
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

async fn reload() -> StatusCode {
    StatusCode::NOT_IMPLEMENTED
}

async fn restart() -> StatusCode {
    StatusCode::NOT_IMPLEMENTED
}

#[derive(Debug, Deserialize)]
pub struct CreateAccountRequest {
    pub username: JidNode,
    pub password: Password,
}

#[derive(Debug, Serialize)]
pub struct CreateAccountResponse {
    pub username: JidNode,
    pub role: String,
}

async fn init_first_account(
    State(ref app_state): State<AppState>,
    State(ref app_config): State<Arc<AppConfig>>,
    Json(dto): Json<CreateAccountRequest>,
) -> Result<Json<CreateAccountResponse>, Error> {
    let mut prosodyctl = app_state.prosodyctl.write().await;

    // Ensure no user already exist.
    let users = prosodyctl
        .user_list(&app_config.server.domain, None)
        .await
        .map_err(NoContext::no_context)?;
    if !users.is_empty() {
        return Err(Error::new(
            ERROR_KIND_CONFLICT,
            "FIRST_ACCOUNT_ALREADY_CREATED",
            StatusCode::CONFLICT,
            "First account already created",
            "Now you need an invitation to join.",
        ));
    }

    // Create first admin account.
    let jid = BareJid::new(&dto.username, &app_config.server.domain);
    let role = "prosody:admin";
    let result = prosodyctl
        .user_create(&jid, &dto.password, Some(role))
        .await;

    // Release lock ASAP.
    drop(prosodyctl);

    match result {
        Ok(summary) => {
            tracing::info!("{summary}");
            let response = CreateAccountResponse {
                username: dto.username,
                role: role.to_owned(),
            };
            Ok(Json(response))
        }
        Err(UserCreateError::Conflict) => {
            // NOTE: Even more so since we lock the prosodyctl shell between
            //   listing users and creating the first admin account.
            unreachable!("There shouldn’t be any user")
        }
        Err(UserCreateError::Internal(error)) => Err(error.no_context()),
    }
}

async fn users_stats(
    State(ref app_state): State<AppState>,
    State(ref app_config): State<Arc<AppConfig>>,
) -> Result<Json<GetUsersStatsResponse>, Error> {
    let domain = &app_config.server.domain;

    let mut prosodyctl = app_state.prosodyctl.write().await;

    let users = prosodyctl.user_list(domain, None).await.no_context()?;

    // Release lock ASAP.
    drop(prosodyctl);

    Ok(Json(GetUsersStatsResponse { count: users.len() }))
}

#[derive(Serialize)]
struct GetUsersStatsResponse {
    pub count: usize,
}

async fn list_admin_jids(
    State(ref app_state): State<AppState>,
    State(ref app_config): State<Arc<AppConfig>>,
) -> Result<Json<Vec<BareJid>>, Error> {
    use std::str::FromStr as _;

    let domain = &app_config.server.domain;

    let mut prosodyctl = app_state.prosodyctl.write().await;

    let jids: Vec<String> = prosodyctl
        .user_get_jids_with_role(domain, "prosody:admin")
        .await
        .no_context()?;

    // Release lock ASAP.
    drop(prosodyctl);

    let jids: Vec<BareJid> = jids
        .iter()
        .map(|str| BareJid::from_str(str).expect(PROSODY_JIDS_ARE_VALID))
        .collect();

    Ok(Json(jids))
}

async fn self_user_info(caller_info: CallerInfo) -> Json<CallerInfo> {
    Json(caller_info)
}

async fn invitations_stats(
    State(ref app_state): State<AppState>,
    State(ref app_config): State<Arc<AppConfig>>,
) -> Result<Json<GetInvitationsStatsResponse>, Error> {
    let domain = &app_config.server.domain;

    let mut prosodyctl = app_state.prosodyctl.write().await;

    let invites = prosodyctl.invite_list(domain).await.no_context()?;

    // Release lock ASAP.
    drop(prosodyctl);

    Ok(Json(GetInvitationsStatsResponse {
        count: invites.len(),
    }))
}

#[derive(Serialize)]
struct GetInvitationsStatsResponse {
    pub count: usize,
}

async fn get_service_account_password(
    State(ref app_state): State<AppState>,
    State(ref app_config): State<Arc<AppConfig>>,
    caller_info: CallerInfo,
    Path(ref username): Path<JidNode>,
) -> Result<Password, Error> {
    // Ensure the caller is an admin.
    // FIXME: Make this more flexible by checking rights instead of roles
    //   (which can be extended).
    match caller_info.primary_role.as_str() {
        "prosody:admin" | "prosody:operator" => {}
        _ => return Err(errors::forbidden("Only admins can do that.")),
    }

    tracing::info!(
        "{jid} requested service account passwords.",
        jid = caller_info.jid,
    );

    let jid = BareJid::new(username, &app_config.server.domain);
    let password = (app_state.service_accounts_credentials)
        .get(&jid)
        .expect(&format!("Service account `{jid}` should exist"))
        .clone();

    Ok(password)
}

// MARK: - Utilities

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
