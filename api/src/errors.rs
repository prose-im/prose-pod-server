// prose-pod-server-api
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use crate::responders::Error;

// MARK: Internal errors

/// NOTE: Not public to “force” the usage of [`internal_server_error`].
const ERROR_KIND_INTERNAL: &'static str = "INTERNAL_ERROR";
/// To use when we don’t want to leak internal info.
pub const ERROR_CODE_INTERNAL: &'static str = "INTERNAL_ERROR";

/// `public_description` is a short user-facing description.
/// It MUST NOT leak any internal information.
///
/// WARN: This description will be sent as
///   “{public_description} logged as {error_id}.”
///   therefore it MUST NOT end with a period (`.`).
#[must_use]
#[inline]
pub fn internal_server_error(
    error: impl std::fmt::Debug,
    code: &'static str,
    public_description: impl Into<String>,
) -> Error {
    let public_description: String = public_description.into();
    assert!(
        !public_description.ends_with("."),
        "Error description will be put in a sentence; \
            it MUST NOT end with a period (`.`)."
    );

    // Log error debug information with a unique ID,
    // and reference this ID in the user-facing description.
    let error_id = crate::util::random_id(8);
    tracing::error!(error_id, "Internal error: {public_description}: {error:?}");
    let description = format!("{public_description} logged as {error_id}.");

    Error::new(
        ERROR_KIND_INTERNAL,
        code,
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        "Internal server error".to_owned(),
        description,
    )
}

// MARK: Auth errors

pub const ERROR_KIND_AUTH: &'static str = "AUTH_ERROR";

#[must_use]
#[inline]
pub fn unauthorized(description: impl Into<String>) -> Error {
    Error::new(
        ERROR_KIND_AUTH,
        "UNAUTHORIZED",
        axum::http::StatusCode::UNAUTHORIZED,
        "Unauthorized".to_owned(),
        description,
    )
}

#[must_use]
#[inline]
pub fn forbidden(description: impl Into<String>) -> Error {
    Error::new(
        ERROR_KIND_AUTH,
        "FORBIDDEN",
        axum::http::StatusCode::FORBIDDEN,
        "Forbidden".to_owned(),
        description,
    )
}
