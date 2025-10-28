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

use crate::{BareJid, JidNodeView, SecretView, Timestamp};
use crate::{ProsodyHttpConfig, Secret, util::RequestBuilderExt as _};

/// Rust interface to [`mod_http_oauth2`](https://hg.prosody.im/prosody-modules/file/tip/mod_http_oauth2).
#[derive(Debug)]
pub struct ProsodyOAuth2 {
    http_config: Arc<ProsodyHttpConfig>,
}

impl ProsodyOAuth2 {
    pub fn new(http_config: Arc<ProsodyHttpConfig>) -> Self {
        Self { http_config }
    }
}

impl ProsodyOAuth2 {
    #[inline]
    pub async fn register(
        &self,
        client_config: &ClientConfig,
    ) -> Result<ClientMetadata, self::Error> {
        let data = json!(client_config);
        let response = self.post("/register").send_json(data)?;

        receive(response)
    }

    #[inline]
    pub async fn userinfo(&self, auth: &SecretView) -> Result<UserInfoResponse, self::Error> {
        let response = self.get("/userinfo").bearer_auth(auth).call()?;

        receive(response)
    }

    #[inline]
    pub async fn revoke(&self, auth: &SecretView) -> Result<(), self::Error> {
        let token = auth;

        #[cfg(feature = "secrecy")]
        let token = token.expose_secret();

        let data = json!({
            "token": token,
        });
        let response = self.post("/revoke").bearer_auth(auth).send_json(data)?;

        receive(response)
    }
}

impl ProsodyOAuth2 {
    /// Utility function (i.e. non-OAuth 2.0) that
    /// logs a user in using their credentials.
    #[inline]
    pub async fn util_log_in(
        &self,
        username: &JidNodeView,
        password: &SecretView,
        ClientCredentials {
            client_id,
            client_secret,
            ..
        }: &ClientCredentials,
    ) -> Result<TokenResponse, self::Error> {
        debug_assert!(
            !username.contains('@'),
            "Invalid username (has domainpart): {username}"
        );

        #[cfg(feature = "secrecy")]
        let password = password.expose_secret();

        let response = self
            .post("/token")
            .basic_auth(client_id, client_secret)
            .send_form([
                ("grant_type", "password"),
                ("username", username),
                ("password", password),
                // DOC: Space-separated list of scopes the client promises to restrict itself to.
                //   Supported scopes: [
                //     "prosody:operator", "prosody:admin", "prosody:member", "prosody:registered", "prosody:guest",
                //     "xmpp", "openid"
                //   ]
                // NOTE: "openid" scope required to use the `/userinfo` route.
                ("scope", "xmpp openid"),
            ])?;

        receive(response)
    }
}

pub use self::OAuth2ClientConfig as ClientConfig;

/// Client metadata required to register a new client.
#[derive(Debug)]
// NOTE: Derive `Default` just so we can use the `..Default::default()` syntax.
#[derive(Default)]
#[serde_with::skip_serializing_none]
#[derive(Serialize)]
pub struct OAuth2ClientConfig {
    /// Client Name.
    ///
    /// Human-readable name of the client, presented
    /// to the user in the consent dialog.
    pub client_name: String,

    /// Client URL.
    ///
    /// Should be an link to a page with information about
    /// the client. The hostname in this URL must be the same
    /// as in every other `_uri` property.
    ///
    /// WARN: Must start with `https:`.
    pub client_uri: String,

    /// Logo URL.
    ///
    /// URL to the clients logotype (not currently used).
    ///
    /// WARN: Must start with `https:`.
    pub logo_uri: Option<String>,

    /// List of Redirect URIs.
    ///
    /// WARN: At least one URI required.
    pub redirect_uris: Vec<String>,

    /// Grant Types.
    ///
    /// List of grant types the client intends to use.
    ///
    /// ```lua
    /// enum = {
    ///   "authorization_code";
    ///   "implicit";
    ///   "password";
    ///   "client_credentials";
    ///   "refresh_token";
    ///   "urn:ietf:params:oauth:grant-type:jwt-bearer";
    ///   "urn:ietf:params:oauth:grant-type:saml2-bearer";
    ///   "urn:ietf:params:oauth:grant-type:device_code";
    /// }
    /// ```
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub grant_types: Vec<String>,

    /// Application Type.
    ///
    /// Determines which kinds of redirect URIs the client may register.
    /// The value `web` limits the client to `https://` URLs with the same
    /// hostname as in `client_uri` while the value `native` allows either
    /// loopback URLs like `http://localhost:8080/` or application specific
    /// URIs like `com.example.app:/redirect`.
    ///
    /// ```lua
    /// enum = { "native"; "web" };
    /// default = "web";
    /// ```
    pub application_type: Option<String>,

    /// Response Types.
    ///
    /// ```lua
    /// enum = { "code"; "token" };
    /// default = { "code" };
    /// ```
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub response_types: Vec<String>,

    /// Token Endpoint Authentication Method.
    ///
    /// Authentication method the client intends to use.
    /// Recommended is `client_secret_basic`.
    /// `none` is only allowed for use with the insecure Implicit flow.
    ///
    /// ```lua
    /// enum = { "none"; "client_secret_post"; "client_secret_basic" }
    /// ```
    pub token_endpoint_auth_method: Option<String>,

    /// Scopes.
    ///
    /// Space-separated list of scopes the client
    /// promises to restrict itself to.
    ///
    /// ```lua
    /// examples = { "openid xmpp" };
    /// ```
    pub scope: Option<String>,

    /// Contact Addresses.
    ///
    /// Addresses, typically email or URLs where
    /// the client developers can be contacted.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub contacts: Vec<String>,

    /// Terms of Service URL.
    ///
    /// Link to Terms of Service for the client,
    /// presented to the user in the consent dialog.
    ///
    /// WARN: MUST be a `https://` URL with hostname
    ///   matching that of `client_uri`.
    pub tos_uri: Option<String>,

    /// Privacy Policy URL.
    ///
    /// Link to a Privacy Policy for the client.
    ///
    /// WARN: MUST be a `https://` URL with hostname
    ///   matching that of `client_uri`.
    pub policy_uri: Option<String>,

    /// Software ID.
    ///
    /// Unique identifier for the client software, common for all instances.
    /// Typically a UUID.
    pub software_id: Option<String>,

    /// Software Version.
    ///
    /// Version of the client software being registered. E.g. to allow
    /// revoking all related tokens in the event of a security incident.
    pub software_version: Option<String>,
}

/// Example value:
///
/// ```json
/// {
///   "scope": "openid xmpp",
///   "expires_in": 3600,
///   "token_type": "bearer",
///   "refresh_token": "secret-token:MjswYm5NamVYb3RfcjA7oz+tTt2tLVp1KnY3yBaGWP+MO3JvYmluX3JvYmVydHM5N0B0b3VnaC1vdmVyc2lnaHQub3Jn",
///   "access_token": "secret-token:MjswYm5NamVYb3RfcjA7ErQtXU5WxeQRK6ypyKSTmTizO3JvYmluX3JvYmVydHM5N0B0b3VnaC1vdmVyc2lnaHQub3Jn"
/// }
/// ```
#[derive(Debug, Deserialize)]
pub struct TokenResponse {
    pub scope: Box<str>,

    #[cfg(not(feature = "time"))]
    #[serde(rename = "expires_in")]
    pub expires_in_secs: u32,

    #[cfg(feature = "time")]
    #[serde(with = "crate::util::serde::time::duration")]
    pub expires_in: time::Duration,

    pub token_type: Box<str>,

    #[serde(default)]
    pub refresh_token: Option<Secret>,

    pub access_token: Secret,
}

pub use self::OAuth2ClientMetadata as ClientMetadata;

/// Example value:
///
/// ```json
/// {
///     "application_type": "web",
///     "client_secret": "3e841d9de79645a1c4c3f82f5d59485531b7e03119d8622acc31dc243baff2a5",
///     "redirect_uris": [
///         "https://prose-pod-api:8080/redirect"
///     ],
///     "iat": 1731265591,
///     "nonce": "Rxd4I7sqjsFq",
///     "response_types": [
///         "code"
///     ],
///     "exp": 1731269191,
///     "client_uri": "https://prose-pod-api:8080",
///     "client_name": "Prose Pod API",
///     "client_id_issued_at": 1731265591,
///     "client_id":"eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJhcHBsaWNhdGlvbl90eXBlIjoid2ViIiwiaWF0IjoxNzMxMjY1NTkxLCJjbGllbnRfbmFtZSI6IlByb3NlIFBvZCBBUEkiLCJyZXwb25zZV90eXBlcyI6WyJjb2RlIl0sImNsaWVudF91cmkiOiJodHRwczovL3Byb3NlLXBvZC1hcGk6ODA4MCIsImdyYW50X3R5cGVzIjpbImF1dGhvcml6YXRpb25fY29kZSJdLCJyZWRcmVjdF91cmlzIjpbImh0dHBzOi8vcHJvc2UtcG9kLWFwaTo4MDgwL3JlZGlyZWN0Il0sIm5vbmNlIjoiUnhkNEk3c3Fqc0ZxIiwidG9rZW5fZW5kcG9pbnRfYXV0aF9tZXRob2QiOiJjGllbnRfc2VjcmV0X2Jhc2ljIiwiZXhwIjoxNzMxMjY5MTkxfQ.-4b6hnqllAzH9TjzaRhQWbJ09cGuVs-8hXB05yLx1Qo",
///     "grant_types": [
///         "authorization_code"
///     ],
///     "token_endpoint_auth_method": "client_secret_basic",
///     "client_secret_expires_at": 0
/// }
/// ```
#[derive(Debug, Deserialize)]
pub struct OAuth2ClientMetadata {
    /// See [`OAuth2ClientConfig::client_name`].
    pub client_name: Box<str>,

    /// See [`OAuth2ClientConfig::client_uri`].
    pub client_uri: Box<str>,

    /// See [`OAuth2ClientConfig::logo_uri`].
    pub logo_uri: Option<Box<str>>,

    /// See [`OAuth2ClientConfig::redirect_uris`].
    pub redirect_uris: Box<[Box<str>]>,

    /// See [`OAuth2ClientConfig::grant_types`].
    pub grant_types: Box<[Box<str>]>,

    /// See [`OAuth2ClientConfig::application_type`].
    pub application_type: Option<Box<str>>,

    /// See [`OAuth2ClientConfig::response_types`].
    pub response_types: Box<[Box<str>]>,

    /// See [`OAuth2ClientConfig::token_endpoint_auth_method`].
    pub token_endpoint_auth_method: Option<Box<str>>,

    /// See [`OAuth2ClientConfig::scope`].
    pub scope: Option<Box<str>>,

    /// See [`OAuth2ClientConfig::contacts`].
    #[serde(default)]
    pub contacts: Box<[Box<str>]>,

    /// See [`OAuth2ClientConfig::tos_uri`].
    pub tos_uri: Option<Box<str>>,

    /// See [`OAuth2ClientConfig::policy_uri`].
    pub policy_uri: Option<Box<str>>,

    /// See [`OAuth2ClientConfig::software_id`].
    pub software_id: Option<Box<str>>,

    /// See [`OAuth2ClientConfig::software_version`].
    pub software_version: Option<Box<str>>,

    pub client_id: Box<str>,

    #[cfg_attr(feature = "time", serde(with = "time::serde::timestamp"))]
    pub client_id_issued_at: Timestamp,

    pub client_secret: Secret,

    /// WARN: DO NOT use `#[serde(default)]` here: `0` has a special meaning.
    #[cfg_attr(feature = "time", serde(with = "time::serde::timestamp"))]
    pub client_secret_expires_at: Timestamp,

    #[cfg_attr(feature = "time", serde(with = "time::serde::timestamp"))]
    pub iat: Timestamp,

    pub nonce: Box<str>,

    #[cfg_attr(feature = "time", serde(with = "time::serde::timestamp"))]
    pub exp: Timestamp,
}

pub use self::OAuth2ClientCredentials as ClientCredentials;

#[derive(Debug, Deserialize, Clone)]
pub struct OAuth2ClientCredentials {
    pub client_id: Box<str>,

    pub client_secret: Secret,
}

impl ClientMetadata {
    #[inline]
    pub fn into_credentials(self) -> ClientCredentials {
        ClientCredentials {
            client_id: self.client_id,
            client_secret: self.client_secret,
        }
    }
}

/// Example value:
///
/// ```json
/// {
///   "iss":"http://prose-pod-server:5280"
///   "sub":"xmpp:alice@test.org"
/// }
/// ```
#[derive(Debug, Deserialize)]
pub struct UserInfoResponse {
    pub iss: Box<str>,

    pub sub: Box<str>,
}

impl UserInfoResponse {
    #[cfg(not(feature = "jid"))]
    pub fn jid(&self) -> &str {
        use crate::util::PROSODY_VALID_JIDS;

        self.sub.strip_prefix("xmpp:").expect(PROSODY_VALID_JIDS)
    }

    #[cfg(feature = "jid")]
    pub fn jid(&self) -> BareJid {
        use crate::util::PROSODY_VALID_JIDS;

        BareJid::new(&self.sub).expect(PROSODY_VALID_JIDS)
    }
}

// MARK: - Errors

use ProsodyHttpErrorOAuth2Info as ApiError;

/// This is what `mod_http_oauth2` sends as `extra` in errors.
/// For user-facing (and API-stable) errors, see [`ProsodyHttpOAuth2Error`].
#[derive(Debug, Deserialize, thiserror::Error)]
#[error("{name}: {desc}", desc = description.as_deref().unwrap_or("<no_description>"))]
struct ProsodyHttpErrorOAuth2Info {
    #[serde(rename = "error")]
    #[doc(alias = "error")]
    name: Box<str>,

    #[serde(rename = "error_description", default)]
    #[doc(alias = "error_description")]
    description: Option<Box<str>>,
}

pub use self::ProsodyHttpOAuth2Error as Error;

#[derive(Debug, thiserror::Error)]
pub enum ProsodyHttpOAuth2Error {
    /// Your credentials are incorrect.
    #[error("Unauthorized: {0:?}")]
    Unauthorized(anyhow::Error),

    /// You’re not allowed to do what you asked for.
    #[error("Forbidden: {0:?}")]
    Forbidden(anyhow::Error),

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

impl From<ureq::Error> for ProsodyHttpOAuth2Error {
    fn from(err: ureq::Error) -> Self {
        Self::Other(anyhow::Error::new(err).context("Network error"))
    }
}

// MARK: - Helpers

impl ProsodyOAuth2 {
    fn url(&self, path: &str) -> String {
        assert!(path.starts_with('/'));
        format!("{base}/oauth2{path}", base = self.http_config.url)
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
}

/// NOTE: This is separated from [`ProsodyOAuth2::get`] and similar
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
            .read_json::<crate::Error<self::ApiError>>()
            .context("Could not decode Prosody OAuth 2.0 API error")?
            .into_inner();

        match error.name.as_ref() {
            // Unauthorized.
            "not-authorized" | "expired_token" | "invalid_grant" | "login_required" => {
                tracing::debug!("{error}");
                Err(self::Error::Unauthorized(anyhow::Error::new(error)))
            }
            "invalid_request" if error.description.as_deref() == Some("invalid JID") => {
                tracing::debug!("{error}");
                Err(self::Error::Unauthorized(anyhow::Error::new(error)))
            }

            // Forbidden.
            "forbidden" | "access_denied" => {
                tracing::warn!("{error}");
                Err(self::Error::Forbidden(anyhow::Error::new(error)))
            }

            // Internal errors.
            "internal-server-error"
            | "feature-not-implemented"
            | "invalid_client"
            | "invalid_client_metadata"
            | "invalid_redirect_uri"
            | "invalid_request"
            | "invalid_scope"
            | "temporarily_unavailable"
            | "unsupported_response_type" => {
                tracing::error!("{error}");
                Err(self::Error::Internal(anyhow::Error::new(error)))
            }
            "unauthorized_client" => {
                tracing::warn!(
                    "OAuth 2.0 client unauthorized ({error}). \
                    Make sure to register one before making calls."
                );
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
