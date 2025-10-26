// prose-pod-server-api
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use axum::Json;
use axum::extract::State;
use serde::Serialize;

use crate::responders::Error;
use crate::state::prelude::*;
use crate::util::NoContext as _;

pub async fn invitations_stats(
    State(AppState {
        ref frontend,
        ref backend,
        ..
    }): State<AppState<f::Running, b::Running>>,
) -> Result<Json<GetInvitationsStatsResponse>, Error> {
    let domain = &frontend.config.server.domain;

    let mut prosodyctl = backend.prosodyctl.write().await;

    let invites = prosodyctl.invite_list(domain).await.no_context()?;

    // Release lock ASAP.
    drop(prosodyctl);

    Ok(Json(GetInvitationsStatsResponse {
        count: invites.len(),
    }))
}

#[derive(Serialize)]
pub struct GetInvitationsStatsResponse {
    pub count: usize,
}
