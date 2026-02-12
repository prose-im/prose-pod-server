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

pub async fn proxy_prose_files(
    app_state: State<AppState>,
    request_uri: Uri,
    request_method: axum::http::Method,
    request_headers: HeaderMap,
    request_body: Body,
) -> Result<Response, Error> {
    let ref app_config = app_state.frontend.config;
    let prose_files_url = app_config.proxy.prose_files_url.to_string();

    crate::util::proxy(
        prose_files_url.as_str(),
        "/prose-files-proxy/",
        app_state,
        request_uri,
        request_method,
        request_headers,
        request_body,
    )
    .await
}
