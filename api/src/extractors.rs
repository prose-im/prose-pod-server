// prose-pod-server
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

mod prelude {
    pub(crate) use axum::{
        body::Bytes,
        extract::{FromRequest, FromRequestParts, Request},
        http::request,
    };

    pub(crate) use crate::state::prelude::*;
    pub(crate) use crate::util::{Context as _, NoContext as _};
    pub(crate) use crate::{errors, responders};
}

use crate::{extractors::prelude::*, util::PROSODY_JIDS_ARE_VALID};

impl<State: Send + Sync> FromRequestParts<State> for crate::models::AuthToken {
    type Rejection = responders::Error;

    #[tracing::instrument(name = "req::auth::bearer", level = "trace", skip_all)]
    async fn from_request_parts(
        parts: &mut request::Parts,
        _state: &State,
    ) -> Result<Self, Self::Rejection> {
        use crate::errors::unauthorized;
        use axum::http::header::AUTHORIZATION;

        const BEARER_PREFIX: &'static str = "Bearer ";

        // Read the **first** `Authorization` header.
        let Some(auth) = parts.headers.get(AUTHORIZATION) else {
            return Err(unauthorized(format!("No '{AUTHORIZATION}' header found.")));
        };

        // Accept only visible ASCII chars.
        let auth = (auth.to_str())
            .map_err(|err| unauthorized(format!("Bad '{AUTHORIZATION}' header value: {err}")))?;

        // Strip prefix.
        let Some(token) = auth.strip_prefix(BEARER_PREFIX) else {
            return Err(unauthorized(format!(
                "The '{AUTHORIZATION}' header does not start with '{BEARER_PREFIX}'."
            )));
        };

        Ok(Self::from(token))
    }
}

impl<F: frontend::State> FromRequestParts<AppState<F, backend::Running>>
    for crate::models::CallerInfo
{
    type Rejection = responders::Error;

    #[tracing::instrument(name = "req::auth::caller_info", level = "trace", skip_all)]
    async fn from_request_parts(
        parts: &mut request::Parts,
        state: &AppState<F, backend::Running>,
    ) -> Result<Self, Self::Rejection> {
        use crate::models::{AuthToken, BareJid};
        use std::str::FromStr as _;

        // NOTE: Ensures we store and read the same type (otherwise caching
        //   would be useless as Axum wouldn’t find the value).
        type CachedValue = Result<crate::models::CallerInfo, responders::Error>;

        // Read cache to avoid unnecessary recomputations.
        // NOTE: On a local run, this extractor seems to take around 5ms to run.
        //   It doesn’t seem much, but this function can be called multiple
        //   times *per request* resulting in unnecessary delay. In addition,
        //   every call to `AuthService::get_user_info` results in at least one
        //   call to the XMPP server and at least one call to the database. If
        //   one of those is already under heavy load (which was not the case in
        //   our local test run), this extractor will take even longer (and
        //   increase said load).
        //   Caching avoids all of that and a cache hit takes around 25µs
        //   (likely O(1)) which is a non-negligible improvement (>200x faster).
        // NOTE: Unless it becomes an issue, we won’t add a higher level cache
        //   to avoid recomputations on repeated calls. Such cache, if
        //   misimplemented, could result in security issues (wrong
        //   role/privileges). If we ever do implement such cache, we MUST make
        //   sure said cache expires after a short time.
        match parts.extensions.get::<CachedValue>() {
            Some(cache) => {
                tracing::debug!("Cache hit.");
                return cache.clone();
            }
            None => tracing::debug!("Cache miss."),
        }

        // Get user info from auth token.
        let token = AuthToken::from_request_parts(parts, state).await?;

        let ref state = state.backend.state;
        let res: CachedValue = match state.oauth2_client.userinfo(&token).await {
            Ok(res) => {
                let jid = (BareJid::from_str(res.jid())).expect(PROSODY_JIDS_ARE_VALID);

                // FIXME: Replace calls to `prosodyctl` by calls to
                //   Prosody modules to avoid blocking shared access
                //   to `prosodyctl` (all calls are mutating).
                let mut prosodyctl = state.prosodyctl.write().await;

                let primary_role = prosodyctl
                    .user_role(jid.as_str(), None)
                    .await
                    .no_context()?;

                drop(prosodyctl);

                let caller_info = Self { jid, primary_role };

                Ok(caller_info)
            }
            Err(err) => Err(err.context("OAUTH2_ERROR", "OAuth 2.0 error")),
        };

        // Cache value to avoid recomputations next time.
        (parts.extensions).insert::<CachedValue>(res.clone());
        tracing::debug!("Cache stored.");

        res
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AvatarFromRequestError {
    #[error("Invalid bytes: {0}")]
    InvalidBytes(#[from] axum::extract::rejection::BytesRejection),
    #[error("Invalid string: {0}")]
    InvalidString(#[from] axum::extract::rejection::StringRejection),
    #[error("Invalid JSON: {0}")]
    InvalidJson(#[from] axum::extract::rejection::JsonRejection),
    #[error("Invalid avatar: {0}")]
    InvalidAvatar(#[from] crate::models::AvatarDecodeError),
    #[error("Unsupported media type.")]
    UnsupportedMediaType,
}

impl<State: Send + Sync> FromRequest<State> for crate::models::Avatar {
    type Rejection = AvatarFromRequestError;

    async fn from_request(req: Request, _state: &State) -> Result<Self, Self::Rejection> {
        use crate::models::Avatar;
        use axum::Json;

        let content_type = req.headers().get("Content-Type");

        async fn from_bytes(req: Request) -> Result<Avatar, AvatarFromRequestError> {
            let bytes = Bytes::from_request(req, &()).await?;
            // TODO: Find a way to avoid this copy?
            let avatar = Avatar::try_from_bytes(bytes.to_vec().into_boxed_slice())?;
            Ok(avatar)
        }

        async fn from_text(req: Request) -> Result<Avatar, AvatarFromRequestError> {
            let string = String::from_request(req, &()).await?;
            let avatar = Avatar::try_from_base64_string(string)?;
            Ok(avatar)
        }

        async fn from_json(req: Request) -> Result<Avatar, AvatarFromRequestError> {
            let Json(string) = Json::<String>::from_request(req, &()).await?;
            let avatar = Avatar::try_from_base64_string(string)?;
            Ok(avatar)
        }

        match content_type {
            None => from_bytes(req).await,
            Some(ct) if ct == "application/octet-stream" => from_bytes(req).await,
            Some(ct) if ct.as_bytes().starts_with("image/".as_bytes()) => from_bytes(req).await,
            Some(ct) if ct == "text/plain" => from_text(req).await,
            Some(ct) if ct == "application/json" => from_json(req).await,
            _ => Err(Self::Rejection::UnsupportedMediaType),
        }
    }
}

// MARK: - Boilerplate

impl axum::response::IntoResponse for AvatarFromRequestError {
    fn into_response(self) -> axum::response::Response {
        match self {
            Self::InvalidAvatar(error) => responders::Error::from(error),
            err @ (Self::UnsupportedMediaType
            | Self::InvalidBytes(_)
            | Self::InvalidString(_)
            | Self::InvalidJson(_)) => {
                errors::validation_error("INVALID_AVATAR", "Invalid avatar", err.to_string())
            }
        }
        .into_response()
    }
}
