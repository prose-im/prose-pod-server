// prose-pod-server-api
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

pub mod prelude {
    pub use axum::http::StatusCode;

    #[allow(unused)]
    pub(crate) use crate::errors;
    pub use crate::responders::Error;
}

use prelude::*;

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
        StatusCode::INTERNAL_SERVER_ERROR,
        "Internal server error",
        description,
    )
}

// MARK: Auth errors

const ERROR_KIND_AUTH: &'static str = "AUTH_ERROR";

#[must_use]
#[inline]
pub fn unauthorized(description: impl Into<String>) -> Error {
    Error::new(
        ERROR_KIND_AUTH,
        "UNAUTHORIZED",
        StatusCode::UNAUTHORIZED,
        "Unauthorized",
        description,
    )
}

#[must_use]
#[inline]
pub fn forbidden(description: impl Into<String>) -> Error {
    Error::new(
        ERROR_KIND_AUTH,
        "FORBIDDEN",
        StatusCode::FORBIDDEN,
        "Forbidden",
        description,
    )
}

// MARK: Lifecycle errors (initialization done, restarting…)

const ERROR_KIND_LIFECYCLE: &'static str = "LIFECYCLE_ERROR";

#[must_use]
#[inline]
pub fn too_late(
    code: &'static str,
    message: impl Into<String>,
    description: impl Into<String>,
) -> Error {
    Error::new(
        ERROR_KIND_LIFECYCLE,
        code,
        StatusCode::GONE,
        message,
        description,
    )
}

// MARK: Other error kinds

#[must_use]
#[inline]
pub fn conflict_error(
    code: &'static str,
    message: impl Into<String>,
    description: impl Into<String>,
) -> Error {
    Error::new(
        "CONFLICT_ERROR",
        code,
        StatusCode::CONFLICT,
        message,
        description,
    )
}

#[must_use]
#[inline]
pub fn validation_error(
    code: &'static str,
    message: impl Into<String>,
    description: impl Into<String>,
) -> Error {
    Error::new(
        "VALIDATION_ERROR",
        code,
        StatusCode::BAD_REQUEST,
        message,
        description,
    )
}

// MARK: - Conversions

pub fn invalid_avatar(err: impl ToString) -> Error {
    self::validation_error("INVALID_AVATAR", "Invalid avatar", err.to_string())
}
