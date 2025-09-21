// prose-pod-server-api
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

// prose-pod-api
//
// Copyright: 2024–2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

mod prelude {
    pub(crate) use axum::{extract::FromRequestParts, http::request};

    pub(crate) use crate::util::{Context as _, NoContext as _};
    pub(crate) use crate::{AppState, responders};
}

use crate::extractors::prelude::*;

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

impl FromRequestParts<AppState> for crate::models::CallerInfo {
    type Rejection = responders::Error;

    #[tracing::instrument(name = "req::auth::caller_info", level = "trace", skip_all)]
    async fn from_request_parts(
        parts: &mut request::Parts,
        state: &AppState,
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

        let res: CachedValue = match state.oauth2_client.userinfo(&token).await {
            Ok(res) => {
                let jid = (BareJid::from_str(res.jid()))
                    .expect("JIDs coming from Prosody should always be valid");

                // FIXME: Replace calls to `prosodyctl` by calls to
                //   Prosody modules to avoid blocking shared access
                //   to `prosodyctl` (all calls are mutating).
                let mut prosodyctl = state.prosodyctl.write().await;

                let primary_role = prosodyctl.user_role(&jid, None).await.no_context()?;

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
