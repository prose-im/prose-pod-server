// prose-pod-server
//
// Copyright: 2025–2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use anyhow::Context as _;
use axum::extract::State;
use axum::http::Uri;
use axum::routing::{MethodRouter, put};
use axum::{Json, Router};
use prosody_rest::prose_xmpp::stanza::VCard4;
use prosody_rest::prose_xmpp::stanza::vcard4::PropertyContainer;
use serde::{Deserialize, Serialize};

use crate::errors::prelude::*;
use crate::models::{Avatar, BareJid, CallerInfo, Color};
use crate::responders;
use crate::state::prelude::*;
use crate::util::NoContext;

const ACCENT_COLOR_EXTENSION_KEY: &'static str = "x-accent-color";
const PROSE_POD_DASHBOARD_URL_EXTENSION_KEY: &'static str = "x-prose-pod-dashboard-url";
const PROSE_POD_API_URL_EXTENSION_KEY: &'static str = "x-prose-pod-api-url";
const PROSE_AUTO_UPDATE_ENABLED_EXTENSION_KEY: &'static str = "x-prose-auto-update-enabled";

pub(in crate::router) fn router() -> axum::Router<AppState<f::Running, b::Running>> {
    Router::<AppState>::new()
        .route("/workspace-init", put(self::init_workspace))
        .route(
            "/workspace",
            MethodRouter::new()
                .get(self::get_workspace)
                .patch(self::patch_workspace),
        )
        .route(
            "/workspace/name",
            MethodRouter::new()
                .get(self::get_workspace_name)
                .put(self::set_workspace_name),
        )
        .route(
            "/workspace/accent-color",
            MethodRouter::new()
                .get(self::get_workspace_accent_color)
                .put(self::set_workspace_accent_color),
        )
        .route(
            "/workspace/icon",
            MethodRouter::new()
                .get(self::get_workspace_icon)
                .put(self::set_workspace_icon),
        )
}

#[derive(Debug)]
#[serde_with::serde_as]
#[derive(Serialize)]
struct WorkspaceProfile {
    name: String,

    icon: Option<Avatar>,

    accent_color: Option<Color>,

    #[serde_as(as = "Option<serde_with::DisplayFromStr>")]
    prose_pod_dashboard_url: Option<Uri>,

    #[serde_as(as = "Option<serde_with::DisplayFromStr>")]
    prose_pod_api_url: Option<Uri>,

    auto_update_enabled: Option<bool>,
}

/// In the Dashboard, one can set the name of their Workspace before creating
/// the first admin account. This means they have to be able to use an
/// unauthenticated route to set the Workspace profile. This route enables it,
/// and works only until the first admin account is created. After that, it’ll
/// return 410 Gone.
async fn init_workspace(
    State(AppState {
        ref frontend,
        ref backend,
        ..
    }): State<AppState>,
    Json(req): Json<InitWorkspaceRequest>,
) -> Result<(), Error> {
    let server_domain = frontend.config.server.domain.as_str();
    let mut prosodyctl = backend.state.prosodyctl.write().await;
    let user_count = prosodyctl
        .user_get_jids_with_role(server_domain, "prosody:member")
        .await
        .no_context()?
        .len();

    if user_count > 0 {
        return Err(errors::too_late(
            "WORKSPACE_ALREADY_INITIALIZED",
            "Workspace already initialized",
            "You now need to log in as an admin to do that.",
        ));
    }

    let ref jid = frontend.config.workspace_jid();
    let ref creds = service_account_credentials(backend, jid)
        .await
        .no_context()?;

    let dashboard_url = frontend.config.dashboard_url();
    let api_url = frontend.config.pod_api_url().map_err(|error| {
        errors::internal_server_error(
            &error,
            "WORKSPACE_INITIALIZATION_FAILED",
            "Could not initialize the Workspace.",
        )
    })?;

    patch_workspace_vcard_unchecked(
        backend,
        creds,
        PatchWorkspaceCommand {
            name: Some(req.name),
            accent_color: req.accent_color.map(Some),
            prose_pod_dashboard_url: Some(Some(dashboard_url.to_owned())),
            prose_pod_api_url: Some(Some(api_url)),
            auto_update_enabled: None,
        },
    )
    .await
    .no_context()
}

#[derive(Debug)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InitWorkspaceRequest {
    name: String,
    #[serde(default)]
    accent_color: Option<Color>,
}

async fn get_workspace(
    State(AppState {
        ref frontend,
        ref backend,
        ..
    }): State<AppState>,
) -> Result<Json<WorkspaceProfile>, Error> {
    let ref jid = frontend.config.workspace_jid();
    let ref ctx = service_account_credentials(backend, jid)
        .await
        .no_context()?;

    let mut workspace = get_workspace_profile_minimal(backend, ctx).await?;

    workspace.icon = match service_account_avatar(backend, ctx).await {
        Ok(Some(avatar)) => Some(avatar),
        Ok(None) => None,
        Err(err) => {
            tracing::warn!("{err}");
            None
        }
    };

    Ok(Json(workspace))
}

pub async fn patch_workspace(
    State(AppState {
        ref frontend,
        ref backend,
        ..
    }): State<AppState>,
    caller_info: CallerInfo,
    Json(req): Json<PatchWorkspaceRequest>,
) -> Result<(), Error> {
    // Ensure the caller is an admin.
    // FIXME: Make this more flexible by checking rights instead of roles
    //   (which can be extended).
    match caller_info.primary_role.as_str() {
        "prosody:admin" | "prosody:operator" => {}
        _ => return Err(errors::forbidden("Only admins can do that.")),
    }

    let ref jid = frontend.config.workspace_jid();
    let ref creds = service_account_credentials(backend, jid)
        .await
        .no_context()?;

    patch_workspace_vcard_unchecked(backend, creds, req.into())
        .await
        .no_context()
}

/// The body of a HTTP `PATCH` request.
/// It contains only what a user can change in the Workspace vCard.
#[derive(Debug)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchWorkspaceRequest {
    #[serde(default)]
    pub name: Option<String>,

    #[serde(default, with = "crate::util::serde::null_as_some_none")]
    pub accent_color: Option<Option<Color>>,

    #[serde(default, with = "crate::util::serde::null_as_some_none")]
    pub auto_update_enabled: Option<Option<bool>>,
}

/// What the API itself can change in the Workspace vCard.
/// This is a superset of [`PatchWorkspaceRequest`].
#[derive(Debug, Default)]
pub struct PatchWorkspaceCommand {
    pub name: Option<String>,
    pub accent_color: Option<Option<Color>>,
    pub prose_pod_dashboard_url: Option<Option<Uri>>,
    pub prose_pod_api_url: Option<Option<Uri>>,
    pub auto_update_enabled: Option<Option<bool>>,
}

pub async fn get_workspace_name(
    State(AppState {
        ref frontend,
        ref backend,
        ..
    }): State<AppState>,
) -> Result<Json<String>, Error> {
    let ref jid = frontend.config.workspace_jid();
    let ref ctx = service_account_credentials(backend, jid)
        .await
        .no_context()?;

    let workspace = get_workspace_profile_minimal(backend, ctx).await?;

    Ok(Json(workspace.name))
}

pub async fn set_workspace_name(
    State(AppState {
        ref frontend,
        ref backend,
        ..
    }): State<AppState>,
    caller_info: CallerInfo,
    Json(name): Json<String>,
) -> Result<(), Error> {
    // Ensure the caller is an admin.
    // FIXME: Make this more flexible by checking rights instead of roles
    //   (which can be extended).
    match caller_info.primary_role.as_str() {
        "prosody:admin" | "prosody:operator" => {}
        _ => return Err(errors::forbidden("Only admins can do that.")),
    }

    let ref jid = frontend.config.workspace_jid();
    let ref creds = service_account_credentials(backend, jid)
        .await
        .no_context()?;

    patch_workspace_vcard_unchecked(
        backend,
        creds,
        PatchWorkspaceCommand {
            name: Some(name),
            ..Default::default()
        },
    )
    .await
    .no_context()
}

pub async fn get_workspace_accent_color(
    State(AppState {
        ref frontend,
        ref backend,
        ..
    }): State<AppState>,
) -> Result<Json<Option<Color>>, Error> {
    let ref jid = frontend.config.workspace_jid();
    let ref ctx = service_account_credentials(backend, jid)
        .await
        .no_context()?;

    let workspace = get_workspace_profile_minimal(backend, ctx).await?;

    Ok(Json(workspace.accent_color))
}

pub async fn set_workspace_accent_color(
    State(AppState {
        ref frontend,
        ref backend,
        ..
    }): State<AppState>,
    caller_info: CallerInfo,
    Json(color_opt): Json<Option<Color>>,
) -> Result<(), Error> {
    // Ensure the caller is an admin.
    // FIXME: Make this more flexible by checking rights instead of roles
    //   (which can be extended).
    match caller_info.primary_role.as_str() {
        "prosody:admin" | "prosody:operator" => {}
        _ => return Err(errors::forbidden("Only admins can do that.")),
    }

    let ref jid = frontend.config.workspace_jid();
    let ref creds = service_account_credentials(backend, jid)
        .await
        .no_context()?;

    patch_workspace_vcard_unchecked(
        backend,
        creds,
        PatchWorkspaceCommand {
            accent_color: Some(color_opt),
            ..Default::default()
        },
    )
    .await
    .no_context()
}

pub async fn get_workspace_icon(
    State(AppState {
        ref frontend,
        ref backend,
        ..
    }): State<AppState>,
) -> Result<Json<Option<Avatar>>, Error> {
    let ref jid = frontend.config.workspace_jid();
    let ref ctx = service_account_credentials(backend, jid)
        .await
        .no_context()?;

    let icon = match service_account_avatar(backend, ctx).await {
        Ok(Some(avatar)) => Some(avatar),
        Ok(None) => None,
        Err(err) => {
            tracing::warn!("{err}");
            None
        }
    };

    Ok(Json(icon))
}

pub async fn set_workspace_icon(
    State(AppState {
        ref frontend,
        ref backend,
        ..
    }): State<AppState>,
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

    let ref jid = frontend.config.workspace_jid();
    let ref ctx = service_account_credentials(backend, jid)
        .await
        .no_context()?;

    backend
        .prosody_rest
        .set_own_avatar(icon.into_bytes(), ctx)
        .await
        .context("Could not set Workspace icon")
        .no_context()?;

    Ok(())
}

// MARK: - Helpers

#[must_use]
#[inline]
pub(crate) async fn service_account_credentials(
    backend: &backend::Running,
    jid: &BareJid,
) -> Result<prosody_rest::CallerCredentials, anyhow::Error> {
    let token = backend.secrets_service.get_token(jid).await?;
    Ok(prosody_rest::CallerCredentials {
        bare_jid: jid.to_owned(),
        auth_token: token.inner().to_owned(),
    })
}

#[must_use]
#[inline]
async fn service_account_vcard(
    backend: &backend::Running,
    creds: &prosody_rest::CallerCredentials,
) -> Result<Option<VCard4>, anyhow::Error> {
    backend
        .prosody_rest
        .get_vcard(&creds.bare_jid, creds)
        .await
        .context("Could not get service account vCard")
}

/// NOTE: Avatars are not stored in vCards, we need to query them separately.
#[must_use]
#[inline]
async fn service_account_avatar(
    backend: &backend::Running,
    creds: &prosody_rest::CallerCredentials,
) -> Result<Option<Avatar>, Error> {
    match backend
        .prosody_rest
        .get_avatar(&creds.bare_jid, creds)
        .await
        .context("Could not get service account avatar")
        .no_context()?
    {
        Some(avatar_data) => Ok(Some(Avatar::try_from(avatar_data)?)),
        None => Ok(None),
    }
}

/// Get the workspace profile populated with vCard data only.
#[must_use]
#[inline]
async fn get_workspace_profile_minimal(
    backend: &backend::Running,
    creds: &prosody_rest::CallerCredentials,
) -> Result<WorkspaceProfile, Error> {
    match service_account_vcard(backend, creds).await.no_context()? {
        Some(vcard) => WorkspaceProfile::try_from(vcard),
        None => Err(workspace_not_initialized_error("No vCard.")),
    }
}

#[must_use]
#[inline]
pub(crate) async fn patch_workspace_vcard_unchecked(
    backend: &backend::Running,
    creds: &prosody_rest::CallerCredentials,
    PatchWorkspaceCommand {
        name,
        accent_color,
        prose_pod_dashboard_url,
        prose_pod_api_url,
        auto_update_enabled,
    }: PatchWorkspaceCommand,
) -> Result<(), anyhow::Error> {
    use prosody_rest::minidom::Element;
    use prosody_rest::prose_xmpp::ns;
    use prosody_rest::prose_xmpp::stanza::vcard4;

    let mut vcard = service_account_vcard(backend, creds)
        .await?
        .unwrap_or_default();
    let vcard_before = vcard.clone();

    if let Some(name) = name {
        vcard.fn_ = vec![vcard4::Fn_ { value: name }];
    }

    #[must_use]
    fn replace(
        unknown_properties: PropertyContainer,
        key: &'static str,
        new_value: Option<Option<impl ToString>>,
    ) -> PropertyContainer {
        match new_value {
            // Set to a new value.
            Some(Some(new_value)) => {
                let new_element = || {
                    Element::builder(key, ns::VCARD4)
                        .append(Element::builder("text", ns::VCARD4).append(new_value.to_string()))
                        .build()
                };

                let mut found = false;
                let mut unknown_properties = unknown_properties
                    .into_iter()
                    // Replace existing value to keep ordering intact.
                    // NOTE: This is important so we can check if changes have
                    //   been made to the vCard to skip unnecessary updates.
                    .map(|element| {
                        if element.name() == key {
                            found = true;
                            new_element()
                        } else {
                            element
                        }
                    })
                    .collect::<PropertyContainer>();

                if !found {
                    unknown_properties.push(new_element());
                }

                unknown_properties
            }

            // Remove existing value.
            Some(None) => unknown_properties
                .into_iter()
                .filter(|e| e.name() != key)
                .collect::<PropertyContainer>(),

            // Do nothing.
            None => unknown_properties,
        }
    }

    // TODO: Improve the `prose_xmpp` API to expose mutating methods not to
    //   force a new value to be created (potentially removing values by
    //   mistake!) and use it instead.
    let mut unknown_properties = vcard.unknown_properties;
    unknown_properties = replace(unknown_properties, ACCENT_COLOR_EXTENSION_KEY, accent_color);
    unknown_properties = replace(
        unknown_properties,
        PROSE_POD_DASHBOARD_URL_EXTENSION_KEY,
        prose_pod_dashboard_url,
    );
    unknown_properties = replace(
        unknown_properties,
        PROSE_POD_API_URL_EXTENSION_KEY,
        prose_pod_api_url,
    );
    unknown_properties = replace(
        unknown_properties,
        PROSE_AUTO_UPDATE_ENABLED_EXTENSION_KEY,
        auto_update_enabled,
    );
    vcard.unknown_properties = unknown_properties;

    if vcard != vcard_before {
        backend
            .prosody_rest
            .set_own_vcard(vcard, creds)
            .await
            .context("Could not set Workspace vCard")?;
    }

    Ok(())
}

// MARK: - Errors

#[must_use]
#[inline]
pub fn workspace_not_initialized_error(error: impl std::fmt::Display) -> Error {
    crate::errors::internal_server_error(
        &anyhow::anyhow!("{error}"),
        "WORKSPACE_NOT_INITIALIZED",
        "Workspace account not initialized.",
    )
}

// MARK: - Plumbing

impl From<PatchWorkspaceRequest> for PatchWorkspaceCommand {
    fn from(
        PatchWorkspaceRequest {
            name,
            accent_color,
            auto_update_enabled,
        }: PatchWorkspaceRequest,
    ) -> Self {
        Self {
            name,
            accent_color,
            prose_pod_dashboard_url: None,
            prose_pod_api_url: None,
            auto_update_enabled,
        }
    }
}

impl TryFrom<prosody_rest::prose_xmpp::stanza::VCard4> for WorkspaceProfile {
    type Error = responders::Error;

    fn try_from(vcard: prosody_rest::prose_xmpp::stanza::VCard4) -> Result<Self, Self::Error> {
        let Some(name) = vcard.fn_.first() else {
            return Err(workspace_not_initialized_error("Missing name."));
        };

        fn get_unknown_property<T: std::str::FromStr>(
            unknown_properties: &PropertyContainer,
            key: &'static str,
        ) -> Option<T>
        where
            T::Err: std::fmt::Display,
        {
            unknown_properties
                .get(ACCENT_COLOR_EXTENSION_KEY)
                .first()
                .map(|v| {
                    T::from_str(&v.text())
                        .inspect_err(|err| {
                            tracing::warn!("Invalid value for '{key}' in Workspace vCard: {err}")
                        })
                        .ok()
                })
                .flatten()
        }

        Ok(Self {
            name: name.value.to_owned(),
            // Avatars are not stored in vCards.
            icon: None,
            accent_color: get_unknown_property(
                &vcard.unknown_properties,
                ACCENT_COLOR_EXTENSION_KEY,
            ),
            prose_pod_dashboard_url: get_unknown_property(
                &vcard.unknown_properties,
                PROSE_POD_DASHBOARD_URL_EXTENSION_KEY,
            ),
            prose_pod_api_url: get_unknown_property(
                &vcard.unknown_properties,
                PROSE_POD_API_URL_EXTENSION_KEY,
            ),
            auto_update_enabled: get_unknown_property(
                &vcard.unknown_properties,
                PROSE_AUTO_UPDATE_ENABLED_EXTENSION_KEY,
            ),
        })
    }
}
