// prose-pod-server
//
// Copyright: 2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Uri};
use axum::response::Response;

use crate::responders::Error;
use crate::state::prelude::*;

pub async fn proxy_cloud_api(
    app_state: State<AppState>,
    request_uri: Uri,
    request_method: axum::http::Method,
    request_headers: HeaderMap,
    request_body: Body,
) -> Result<Response, Error> {
    let ref app_config = app_state.frontend.config;
    let cloud_api_url = app_config.proxy.cloud_api_url.to_string();

    crate::util::proxy(
        cloud_api_url.as_str(),
        "/cloud-api-proxy/",
        app_state,
        request_uri,
        request_method,
        request_headers,
        request_body,
    )
    .await
}
