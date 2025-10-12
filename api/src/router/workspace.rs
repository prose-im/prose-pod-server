// prose-pod-server-api
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::sync::Arc;

use anyhow::Context as _;
use axum::extract::State;
use axum::routing::{MethodRouter, get};
use axum::{Json, Router};
use prosody_rest::prose_xmpp::stanza::VCard4;
use serde::Serialize;

use crate::errors::{invalid_avatar, prelude::*};
use crate::models::{Avatar, BareJid, CallerInfo, Color};
use crate::state::AppState;
use crate::util::{NoContext, jid_0_12_to_jid_0_11};
use crate::{AppConfig, responders};

const ACCENT_COLOR_EXTENSION_KEY: &'static str = "x-accent-color";

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/workspace", get(get_workspace))
        .route(
            "/workspace/name",
            MethodRouter::new()
                .get(get_workspace_name)
                .put(set_workspace_name),
        )
        .route(
            "/workspace/accent-color",
            MethodRouter::new()
                .get(get_workspace_accent_color)
                .put(set_workspace_accent_color),
        )
        .route(
            "/workspace/icon",
            MethodRouter::new()
                .get(get_workspace_icon)
                .put(set_workspace_icon),
        )
}

#[derive(Debug)]
#[derive(Serialize)]
struct WorkspaceProfile {
    name: String,
    icon: Option<Avatar>,
    accent_color: Option<Color>,
}

async fn get_workspace(
    State(ref app_state): State<AppState>,
    State(ref app_config): State<Arc<AppConfig>>,
) -> Result<Json<WorkspaceProfile>, Error> {
    let ref jid = app_config.workspace_jid();
    let ref ctx = service_account_credentials(app_state, jid).await?;

    let mut workspace = get_workspace_profile_minimal(app_state, ctx).await?;

    workspace.icon = match service_account_avatar(app_state, ctx).await {
        Ok(Some(avatar)) => Some(avatar),
        Ok(None) => None,
        Err(err) => {
            tracing::warn!("{err}");
            None
        }
    };

    Ok(Json(workspace))
}

async fn get_workspace_name(
    State(ref app_state): State<AppState>,
    State(ref app_config): State<Arc<AppConfig>>,
) -> Result<Json<String>, Error> {
    let ref jid = app_config.workspace_jid();
    let ref ctx = service_account_credentials(app_state, jid).await?;

    let workspace = get_workspace_profile_minimal(app_state, ctx).await?;

    Ok(Json(workspace.name))
}

async fn set_workspace_name(
    State(ref app_state): State<AppState>,
    State(ref app_config): State<Arc<AppConfig>>,
    caller_info: CallerInfo,
    Json(name): Json<String>,
) -> Result<(), Error> {
    use prosody_rest::prose_xmpp::stanza::vcard4;

    // Ensure the caller is an admin.
    // FIXME: Make this more flexible by checking rights instead of roles
    //   (which can be extended).
    match caller_info.primary_role.as_str() {
        "prosody:admin" | "prosody:operator" => {}
        _ => return Err(errors::forbidden("Only admins can do that.")),
    }

    let ref jid = app_config.workspace_jid();
    let ref ctx = service_account_credentials(app_state, jid).await?;

    let mut vcard = service_account_vcard(app_state, ctx)
        .await?
        .unwrap_or_default();

    vcard.fn_ = vec![vcard4::Fn_ { value: name }];

    app_state
        .prosody_rest
        .set_own_vcard(vcard, ctx)
        .await
        .context("Could not set Workspace vCard")
        .no_context()?;

    Ok(())
}

async fn get_workspace_accent_color(
    State(ref app_state): State<AppState>,
    State(ref app_config): State<Arc<AppConfig>>,
) -> Result<Json<Option<Color>>, Error> {
    let ref jid = app_config.workspace_jid();
    let ref ctx = service_account_credentials(app_state, jid).await?;

    let workspace = get_workspace_profile_minimal(app_state, ctx).await?;

    Ok(Json(workspace.accent_color))
}

async fn set_workspace_accent_color(
    State(ref app_state): State<AppState>,
    State(ref app_config): State<Arc<AppConfig>>,
    caller_info: CallerInfo,
    Json(color_opt): Json<Option<Color>>,
) -> Result<(), Error> {
    use prosody_rest::minidom::Element;
    use prosody_rest::prose_xmpp::ns;

    // Ensure the caller is an admin.
    // FIXME: Make this more flexible by checking rights instead of roles
    //   (which can be extended).
    match caller_info.primary_role.as_str() {
        "prosody:admin" | "prosody:operator" => {}
        _ => return Err(errors::forbidden("Only admins can do that.")),
    }

    let ref jid = app_config.workspace_jid();
    let ref ctx = service_account_credentials(app_state, jid).await?;

    let Some(mut vcard) = service_account_vcard(app_state, ctx).await? else {
        return Err(workspace_not_initialized_error("No vCard."));
    };

    // FIXME: Do not override all unknown properties! Improve the `prose_xmpp`
    //   API to expose mutating methods and use it here instead.
    match color_opt {
        Some(color) => {
            vcard.unknown_properties = vec![
                Element::builder(ACCENT_COLOR_EXTENSION_KEY, ns::VCARD4)
                    .append(Element::builder("text", ns::VCARD4).append(color.to_string()))
                    .build(),
            ]
            .into_iter()
            .collect()
        }
        None => vcard.unknown_properties = Default::default(),
    };

    app_state
        .prosody_rest
        .set_own_vcard(vcard, ctx)
        .await
        .context("Could not set Workspace vCard")
        .no_context()?;

    Ok(())
}

async fn get_workspace_icon(
    State(ref app_state): State<AppState>,
    State(ref app_config): State<Arc<AppConfig>>,
) -> Result<Json<Option<Avatar>>, Error> {
    let ref jid = app_config.workspace_jid();
    let ref ctx = service_account_credentials(app_state, jid).await?;

    let icon = match service_account_avatar(app_state, ctx).await {
        Ok(Some(avatar)) => Some(avatar),
        Ok(None) => None,
        Err(err) => {
            tracing::warn!("{err}");
            None
        }
    };

    Ok(Json(icon))
}

async fn set_workspace_icon(
    State(ref app_state): State<AppState>,
    State(ref app_config): State<Arc<AppConfig>>,
    caller_info: CallerInfo,
    icon: Avatar,
) -> Result<(), Error> {
    // Ensure the caller is an admin.
    // FIXME: Make this more flexible by checking rights instead of roles
    //   (which can be extended).
    match caller_info.primary_role.as_str() {
        "prosody:admin" | "prosody:operator" => {}
        _ => return Err(errors::forbidden("Only admins can do that.")),
    }

    let ref jid = app_config.workspace_jid();
    let ref ctx = service_account_credentials(app_state, jid).await?;

    app_state
        .prosody_rest
        .set_own_avatar(icon.bytes, ctx)
        .await
        .context("Could not set Workspace icon")
        .no_context()?;

    Ok(())
}

// MARK: - Helpers

#[must_use]
#[inline]
async fn service_account_credentials(
    app_state: &AppState,
    jid: &BareJid,
) -> Result<prosody_rest::CallerCredentials, Error> {
    let token = app_state
        .secrets_service
        .get_token(jid)
        .await
        .no_context()?;
    Ok(prosody_rest::CallerCredentials {
        bare_jid: jid_0_12_to_jid_0_11(jid),
        auth_token: token.inner().to_owned(),
    })
}

#[must_use]
#[inline]
async fn service_account_vcard(
    app_state: &AppState,
    creds: &prosody_rest::CallerCredentials,
) -> Result<Option<VCard4>, Error> {
    app_state
        .prosody_rest
        .get_vcard(&creds.bare_jid, creds)
        .await
        .context("Could not get service account vCard")
        .no_context()
}

/// NOTE: Avatars are not stored in vCards, we need to query them separately.
#[must_use]
#[inline]
async fn service_account_avatar(
    app_state: &AppState,
    creds: &prosody_rest::CallerCredentials,
) -> Result<Option<Avatar>, Error> {
    match app_state
        .prosody_rest
        .get_avatar(&creds.bare_jid, creds)
        .await
        .context("Could not get service account avatar")
        .no_context()?
    {
        Some(avatar_data) => Ok(Some(Avatar::try_from(avatar_data).map_err(invalid_avatar)?)),
        None => Ok(None),
    }
}

/// Get the workspace profile populated with vCard data only.
#[must_use]
#[inline]
async fn get_workspace_profile_minimal(
    app_state: &AppState,
    creds: &prosody_rest::CallerCredentials,
) -> Result<WorkspaceProfile, Error> {
    match service_account_vcard(app_state, creds).await? {
        Some(vcard) => WorkspaceProfile::try_from(vcard),
        None => Err(workspace_not_initialized_error("No vCard.")),
    }
}

// MARK: - Errors

#[must_use]
#[inline]
pub fn workspace_not_initialized_error(error: impl std::fmt::Debug) -> Error {
    crate::errors::internal_server_error(
        error,
        "WORKSPACE_NOT_INITIALIZED",
        "Workspace account not initialized",
    )
}

// MARK: - Plumbing

impl TryFrom<prosody_rest::prose_xmpp::stanza::VCard4> for WorkspaceProfile {
    type Error = responders::Error;

    fn try_from(vcard: prosody_rest::prose_xmpp::stanza::VCard4) -> Result<Self, Self::Error> {
        use std::str::FromStr as _;

        let Some(name) = vcard.fn_.first() else {
            return Err(workspace_not_initialized_error("Missing name."));
        };

        Ok(Self {
            name: name.value.to_owned(),
            // Avatars are not stored in vCards.
            icon: None,
            accent_color: vcard
                .unknown_properties
                .get(ACCENT_COLOR_EXTENSION_KEY)
                .first()
                .map(|v| {
                    Color::from_str(&v.text())
                        .inspect_err(|err| {
                            tracing::warn!("Invalid accent color stored in Workspace vCard: {err}")
                        })
                        .ok()
                })
                .flatten(),
        })
    }
}
