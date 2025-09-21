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
        message: String,
        description: impl Into<String>,
    ) -> Self {
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
