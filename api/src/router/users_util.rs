// prose-pod-server
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use axum::Json;
use axum::extract::State;
use serde::Serialize;

use crate::auth::CallerInfo;
use crate::models::jid::BareJid;
use crate::responders::Error;
use crate::state::prelude::*;
use crate::util::{NoPublicContext as _, PROSODY_JIDS_ARE_VALID};

pub async fn users_stats(
    State(AppState {
        ref frontend,
        ref backend,
        ..
    }): State<AppState>,
) -> Result<Json<GetUsersStatsResponse>, Error> {
    let domain = &frontend.config.server.domain;

    let mut prosodyctl = backend.prosodyctl.write().await;

    // Filter out service accounts.
    // NOTE: Given how roles are attributed at the moment,
    //   `.user_get_jids_with_role(domain, "prosody:member")` doesn’t return
    //   what we want. `prosody:admin` accounts inherit the `prosody:member`
    //   role, but it’s not taken into account as it’s not an explicit
    //   secondary role. As a workaround, we’ll count all `prosody:member`,
    //   `prosody:admin` and `prosody:operator`. It’s a footgun but we’ll
    //   improve that when we rework permissions. We could list all users then
    //   remove the ones which have the `prosody:registered` role, but during
    //   testing we realized it doesn’t work because of a strange bug in
    //   Prosody (see https://gist.github.com/RemiBardon/4a62f940376cf707d66fd1b933ed2e2a).
    //   Until we figure out what’s going wrong there, this will do.
    let mut user_count = 0;
    for role in [
        "prosody:member",
        "prosody:admin",
        "prosody:operator",
    ] {
        user_count += prosodyctl
            .user_get_jids_with_role(domain, role)
            .await
            .no_context()?
            .len();
    }

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
        .no_public_context()?;

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
