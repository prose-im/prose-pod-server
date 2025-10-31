// prose-pod-server-api
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use axum::Json;
use axum::extract::State;
use serde::Serialize;

use crate::models::{BareJid, CallerInfo};
use crate::responders::Error;
use crate::state::prelude::*;
use crate::util::{NoContext as _, PROSODY_JIDS_ARE_VALID};

pub async fn users_stats(
    State(AppState {
        ref frontend,
        ref backend,
        ..
    }): State<AppState>,
) -> Result<Json<GetUsersStatsResponse>, Error> {
    let domain = &frontend.config.server.domain;

    let mut prosodyctl = backend.prosodyctl.write().await;

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
pub struct GetUsersStatsResponse {
    pub count: usize,
}

pub async fn list_admin_jids(
    State(AppState {
        ref frontend,
        ref backend,
        ..
    }): State<AppState>,
) -> Result<Json<Vec<BareJid>>, Error> {
    use std::str::FromStr as _;

    let domain = &frontend.config.server.domain;

    let mut prosodyctl = backend.prosodyctl.write().await;

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

pub async fn self_user_info(caller_info: CallerInfo) -> Json<CallerInfo> {
    Json(caller_info)
}
