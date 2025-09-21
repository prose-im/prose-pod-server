// prose-pod-server-api
//
// Copyright: 2024–2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! Utilities.

#[must_use]
#[inline]
pub const fn is_upper_snake_case(b: u8) -> bool {
    b.is_ascii_uppercase() || b == b'_'
}

/// Equivalent of [`debug_assert!`] but still
/// logs an error message in release mode.
pub fn debug_assert_or_log_error(cond: bool, msg: String) {
    if cfg!(debug_assertions) {
        assert!(cond, "{msg}");
    } else if !cond {
        tracing::error!(msg);
    }
}

// MARK: - Random generators

pub use rand::*;
pub mod rand {
    /// Generates a random string.
    ///
    /// WARN: Do not generate secrets with this function! Instead, use
    ///   [`crate::util::secrets::random_secret`].
    #[must_use]
    #[inline]
    pub fn random_string(length: usize) -> String {
        use rand::{Rng as _, distr::Alphanumeric};

        // NOTE: Code taken from <https://rust-lang-nursery.github.io/rust-cookbook/algorithms/randomness.html#create-random-passwords-from-a-set-of-alphanumeric-characters>.
        rand::rng()
            .sample_iter(&Alphanumeric)
            .take(length)
            .map(char::from)
            .collect::<String>()
    }
}

pub use secrets::*;
pub mod secrets {
    use secrecy::SecretString;

    /// Generates a random secret string.
    #[must_use]
    #[inline]
    pub fn random_secret(length: usize) -> SecretString {
        assert!(length >= 16);

        super::rand::random_string(length).into()
    }

    /// Generates a very strong random password.
    #[must_use]
    #[inline]
    pub fn random_strong_password() -> SecretString {
        // 256 characters because why not.
        self::random_secret(256)
    }

    pub fn random_oauth2_registration_key() -> SecretString {
        use rand::RngCore as _;

        let mut key = [0u8; 256];
        rand::rng().fill_bytes(&mut key);

        fn bytes_to_base64(bytes: &[u8]) -> String {
            use base64::Engine as _;
            base64::prelude::BASE64_STANDARD.encode(bytes)
        }

        SecretString::from(bytes_to_base64(&key))
    }
}

// MARK: - Error helpers

/// NOTE: Inspired by [`anyhow::Context`].
pub trait Context<Res> {
    fn context(
        self,
        internal_error_code: &'static str,
        public_description: impl Into<String>,
    ) -> Res;
}

impl<T, E1: Context<E2>, E2> Context<Result<T, E2>> for Result<T, E1> {
    fn context(
        self,
        internal_error_code: &'static str,
        public_description: impl Into<String>,
    ) -> Result<T, E2> {
        match self {
            Ok(val) => Ok(val),
            Err(err) => Err(err.context(internal_error_code, public_description)),
        }
    }
}

impl Context<crate::responders::Error> for prosody_http::Error {
    fn context(
        self,
        internal_error_code: &'static str,
        public_description: impl Into<String>,
    ) -> crate::responders::Error {
        use crate::errors;

        match self {
            Self::Unauthorized { reason } => errors::unauthorized(reason),
            Self::Forbidden { reason } => errors::forbidden(reason),
            Self::Internal(err) => {
                errors::internal_server_error(err, internal_error_code, public_description)
            }
        }
    }
}

impl Context<crate::responders::Error> for anyhow::Error {
    fn context(
        self,
        internal_error_code: &'static str,
        public_description: impl Into<String>,
    ) -> crate::responders::Error {
        use crate::errors;

        errors::internal_server_error(self, internal_error_code, public_description)
    }
}

/// For internal errors (where we don’t want to leak internal info).
///
/// NOTE: Not using `impl From<anyhow::Error> for crate::responders::Error`
///   to make conversions explicit (and to remind one that they should add
///   context for the user if possible).
pub trait NoContext<Res> {
    fn no_context(self) -> Res;
}

impl<T, E2> NoContext<Result<T, E2>> for Result<T, anyhow::Error>
where
    anyhow::Error: Context<E2>,
{
    fn no_context(self) -> Result<T, E2> {
        match self {
            Ok(val) => Ok(val),
            Err(err) => Err(Context::context(
                err,
                crate::errors::ERROR_CODE_INTERNAL,
                "Internal error",
            )),
        }
    }
}
