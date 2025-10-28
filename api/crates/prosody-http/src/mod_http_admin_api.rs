// prosody-http-rs
//
// Copyright: 2024–2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::sync::Arc;

#[cfg(feature = "secrecy")]
use secrecy::ExposeSecret as _;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::json;
use ureq::http::header::ACCEPT;

use crate::{ProsodyHttpConfig, Secret, Timestamp, util::RequestBuilderExt as _};

/// Rust interface to [`mod_http_admin_api`](https://hg.prosody.im/prosody-modules/file/tip/mod_http_admin_api).
#[derive(Debug)]
pub struct ProsodyAdminApi {
    http_config: Arc<ProsodyHttpConfig>,
}

impl ProsodyAdminApi {
    pub fn new(http_config: Arc<ProsodyHttpConfig>) -> Self {
        Self { http_config }
    }
}

// MARK: Users

impl ProsodyAdminApi {
    pub async fn list_users(&self, auth: &Secret) -> Result<Box<[UserInfo]>, self::Error> {
        let response = self.get("/users").bearer_auth(auth).call()?;

        receive(response)
    }

    pub async fn get_user_by_name(
        &self,
        username: &str,
        auth: &Secret,
    ) -> Result<Option<UserInfo>, self::Error> {
        let response = self
            .get(&format!("/users/{username}"))
            .bearer_auth(auth)
            .call()?;

        receive(response).or_else(accept_not_found)
    }

    pub async fn update_user(
        &self,
        username: &str,
        req: &UpdateUserInfoRequest,
        auth: &Secret,
    ) -> Result<(), self::Error> {
        // TODO: It’s `PUT`, but it really behaves as a `PATCH`.
        //   We will fix this but until then we have to use `PUT`.
        let response = self
            .put(&format!("/users/{username}"))
            .bearer_auth(auth)
            .send_json(req)?;

        receive(response)
    }

    pub async fn delete_user(&self, username: &str, auth: &Secret) -> Result<(), self::Error> {
        let response = self
            .delete(&format!("/users/{username}"))
            .bearer_auth(auth)
            .call()?;

        // TODO: Ensure Prosody returns a `NotFound`-mapped error code
        //   when the user doesn’t exist.
        receive(response).or_else(accept_not_found)
    }
}

#[derive(Deserialize)]
pub struct UserInfo {
    pub jid: Box<str>,

    pub username: Box<str>,

    pub display_name: Box<str>,

    pub role: Option<Box<str>>,

    pub secondary_roles: Box<[Box<str>]>,

    pub groups: Box<[Box<str>]>,

    #[serde(default)]
    #[cfg_attr(feature = "time", serde(with = "time::serde::timestamp::option"))]
    pub last_active: Option<Timestamp>,
}

#[derive(Default)]
#[serde_with::skip_serializing_none]
#[derive(Serialize)]
pub struct UpdateUserInfoRequest {
    pub display_name: Option<String>,

    pub role: Option<String>,

    pub enabled: Option<bool>,

    pub email: Option<String>,
}

// MARK: Groups

impl ProsodyAdminApi {
    pub async fn create_group(
        &self,
        group_id: &str,
        group_name: &str,
        auth: &Secret,
    ) -> Result<(), self::Error> {
        let response = self
            .put("/groups")
            .bearer_auth(auth)
            .send_json(&json!({ "name": group_name, "id": group_id }))?;

        receive(response)
    }

    pub async fn add_group_member(
        &self,
        group_id: &str,
        username: &str,
        auth: &Secret,
    ) -> Result<(), self::Error> {
        let response = self
            .put(&format!("/groups/{group_id}/members/{username}"))
            .bearer_auth(auth)
            .send_empty()?;

        receive(response)
    }

    pub async fn remove_group_member(
        &self,
        group_id: &str,
        username: &str,
        auth: &Secret,
    ) -> Result<(), self::Error> {
        let response = self
            .delete(&format!("/groups/{group_id}/members/{username}"))
            .bearer_auth(auth)
            .call()?;

        receive(response).or_else(accept_not_found)
    }
}

// MARK: Invites

impl ProsodyAdminApi {
    pub async fn list_invites(&self, auth: &Secret) -> Result<Box<[InviteInfo]>, self::Error> {
        let response = self.get("/invites").bearer_auth(auth).call()?;

        receive(response)
    }

    pub async fn create_invite_for_account(
        &self,
        req: &CreateAccountInvitationRequest,
        auth: &Secret,
    ) -> Result<InviteInfo, self::Error> {
        let response = self
            .post("/invites/account")
            .bearer_auth(auth)
            .send_json(req)?;

        receive(response)
    }

    pub async fn create_invite_for_account_reset(
        &self,
        req: &CreateAccountResetInvitationRequest,
        auth: &Secret,
    ) -> Result<InviteInfo, self::Error> {
        let response = self
            .post("/invites/reset")
            .bearer_auth(auth)
            .send_json(req)?;

        receive(response)
    }

    pub async fn get_invite_by_id(
        &self,
        invite_id: &InviteId,
        auth: &Secret,
    ) -> Result<Option<InviteInfo>, self::Error> {
        #[cfg(feature = "secrecy")]
        let invite_id = invite_id.expose_secret();

        let response = self
            .get(&format!("/invites/{invite_id}"))
            .bearer_auth(auth)
            .call()?;

        receive(response).or_else(accept_not_found)
    }

    pub async fn delete_invite(
        &self,
        invite_id: &InviteId,
        auth: &Secret,
    ) -> Result<(), self::Error> {
        #[cfg(feature = "secrecy")]
        let invite_id = invite_id.expose_secret();

        let response = self
            .delete(&format!("/invites/{invite_id}"))
            .bearer_auth(auth)
            .call()?;

        receive(response).or_else(accept_not_found)
    }
}

// NOTE: What `mod_http_admin_api` calls “Invite IDs” really are invite tokens.
pub type InviteId = crate::Secret;

#[serde_with::skip_serializing_none]
#[derive(Serialize)]
pub struct CreateAccountInvitationRequest<AdditionalData = serde_json::Value> {
    pub username: Option<String>,

    #[cfg(not(feature = "time"))]
    #[serde(rename = "ttl")]
    pub ttl_secs: Option<u32>,

    #[cfg(feature = "time")]
    #[serde(with = "crate::util::serde::time::duration::option")]
    pub ttl: Option<time::Duration>,

    pub groups: Option<Vec<String>>,

    pub roles: Option<Vec<String>>,

    pub note: Option<String>,

    pub additional_data: AdditionalData,
}

#[serde_with::skip_serializing_none]
#[derive(Serialize)]
pub struct CreateAccountResetInvitationRequest<AdditionalData = serde_json::Value> {
    pub username: Option<String>,

    #[cfg(not(feature = "time"))]
    #[serde(rename = "ttl")]
    pub ttl_secs: Option<u32>,

    #[cfg(feature = "time")]
    #[serde(with = "crate::util::serde::time::duration::option")]
    pub ttl: Option<time::Duration>,

    pub additional_data: AdditionalData,
}

#[derive(Deserialize)]
pub struct InviteInfo<AdditionalData = serde_json::Value> {
    pub id: Box<str>,

    pub r#type: Box<str>,

    pub reusable: bool,

    #[serde(default)]
    pub inviter: Option<Box<str>>,

    pub jid: Box<str>,

    pub uri: Box<str>,

    #[serde(default)]
    pub landing_page: Option<Box<str>>,

    #[cfg_attr(feature = "time", serde(with = "time::serde::timestamp"))]
    pub created_at: Timestamp,

    #[cfg_attr(feature = "time", serde(with = "time::serde::timestamp"))]
    pub expires: Timestamp,

    #[serde(default)]
    pub groups: Box<[Box<str>]>,

    #[serde(default)]
    pub roles: Box<[Box<str>]>,

    #[serde(default)]
    pub source: Option<Box<str>>,

    pub reset: bool,

    #[serde(default)]
    pub note: Option<Box<str>>,

    pub additional_data: AdditionalData,
}

// MARK: - Errors

pub use self::ProsodyHttpAdminApiError as Error;

#[derive(Debug, thiserror::Error)]
pub enum ProsodyHttpAdminApiError {
    /// Bad request.
    #[error("Bad request: {0:?}")]
    BadRequest(anyhow::Error),

    /// Your authentication token is incorrect (possibly expired).
    #[error("Unauthorized: {0:?}")]
    Unauthorized(anyhow::Error),

    /// You’re not allowed to do what you asked for.
    #[error("Forbidden: {0:?}")]
    Forbidden(anyhow::Error),

    /// What you asked for doesn’t exist.
    ///
    /// Note that while most “not found” errors are mapped to `None` for better
    /// ergonomics, some non-`GET` routes might still return “not found” for
    /// internal reasons.
    #[error("Not found: {0:?}")]
    NotFound(anyhow::Error),

    /// What you wanted to create already exists.
    #[error("Conflict: {0:?}")]
    Conflict(anyhow::Error),

    /// One of us made a mistake somewhere.
    #[error("{0:?}")]
    Internal(anyhow::Error),

    /// An unknown error happened.
    ///
    /// The request has failed at the networking layer, there was a breaking
    /// change in Prosody or we didn’t write enough tests.
    #[error("{0:?}")]
    Other(#[from] anyhow::Error),
}

impl From<ureq::Error> for ProsodyHttpAdminApiError {
    fn from(err: ureq::Error) -> Self {
        Self::Other(anyhow::Error::new(err).context("Network error"))
    }
}

// MARK: - Helpers

impl ProsodyAdminApi {
    fn url(&self, path: &str) -> String {
        assert!(path.starts_with('/'));
        format!("{base}/admin_api{path}", base = self.http_config.url)
    }

    fn http_client(&self) -> ureq::Agent {
        let config = ureq::Agent::config_builder()
            // Do not let `ureq` handle client errors
            // so we can do debugging here.
            .http_status_as_error(false)
            .build();

        ureq::Agent::new_with_config(config)
    }

    fn get(&self, path: &str) -> ureq::RequestBuilder<ureq::typestate::WithoutBody> {
        self.http_client()
            .get(self.url(path))
            .header(ACCEPT, "application/json")
    }

    fn post(&self, path: &str) -> ureq::RequestBuilder<ureq::typestate::WithBody> {
        self.http_client()
            .post(self.url(path))
            .header(ACCEPT, "application/json")
    }

    fn put(&self, path: &str) -> ureq::RequestBuilder<ureq::typestate::WithBody> {
        self.http_client()
            .put(self.url(path))
            .header(ACCEPT, "application/json")
    }

    fn delete(&self, path: &str) -> ureq::RequestBuilder<ureq::typestate::WithoutBody> {
        self.http_client()
            .delete(self.url(path))
            .header(ACCEPT, "application/json")
    }
}

/// NOTE: This is separated from [`ProsodyAdminApi::get`] and similar
///   for two reasons:
///
///   1. Separate concerns.
///   2. Allow routes to do something with the response before it’s parsed.
///      It’s not something we do at the moment of writing this, but at least
///      we won’t have to rewrite everything if we need to do this.
fn receive<Response: DeserializeOwned>(
    mut response: ureq::http::Response<ureq::Body>,
) -> Result<Response, self::Error> {
    use anyhow::Context as _;

    if response.status().is_success() {
        let response = response
            .body_mut()
            .read_json::<Response>()
            .context("Could not decode Prosody OAuth 2.0 API response")?;

        Ok(response)
    } else {
        // Read the reponse body, as `mod_http_oauth2` isn’t very
        // expressive with HTTP status codes.
        let error = response
            .body_mut()
            .read_json::<crate::Error>()
            .context("Could not decode Prosody OAuth 2.0 API error")?;

        let condition = error
            .extra
            .as_ref()
            .map_or(error.condition.as_ref(), |extra| extra.condition.as_ref());

        match condition {
            // Bad request.
            "bad-request" | "group-name-required" => {
                tracing::debug!("{error}");
                Err(self::Error::BadRequest(anyhow::Error::new(error)))
            }

            // Unauthorized.
            "not-authorized" => {
                tracing::debug!("{error}");
                Err(self::Error::Unauthorized(anyhow::Error::new(error)))
            }

            // Forbidden.
            "forbidden" => {
                tracing::warn!("{error}");
                Err(self::Error::Forbidden(anyhow::Error::new(error)))
            }

            // Not found.
            "item-not-found" | "user-not-found" | "group-not-found" => {
                tracing::debug!("{error}");
                Err(self::Error::Unauthorized(anyhow::Error::new(error)))
            }

            // Conflict.
            "conflict" => {
                tracing::debug!("{error}");
                Err(self::Error::Conflict(anyhow::Error::new(error)))
            }

            // Internal errors.
            "internal-server-error" | "feature-not-implemented" => {
                tracing::error!("{error}");
                Err(self::Error::Internal(anyhow::Error::new(error)))
            }

            // Catch-all.
            _ => {
                tracing::error!("{error}");
                if cfg!(debug_assertions) {
                    panic!("Unknown error")
                }
                Err(self::Error::Internal(anyhow::Error::new(error)))
            }
        }
    }
}

fn accept_not_found<T: Default>(error: self::Error) -> Result<T, self::Error> {
    match error {
        self::Error::NotFound(_) => Ok(Default::default()),
        err => Err(err),
    }
}
