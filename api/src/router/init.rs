// prose-pod-server-api
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use axum::Json;
use axum::extract::State;
use prosodyctl::UserCreateError;
use serde::{Deserialize, Serialize};

use crate::errors;
use crate::models::{BareJid, JidNode, Password};
use crate::responders::Error;
use crate::state::prelude::*;
use crate::util::NoContext as _;

#[serde_with::serde_as]
#[derive(Debug, Deserialize)]
pub struct CreateAccountRequest {
    #[serde_as(as = "serde_with::DisplayFromStr")]
    pub username: JidNode,
    pub password: Password,
}

#[serde_with::serde_as]
#[derive(Debug, Serialize)]
pub struct CreateAccountResponse {
    #[serde_as(as = "serde_with::DisplayFromStr")]
    pub username: JidNode,
    pub role: String,
}

pub async fn init_first_account(
    State(AppState {
        ref frontend,
        ref backend,
        ..
    }): State<AppState<f::Running, b::Running>>,
    Json(dto): Json<CreateAccountRequest>,
) -> Result<Json<CreateAccountResponse>, Error> {
    let ref server_domain = frontend.config.server.domain;
    let mut prosodyctl = backend.prosodyctl.write().await;

    let first_account_role = "prosody:admin";

    // Ensure no user already exists.
    // FIX: While it shouldn’t be possible to delete the last admin
    //   (see [prose-im/prose-pod-api#344](https://github.com/prose-im/prose-pod-api/issues/344)),
    //   we can re-enable this route whenever no admin exists,
    //   for convenience. I (@RemiBardon) feel like it’s going to
    //   save us from a bad situation one day and that day I’ll
    //   thank myself for taking this decision.
    let user_count = prosodyctl
        .user_get_jids_with_role(server_domain, first_account_role)
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
    let jid = BareJid::from_parts(Some(&dto.username), server_domain);
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
