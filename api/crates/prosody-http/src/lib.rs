// prosody-http-rs
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

#[cfg(feature = "mod_http_admin_api")]
pub mod mod_http_admin_api;
#[cfg(feature = "mod_http_oauth2")]
pub mod mod_http_oauth2;
mod util;

#[cfg(feature = "secrecy")]
pub use secrecy;

pub use self::error::{Error, ProsodyHttpError};
#[cfg(feature = "mod_http_admin_api")]
pub use self::mod_http_admin_api as admin_api;
#[cfg(feature = "mod_http_oauth2")]
pub use self::mod_http_oauth2 as oauth2;

#[derive(Debug)]
pub struct ProsodyHttpConfig {
    pub url: String,
}

#[cfg(not(feature = "secrecy"))]
pub type Secret = str;
#[cfg(feature = "secrecy")]
pub type Secret = secrecy::SecretString;

#[cfg(not(feature = "time"))]
pub type Timestamp = u32;
#[cfg(feature = "time")]
pub type Timestamp = time::OffsetDateTime;

pub mod error {
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
    pub struct ProsodyHttpError<ExtraInfo = Option<DefaultExtraInfo>> {
        error: ProsodyHttpErrorDetails<ExtraInfo>,
        pub code: u16,
    }

    impl<T> ProsodyHttpError<T> {
        #[inline]
        pub fn into_inner(self) -> T {
            self.error.extra
        }
    }

    /// See [`ProsodyHttpError`].
    #[derive(Debug, Deserialize)]
    pub struct ProsodyHttpErrorDetails<ExtraInfo> {
        pub source: Box<str>,
        pub text: Box<str>,
        pub condition: Box<str>,
        pub r#type: Box<str>,
        pub extra: ExtraInfo,
    }

    pub use ProsodyHttpErrorDefaultExtraInfo as DefaultExtraInfo;

    /// This is what `util/error.lua` sends as `extra` in errors
    /// by default in certain cases. It’s not always present, therefore
    /// one should always use `Option<ProsodyHttpErrorDefaultExtraInfo>`
    /// if not using a custom type, [`serde_json::Value`] or `()`.
    #[derive(Debug, Deserialize)]
    pub struct ProsodyHttpErrorDefaultExtraInfo {
        /// E.g. `"https://prosody.im/protocol/errors"`.
        pub namespace: Box<str>,

        /// E.g. `"user-not-found"`.
        pub condition: Box<str>,
    }

    // MARK: - Boilerplate

    impl<T> std::ops::Deref for ProsodyHttpError<T> {
        type Target = ProsodyHttpErrorDetails<T>;

        #[inline]
        fn deref(&self) -> &Self::Target {
            &self.error
        }
    }
}
