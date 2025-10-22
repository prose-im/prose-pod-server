// prose-pod-server-api
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};

impl IntoResponse for crate::models::Password {
    fn into_response(self) -> Response {
        use secrecy::ExposeSecret as _;

        let bytes = axum::body::Bytes::copy_from_slice(self.expose_secret().as_bytes());
        let body = axum::body::Body::from(bytes);
        Response::new(body)
    }
}

impl IntoResponse for crate::models::AuthToken {
    fn into_response(self) -> Response {
        use secrecy::ExposeSecret as _;

        let bytes = axum::body::Bytes::copy_from_slice(self.expose_secret().as_bytes());
        let body = axum::body::Body::from(bytes);
        Response::new(body)
    }
}

impl IntoResponse for &crate::state::AppStatus {
    fn into_response(self) -> Response {
        use crate::errors;
        use crate::state::AppStatus as Status;
        use crate::util::ResponseExt as _;

        match self {
            Status::Starting => errors::too_early(
                "SERVER_STARTING",
                "A moment please",
                "Your Prose Server is starting.",
            )
            .into_response()
            .retry_after(1),
            Status::Running => StatusCode::OK.into_response(),
            Status::Restarting => errors::too_early(
                "SERVER_RESTARTING",
                "A moment please",
                "Your Prose Server is restarting.",
            )
            .into_response()
            .retry_after(1),
            Status::RestartFailed(err) => errors::internal_server_error(
                err,
                "RESTART_FAILED",
                "Something went wrong while restarting your Prose Server.",
            )
            .into_response(),
            Status::Misconfigured(err) => errors::bad_configuration(err).into_response(),
            Status::UndergoingFactoryReset => errors::service_unavailable(
                "FACTORY_RESET_IN_PROGRESS",
                "Factory reset in progress",
                "Come back in a few moments to find your brand-new Prose Workspace.",
            )
            .into_response()
            // FIXME: Test if this value makes sense.
            .retry_after(15),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Error {
    /// Error kind (to group error codes).
    ///
    /// MUST be “UPPER_SNAKE_CASE” and MUST end with `_ERROR`.
    ///
    /// E.g. "AUTH_ERROR".
    kind: &'static str,

    /// Error kind (to group error codes).
    ///
    /// MUST be “UPPER_SNAKE_CASE”.
    ///
    /// E.g. "FORBIDDEN".
    code: &'static str,

    status: StatusCode,

    message: String,

    description: String,
}

impl Error {
    #[must_use]
    #[inline]
    pub fn new(
        kind: &'static str,
        code: &'static str,
        status: StatusCode,
        message: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        let message: String = message.into();
        let description: String = description.into();

        // Check values.
        {
            use crate::util::{debug_assert_or_log_error, is_upper_snake_case};

            debug_assert_or_log_error(
                !kind.is_empty(),
                format!("Invalid error kind '{kind}': Cannot be empty."),
            );
            debug_assert_or_log_error(
                kind.bytes().all(is_upper_snake_case),
                format!("Invalid error kind '{kind}': Only `[A-Z_]` allowed."),
            );

            const KIND_SUFFIX: &'static str = "_ERROR";
            debug_assert_or_log_error(
                kind.ends_with(KIND_SUFFIX),
                format!("Invalid error kind '{kind}': Missing '{KIND_SUFFIX}' suffix."),
            );

            debug_assert_or_log_error(
                !code.is_empty(),
                format!("Invalid error code '{code}': Cannot be empty."),
            );
            debug_assert_or_log_error(
                code.bytes().all(is_upper_snake_case),
                format!("Invalid error code '{code}': Only `[A-Z_]` allowed."),
            );

            debug_assert_or_log_error(
                !message.is_empty(),
                format!("Error message cannot be empty."),
            );
            debug_assert_or_log_error(
                !message.ends_with("."),
                format!("Error message cannot end with a period (`.`)."),
            );

            debug_assert_or_log_error(
                !description.is_empty(),
                format!("Error description cannot be empty."),
            );
        }

        Self {
            kind,
            code,
            status,
            message,
            description,
        }
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        use serde_json::json;

        // Destructure to ensure we don’t forget fields.
        let Self {
            kind,
            code,
            status,
            message,
            description,
        } = self;

        let body = json!({
            "error": true,
            "kind": kind,
            "code": code,
            "message": message,
            "description": description,
        });

        (status, axum::Json(body)).into_response()
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.message, f)
    }
}
