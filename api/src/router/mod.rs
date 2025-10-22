// prose-pod-server-api
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

mod lifecycle;
mod workspace;

use std::sync::Arc;

use axum::extract::State;
use axum::response::Response;
use axum::routing::{get, put};
use axum::{Json, Router};
use prosodyctl::UserCreateError;
use serde::{Deserialize, Serialize};

use crate::models::{BareJid, CallerInfo, JidNode, Password};
use crate::responders::Error;
use crate::state::{Layer0AppState, Layer2AppState};
use crate::util::{NoContext as _, PROSODY_JIDS_ARE_VALID};
use crate::{AppConfig, errors};

/// Base router that’s always active. Routes defined here will always be available.
pub fn base_router() -> Router<Layer0AppState> {
    Router::new().route("/health", get(health))
    // TODO: /version
}

/// Note that this should report the health of the Server API itself, not
/// the underlying XMPP server. The reason for it is that the Server API
/// can be in a fail state because it’s misconfigured, but keeping the
/// correctly-configured Prosody running in the background to keep a high
/// XMPP server availability (i.e. reduce XMPP downtime).
async fn health(State(app_state): State<Layer0AppState>) -> Response {
    use axum::response::IntoResponse;
    app_state.status().into_response()
}

pub fn startup_router() -> Router {
    Router::new()
}

pub fn main_router() -> Router<Layer2AppState> {
    Router::new()
        .route("/init/first-account", put(init_first_account))
        .route("/users-util/stats", get(users_stats))
        .route("/users-util/admin-jids", get(list_admin_jids))
        .route("/users-util/self", get(self_user_info))
        .route("/invitations-util/stats", get(invitations_stats))
        .merge(lifecycle::router())
        .merge(workspace::router())
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
    State(ref app_state): State<Layer2AppState>,
    State(ref app_config): State<Arc<AppConfig>>,
    Json(dto): Json<CreateAccountRequest>,
) -> Result<Json<CreateAccountResponse>, Error> {
    let mut prosodyctl = app_state.prosodyctl.write().await;

    let first_account_role = "prosody:admin";

    // Ensure no user already exists.
    // FIX: While it shouldn’t be possible to delete the last admin
    //   (see [prose-im/prose-pod-api#344](https://github.com/prose-im/prose-pod-api/issues/344)),
    //   we can re-enable this route whenever no admin exists,
    //   for convenience. I (@RemiBardon) feel like it’s going to
    //   save us from a bad situation one day and that day I’ll
    //   thank myself for taking this decision.
    let user_count = prosodyctl
        .user_get_jids_with_role(&app_config.server.domain, first_account_role)
        .await
        .no_context()?
        .len();
    if user_count > 0 {
        return Err(crate::errors::conflict_error(
            "FIRST_ACCOUNT_ALREADY_CREATED",
            "First account already created",
            "You now need an invitation to join.",
        ));
    }

    // Create first admin account.
    let jid = BareJid::from_parts(Some(&dto.username), &app_config.server.domain);
    let result = prosodyctl
        .user_create(jid.as_str(), &dto.password, Some(first_account_role))
        .await;

    // Release lock ASAP.
    drop(prosodyctl);

    match result {
        Ok(summary) => {
            tracing::info!("{summary}");
            let response = CreateAccountResponse {
                username: dto.username,
                role: first_account_role.to_owned(),
            };
            Ok(Json(response))
        }
        Err(UserCreateError::Conflict) => {
            // // NOTE: Even more so since we lock the prosodyctl shell between
            // //   listing users and creating the first admin account.
            // unreachable!("There shouldn’t be any user")
            // NOTE: Because we now check for admins only,
            //   there might still be a conflict.
            Err(errors::conflict_error(
                "USERNAME_ALREADY_TAKEN",
                "Username already taken",
                "Choose another username.",
            ))
        }
        Err(UserCreateError::Internal(error)) => Err(error.no_context()),
    }
}

async fn users_stats(
    State(ref app_state): State<Layer2AppState>,
    State(ref app_config): State<Arc<AppConfig>>,
) -> Result<Json<GetUsersStatsResponse>, Error> {
    let domain = &app_config.server.domain;

    let mut prosodyctl = app_state.prosodyctl.write().await;

    // NOTE: We need to filter out service accounts,
    //   which don’t have the `prosody:member` role.
    let user_count = prosodyctl
        .user_get_jids_with_role(domain, "prosody:member")
        .await
        .no_context()?
        .len();

    // Release lock ASAP.
    drop(prosodyctl);

    Ok(Json(GetUsersStatsResponse { count: user_count }))
}

#[derive(Serialize)]
struct GetUsersStatsResponse {
    pub count: usize,
}

async fn list_admin_jids(
    State(ref app_state): State<Layer2AppState>,
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
    State(ref app_state): State<Layer2AppState>,
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

// MARK: - Utilities

pub(crate) mod util {
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
}
