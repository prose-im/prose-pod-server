// prose-pod-server
//
// Copyright: 2024–2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! Utilities.

mod cache;
mod proxy;
mod rw_lock_guards;
pub mod serde;
pub mod tracing_subscriber_ext;

pub use cache::Cache;
pub use proxy::proxy;
pub use rw_lock_guards::OptionRwLockReadGuard;

#[must_use]
#[inline]
pub const fn is_upper_snake_case(b: u8) -> bool {
    b.is_ascii_uppercase() || b == b'_' || b.is_ascii_digit()
}

pub fn unix_timestamp() -> u64 {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

/// Equivalent of [`debug_assert!`] but still
/// logs an error message in release mode.
macro_rules! debug_assert_or_log_error {
    ($cond:expr, $($args:tt)*) => {
        if cfg!(debug_assertions) {
            assert!($cond, "[debug_only] {}", format!($($args)*));
        } else if !$cond {
            tracing::error!($($args)*);
        }
    };
}
pub(crate) use debug_assert_or_log_error;

/// [`panic!`] in debug mode, [`tracing::error!`] in release.
macro_rules! debug_panic_or_log_error {
    ($($args:tt)*) => {
        if cfg!(debug_assertions) {
            panic!("[debug_only] {}", format!($($args)*));
        } else {
            tracing::error!($($args)*);
        }
    };
}
pub(crate) use debug_panic_or_log_error;

pub trait ResponseExt {
    fn retry_after(self, seconds: u8) -> Self;
}

impl ResponseExt for axum::response::Response {
    fn retry_after(mut self, seconds: u8) -> Self {
        use axum::http::HeaderValue;

        self.headers_mut().append(
            "Retry-After",
            HeaderValue::from_str(seconds.to_string().as_str()).unwrap(),
        );

        self
    }
}

#[inline]
pub fn empty_dir(path: impl AsRef<std::path::Path>) -> std::io::Result<()> {
    use std::fs;

    for entry in fs::read_dir(path)? {
        let entry = entry?;

        if entry.file_type()?.is_dir() {
            tracing::trace!("Deleting directory `{}`…", entry.path().display());
            fs::remove_dir_all(entry.path())?;
        } else {
            tracing::trace!("Deleting file `{}`…", entry.path().display());
            fs::remove_file(entry.path())?;
        }
    }

    Ok(())
}

pub fn append_path_segment(
    uri: &axum::http::Uri,
    segment: &str,
) -> Result<axum::http::Uri, axum::http::Error> {
    use axum::http::uri::PathAndQuery;

    let mut path = uri.path().to_owned();

    if !path.ends_with('/') && !segment.starts_with('/') {
        path.push('/');
    }
    path.push_str(segment);

    let new_path_and_query = match uri.query() {
        Some(query) => format!("{path}?{query}"),
        None => path,
    };

    axum::http::uri::Builder::from(uri.to_owned())
        .path_and_query(PathAndQuery::from_maybe_shared(new_path_and_query)?)
        .build()
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
    pub fn random_string_alphanumeric(length: usize) -> String {
        use rand::{Rng as _, distr::Alphanumeric};

        // NOTE: Code taken from <https://rust-lang-nursery.github.io/rust-cookbook/algorithms/randomness.html#create-random-passwords-from-a-set-of-alphanumeric-characters>.
        rand::rng()
            .sample_iter(&Alphanumeric)
            .take(length)
            .map(char::from)
            .collect::<String>()
    }

    /// Generates a random ID (essentially a random alphanumeric string)
    #[must_use]
    #[inline]
    pub fn random_id(length: usize) -> String {
        self::random_string_alphanumeric(length)
    }

    #[must_use]
    #[inline]
    pub fn random_bytes<const N: usize>() -> [u8; N] {
        use rand::RngCore as _;

        let mut buf = [0u8; N];
        rand::rng().fill_bytes(&mut buf);
        buf
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

        super::rand::random_string_alphanumeric(length).into()
    }

    /// Generates a very strong random password.
    // FIXME: Use a wider character set.
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

pub mod either {
    #[derive(Debug)]
    pub enum Either<E1, E2> {
        E1(E1),
        E2(E2),
    }
}

pub mod sync {
    use tokio_util::sync::CancellationToken;

    #[derive(Debug)]
    #[repr(transparent)]
    pub struct AutoCancelToken(pub CancellationToken);

    impl AutoCancelToken {
        #[allow(unused)]
        pub fn new() -> Self {
            Self(CancellationToken::new())
        }

        #[allow(unused)]
        pub fn token(&self) -> &CancellationToken {
            &self.0
        }
    }

    impl Drop for AutoCancelToken {
        fn drop(&mut self) {
            tracing::debug!("[Drop] Dropping `AutoCancelToken`…");
            self.0.cancel();
        }
    }
}

// MARK: - Error helpers

pub const PROSODY_JIDS_ARE_VALID: &'static str = "JIDs coming from Prosody should always be valid";

/// NOTE: Inspired by [`anyhow::Context`].
pub trait Context<Res> {
    fn context(self, internal_error_code: &'static str, public_description: &str) -> Res;
}

impl<T, E1: Context<E2>, E2> Context<Result<T, E2>> for Result<T, E1> {
    fn context(self, internal_error_code: &'static str, public_description: &str) -> Result<T, E2> {
        match self {
            Ok(val) => Ok(val),
            Err(err) => Err(err.context(internal_error_code, public_description)),
        }
    }
}

impl Context<crate::responders::Error> for prosody_http::oauth2::Error {
    fn context(
        self,
        internal_error_code: &'static str,
        public_description: &str,
    ) -> crate::responders::Error {
        use crate::errors;

        match self {
            Self::Unauthorized(_) => errors::unauthorized(
                "Try logging in again, then ask an administrator if it persists.",
            ),
            Self::Forbidden(_) => errors::forbidden("You cannot do that."),
            Self::Internal(err) => {
                errors::internal_server_error(&err, internal_error_code, public_description)
            }
            Self::Other(err) => {
                errors::internal_server_error(&err, internal_error_code, public_description)
            }
        }
    }
}

impl Context<crate::responders::Error> for anyhow::Error {
    fn context(
        self,
        internal_error_code: &'static str,
        public_description: &str,
    ) -> crate::responders::Error {
        Context::context(&self, internal_error_code, public_description)
    }
}

impl Context<crate::responders::Error> for &anyhow::Error {
    fn context(
        self,
        internal_error_code: &'static str,
        public_description: &str,
    ) -> crate::responders::Error {
        crate::errors::internal_server_error(&self, internal_error_code, public_description)
    }
}

impl Context<crate::responders::Error> for std::io::Error {
    fn context(
        self,
        internal_error_code: &'static str,
        public_description: &str,
    ) -> crate::responders::Error {
        crate::errors::internal_server_error(
            &anyhow::Error::new(self),
            internal_error_code,
            public_description,
        )
    }
}

impl Context<crate::responders::Error> for reqwest::Error {
    fn context(
        self,
        internal_error_code: &'static str,
        public_description: &str,
    ) -> crate::responders::Error {
        crate::errors::internal_server_error(
            &anyhow::Error::new(self),
            internal_error_code,
            public_description,
        )
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

impl<E2> NoContext<E2> for &anyhow::Error
where
    for<'a> &'a anyhow::Error: Context<E2>,
{
    fn no_context(self) -> E2 {
        Context::context(self, crate::errors::ERROR_CODE_INTERNAL, "Internal error")
    }
}

impl<T, E1, E2> NoContext<Result<T, E2>> for Result<T, E1>
where
    E1: Context<E2>,
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
