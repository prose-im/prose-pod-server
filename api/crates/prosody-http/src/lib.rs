// prosody-http-rs
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

#[cfg(feature = "mod_http_oauth2")]
pub mod mod_http_oauth2;
mod util;

#[cfg(feature = "secrecy")]
pub use secrecy;

pub use self::error::{Error, ProsodyHttpError};
#[cfg(feature = "mod_http_oauth2")]
pub use self::mod_http_oauth2 as oauth2;

#[derive(Debug)]
pub struct ProsodyHttpConfig {
    pub url: String,
}

#[cfg(not(feature = "secrecy"))]
pub type Password = str;
#[cfg(feature = "secrecy")]
pub type Password = secrecy::SecretString;

pub mod error {
    use std::sync::Arc;

    use serde::Deserialize;

    pub use self::ProsodyHttpError as Error;

    /// E.g.
    ///
    /// ```json
    /// {
    ///   "error": {
    ///     "source": "http_admin_api",
    ///     "text": "User not found",
    ///     "condition": "item-not-found",
    ///     "type": "cancel",
    ///     "extra": {
    ///       "namespace": "https://prosody.im/protocol/errors",
    ///       "condition": "user-not-found"
    ///     }
    ///   },
    ///   "type": "error",
    ///   "code": 500
    /// }
    /// ```
    #[derive(Debug, Deserialize, thiserror::Error)]
    #[error("{reason}", reason = error.text)]
    pub struct ProsodyHttpError<ExtraInfo = serde_json::Value> {
        error: ProsodyHttpErrorDetails<ExtraInfo>,
        pub code: u16,
    }

    impl<T> ProsodyHttpError<T> {
        pub fn into_inner(self) -> T {
            self.error.extra
        }
    }

    /// See [`ProsodyHttpError`].
    ///
    /// NOTE: Using `Arc` instead of `Box` even though type is not `Clone`
    ///   to keep it `Send + Sync`.
    #[derive(Debug, Deserialize)]
    pub struct ProsodyHttpErrorDetails<ExtraInfo> {
        pub source: Arc<str>,
        pub text: Arc<str>,
        pub condition: Arc<str>,
        pub r#type: Arc<str>,
        pub extra: ExtraInfo,
    }

    // MARK: - Boilerplate

    impl<T> std::ops::Deref for ProsodyHttpError<T> {
        type Target = ProsodyHttpErrorDetails<T>;

        fn deref(&self) -> &Self::Target {
            &self.error
        }
    }
}
