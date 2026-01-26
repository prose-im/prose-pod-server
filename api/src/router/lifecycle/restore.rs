// prose-pod-server
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use axum::extract::State;
use axum::http::StatusCode;

use crate::responders::Error;
use crate::state::prelude::*;

// MARK: - Routes

pub(in crate::router) async fn restore(
    State(app_state): State<AppState<f::Running, b::Running>>,
) -> Result<StatusCode, Error> {
    todo!()
}
