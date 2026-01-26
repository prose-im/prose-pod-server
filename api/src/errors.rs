// prose-pod-server
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

pub mod prelude {
    pub use axum::http::StatusCode;

    #[allow(unused)]
    pub(crate) use crate::errors;
    pub use crate::responders::Error;
}

use anyhow::anyhow;
use prelude::*;

// NOTE: I(@RemiBardon) don’t like that we have functions named as HTTP status
//   code. It’d be better if it was domain-specific or even better if errors
//   were created inline. However I don’t like having to import the `StatusCode`
//   type everywhere just for this, hence the helpers. Once
//   [hyperium/http#761 “Make `StatusCode::from_u16` const”](https://github.com/hyperium/http/pull/761)
//   gets released, we will have the opportunity to pass an integer directly,
//   without having to construct a non-`const` `StatusCode` at runtime.
//   We could also use macros to pass the identifier directly, but they
//   would be accessible via `crate::` and not `crate::errors::` which
//   I’m not a fan of either.

// MARK: Internal errors

/// NOTE: Not public to “force” the usage of [`internal_server_error`].
const ERROR_KIND_INTERNAL: &'static str = "INTERNAL_ERROR";
/// To use when we don’t want to leak internal info.
pub const ERROR_CODE_INTERNAL: &'static str = "INTERNAL_ERROR";

/// `public_description` is a short user-facing description.
/// It MUST NOT leak any internal information.
///
/// It will be sent as “{public_description} (logged as error_id={error_id})”.
#[must_use]
#[inline]
pub fn internal_server_error(
    error: &anyhow::Error,
    code: &'static str,
    public_description: impl AsRef<str>,
) -> Error {
    Error::new(
        ERROR_KIND_INTERNAL,
        code,
        StatusCode::INTERNAL_SERVER_ERROR,
        "Internal server error",
        auto_log(error, public_description),
    )
}

#[must_use]
#[inline]
pub fn missing_configuration(config_key: &'static str) -> Error {
    use crate::app_config::CONFIG_FILE_NAME;

    self::internal_server_error(
        &anyhow!(
            "Missing key `{config_key}` in the app configuration. \
            Add it to `{CONFIG_FILE_NAME}` or use environment variables."
        ),
        "MISSING_CONFIGURATION",
        "Your Prose Server is misconfigured. \
        Contact an administrator to fix this.",
    )
}

// MARK: Auth errors

const ERROR_KIND_AUTH: &'static str = "AUTH_ERROR";

#[must_use]
#[inline]
pub fn unauthorized(description: impl AsRef<str>) -> Error {
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
pub fn forbidden(description: impl AsRef<str>) -> Error {
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
pub fn too_early(
    code: &'static str,
    message: impl AsRef<str>,
    description: impl AsRef<str>,
) -> Error {
    Error::new(
        ERROR_KIND_LIFECYCLE,
        code,
        StatusCode::TOO_EARLY,
        message,
        description,
    )
}

#[must_use]
#[inline]
pub fn service_unavailable(
    code: &'static str,
    message: impl AsRef<str>,
    description: impl AsRef<str>,
) -> Error {
    Error::new(
        ERROR_KIND_LIFECYCLE,
        code,
        StatusCode::SERVICE_UNAVAILABLE,
        message,
        description,
    )
}

/// `public_description` is a short user-facing description.
/// It MUST NOT leak any internal information.
///
/// It will be sent as “{public_description} (logged as error_id={error_id})”.
#[must_use]
#[inline]
pub fn service_unavailable_err(
    error: &anyhow::Error,
    code: &'static str,
    message: impl AsRef<str>,
    public_description: impl AsRef<str>,
) -> Error {
    Error::new(
        ERROR_KIND_LIFECYCLE,
        code,
        StatusCode::SERVICE_UNAVAILABLE,
        message,
        auto_log(error, public_description),
    )
}

#[must_use]
#[inline]
pub fn too_late(
    code: &'static str,
    message: impl AsRef<str>,
    description: impl AsRef<str>,
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
    message: impl AsRef<str>,
    description: impl AsRef<str>,
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
    message: impl AsRef<str>,
    description: impl AsRef<str>,
) -> Error {
    Error::new(
        "VALIDATION_ERROR",
        code,
        StatusCode::BAD_REQUEST,
        message,
        description,
    )
}

// MARK: - Helpers

#[must_use]
#[inline]
fn auto_log(error: &anyhow::Error, public_description: impl AsRef<str>) -> String {
    let public_description: &str = public_description.as_ref();

    // Log error debug information with a unique ID,
    // and reference this ID in the user-facing description.
    let error_id = crate::util::random_id(8);
    tracing::error!(%error_id, "{error:?}");

    format!("{public_description} (logged as error_id={error_id})")
}
