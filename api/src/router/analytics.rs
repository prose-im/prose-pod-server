// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use axum::Json;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Uri};
use axum::response::NoContent;
use axum_extra::either::Either;
use reqwest::header::CONTENT_LENGTH;

use crate::analytics::{AnalyticsEvent, process_event};
use crate::responders::Error;
use crate::state::prelude::*;
use crate::util::NoContext as _;

pub async fn proxy_analytics_event(
    app_state: State<AppState>,
    request_uri: Uri,
    request_method: axum::http::Method,
    mut request_headers: HeaderMap,
    Json(event): Json<AnalyticsEvent>,
) -> Result<Either<axum::response::Response, NoContent>, Error> {
    use anyhow::Context as _;

    let domain = &app_state.frontend.config.server.domain;

    let mut prosodyctl = app_state.backend.prosodyctl.write().await;

    // NOTE: We need to filter out service accounts,
    //   which don’t have the `prosody:member` role.
    let user_count = prosodyctl
        .user_get_jids_with_role(domain, "prosody:member")
        .await
        .no_context()?
        .len();

    drop(prosodyctl);

    let Some(event) = process_event(
        event,
        &app_state.frontend.config.vendor_analytics,
        domain,
        user_count as u64,
        &app_state.backend.server_salt,
    ) else {
        return Ok(Either::E2(NoContent));
    };

    let body_bytes: Vec<u8> = json::to_vec(&event)
        .context("Could not serialize processed analytics event")
        .no_context()?;

    // Update `Content-Length` header.
    request_headers.remove(CONTENT_LENGTH);
    request_headers.append(CONTENT_LENGTH, body_bytes.len().into());

    super::cloud_api_proxy::proxy_cloud_api(
        app_state,
        request_uri,
        request_method,
        request_headers,
        Body::from(body_bytes),
    )
    .await
    .map(Either::E1)
}
