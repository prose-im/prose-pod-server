// prosody-http-rs
//
// Copyright: 2024–2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::sync::Arc;

use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use serde_json::json;
use ureq::http::StatusCode;

use crate::{Password, ProsodyHttpConfig, util::RequestBuilderExt};

const BAD_RESPONSE_CONTEXT: &'static str = "Could not decode Prosody OAuth 2.0 API response";

pub type Client = ProsodyOAuth2Client;

/// Rust interface to [`mod_http_oauth2`](https://hg.prosody.im/prosody-modules/file/tip/mod_http_oauth2).
#[derive(Debug)]
pub struct ProsodyOAuth2Client {
    http_config: Arc<ProsodyHttpConfig>,
    client_config: OAuth2ClientConfig,
}

impl ProsodyOAuth2Client {
    pub fn new(http_config: Arc<ProsodyHttpConfig>, client_config: OAuth2ClientConfig) -> Self {
        Self {
            http_config: http_config.clone(),
            client_config,
        }
    }
}

impl ProsodyOAuth2Client {
    fn url(&self, path: &str) -> String {
        format!("{base}/oauth2/{path}", base = self.http_config.url)
    }

    pub async fn register(&self) -> crate::Result<OAuth2ClientMetadata> {
        let data = json!(self.client_config);
        let mut response = ureq::post(self.url("register")).send_json(data)?;

        let body: OAuth2ClientMetadata = response
            .body_mut()
            .read_json()
            .context(BAD_RESPONSE_CONTEXT)?;

        Ok(body)
    }

    pub async fn userinfo(&self, token: &Password) -> crate::Result<UserInfoResponse> {
        // NOTE: `403 Forbidden` doesn’t map to `500 Internal Server Error`
        //   thanks to `From<ureq::Error> for ProsodyHttpError`.
        let mut response = ureq::get(self.url("userinfo")).bearer_auth(token).call()?;

        let body: UserInfoResponse = response
            .body_mut()
            .read_json()
            .context(BAD_RESPONSE_CONTEXT)?;

        Ok(body)
    }
}

impl ProsodyOAuth2Client {
    /// Utility function (i.e. non-OAuth 2.0) that
    /// logs a user in using their credentials.
    pub async fn util_log_in(
        &self,
        jid: &str,
        password: &Password,
    ) -> crate::Result<TokenResponse> {
        let config = ureq::Agent::config_builder()
            // Do not let `ureq` handle client errors
            // so we can do debugging here.
            .http_status_as_error(false)
            .build();

        let mut response = ureq::Agent::new_with_config(config)
            .post(self.url("token"))
            .basic_auth(jid, password)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .send_form([
                ("grant_type", "password"),
                ("username", jid),
                #[cfg(feature = "secrecy")]
                ("password", secrecy::ExposeSecret::expose_secret(password)),
                #[cfg(not(feature = "secrecy"))]
                ("password", password),
                // DOC: Space-separated list of scopes the client promises to restrict itself to.
                //   Supported scopes: [
                //     "prosody:operator", "prosody:admin", "prosody:member", "prosody:registered", "prosody:guest",
                //     "xmpp", "openid"
                //   ]
                // NOTE: "openid" scope required to use the `/userinfo` route.
                ("scope", "xmpp openid"),
            ])?;

        let status = response.status();
        if status.is_success() {
            let body: TokenResponse = response
                .body_mut()
                .read_json()
                .context(BAD_RESPONSE_CONTEXT)?;

            Ok(body)
        } else if status == StatusCode::BAD_REQUEST {
            // Read the reponse body, as `mod_http_oauth2` isn’t very
            // expressive with HTTP status codes.
            let body = response.body_mut().read_to_string()?;
            tracing::debug!("Prosody OAuth 2.0 API returned status {status}: {body}");

            // NOTE: `mod_http_oauth2` error codes aren’t granular enough
            //   so we have to check individual error descriptions.
            //   It’s not ideal but we don’t have much choice.
            if body.contains("incorrect credentials") || body.contains("invalid JID") {
                Err(crate::Error::unauthorized("Invalid credentials."))
            } else {
                Err(crate::Error::from(response))
            }
        } else {
            Err(crate::Error::from(response))
        }
    }
}

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
///   "scope": "",
///   "expires_in": 3600,
///   "token_type": "bearer",
///   "refresh_token": "secret-token:MjswYm5NamVYb3RfcjA7oz+tTt2tLVp1KnY3yBaGWP+MO3JvYmluX3JvYmVydHM5N0B0b3VnaC1vdmVyc2lnaHQub3Jn",
///   "access_token": "secret-token:MjswYm5NamVYb3RfcjA7ErQtXU5WxeQRK6ypyKSTmTizO3JvYmluX3JvYmVydHM5N0B0b3VnaC1vdmVyc2lnaHQub3Jn"
/// }
/// ```
#[derive(Debug, Deserialize)]
pub struct TokenResponse {
    pub scope: String,
    pub expires_in: u32,
    pub token_type: String,
    pub refresh_token: Password,
    pub access_token: Password,
    // pub grant_jid: String,
}

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
    /// See [`PartialClientMetadata::client_name`].
    pub client_name: String,

    /// See [`PartialClientMetadata::client_uri`].
    pub client_uri: String,

    /// See [`PartialClientMetadata::logo_uri`].
    pub logo_uri: Option<String>,

    /// See [`PartialClientMetadata::redirect_uris`].
    pub redirect_uris: Vec<String>,

    /// See [`PartialClientMetadata::grant_types`].
    pub grant_types: Vec<String>,

    /// See [`PartialClientMetadata::application_type`].
    pub application_type: Option<String>,

    /// See [`PartialClientMetadata::response_types`].
    pub response_types: Vec<String>,

    /// See [`PartialClientMetadata::token_endpoint_auth_method`].
    pub token_endpoint_auth_method: Option<String>,

    /// See [`PartialClientMetadata::scope`].
    pub scope: Option<String>,

    /// See [`PartialClientMetadata::contacts`].
    #[serde(default)]
    pub contacts: Vec<String>,

    /// See [`PartialClientMetadata::tos_uri`].
    pub tos_uri: Option<String>,

    /// See [`PartialClientMetadata::policy_uri`].
    pub policy_uri: Option<String>,

    /// See [`PartialClientMetadata::software_id`].
    pub software_id: Option<String>,

    /// See [`PartialClientMetadata::software_version`].
    pub software_version: Option<String>,

    pub client_id: String,
    pub client_id_issued_at: u32,
    pub client_secret: Password,
    pub client_secret_expires_at: u32,
    pub iat: u32,
    pub nonce: String,
    pub exp: u32,
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
    pub iss: String,
    pub sub: String,
}

impl UserInfoResponse {
    pub fn jid(&self) -> &str {
        self.sub.strip_prefix("xmpp:").unwrap()
    }
}
