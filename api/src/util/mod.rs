// prose-pod-server-api
//
// Copyright: 2024–2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! Utilities.

mod cache;
mod rw_lock_guards;
pub mod serde;

pub use cache::Cache;
pub use rw_lock_guards::OptionRwLockReadGuard;

#[must_use]
#[inline]
pub const fn is_upper_snake_case(b: u8) -> bool {
    b.is_ascii_uppercase() || b == b'_'
}

/// Equivalent of [`debug_assert!`] but still
/// logs an error message in release mode.
#[inline(always)]
pub fn debug_assert_or_log_error(cond: bool, msg: String) {
    if cfg!(debug_assertions) {
        assert!(cond, "{msg}");
    } else if !cond {
        tracing::error!(msg);
    }
}

/// [`panic!`] in debug mode, [`tracing::error!`] in release.
#[inline(always)]
pub fn debug_panic_or_log_error(msg: impl AsRef<str>) {
    let msg = msg.as_ref();
    if cfg!(debug_assertions) {
        panic!("{msg}");
    } else {
        tracing::error!(msg);
    }
}

// TODO: Get rid of this.
/// `jid@0.12` introduces `serde` support for `NodePart`, which we need here.
/// `prose_xmpp` depends on `jid@0.11`, and we can’t easily bump because of
/// breaking changes in `jid@0.12`. For now we’ll do manual mapping here, and
/// we’ll get rid of this after we bump.
pub fn jid_0_12_to_jid_0_11(jid_0_12: &jid::BareJid) -> prosody_rest::BareJid {
    prosody_rest::BareJid::new(jid_0_12.as_str()).unwrap()
}

pub trait ResponseExt {
    fn retry_after(self, duration: u8) -> Self;
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

#[macro_export]
macro_rules! app_status_if_matching {
    ($pattern:pat) => {
        async move |State(ref app_state): State<Layer0AppState>| {
            use axum::response::IntoResponse;

            match app_state.status().as_ref() {
                status @ $pattern => status.into_response(),
                status => {
                    if cfg!(debug_assertions) {
                        unreachable!()
                    } else {
                        errors::internal_server_error(
                            &anyhow::anyhow!("Unexpected app status: {status}"),
                            ERROR_CODE_INTERNAL,
                            "Internal error",
                        )
                        .into_response()
                    }
                }
            }
        }
    };
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

// MARK: - Error helpers

pub const PROSODY_JIDS_ARE_VALID: &'static str = "JIDs coming from Prosody should always be valid";

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
                errors::internal_server_error(&err, internal_error_code, public_description)
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
        crate::errors::internal_server_error(&self, internal_error_code, public_description)
    }
}

impl Context<crate::responders::Error> for std::io::Error {
    fn context(
        self,
        internal_error_code: &'static str,
        public_description: impl Into<String>,
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

impl<E2> NoContext<E2> for anyhow::Error
where
    anyhow::Error: Context<E2>,
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

pub trait ResultPanic {
    fn debug_panic_or_log_error(self) -> Self;
}

impl<T> ResultPanic for Result<T, anyhow::Error> {
    fn debug_panic_or_log_error(self) -> Self {
        if cfg!(debug_assertions) {
            if let Err(err) = self.as_ref() {
                debug_panic_or_log_error(err.to_string());
            }
        }
        self
    }
}
