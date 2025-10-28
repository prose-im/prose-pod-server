// prosody-http-rs
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use crate::Password;

pub(crate) trait RequestBuilderExt {
    fn basic_auth(self, username: &str, password: &Password) -> Self;
    fn bearer_auth(self, token: &Password) -> Self;
}

impl<T> RequestBuilderExt for ureq::RequestBuilder<T> {
    fn basic_auth(self, username: &str, password: &Password) -> Self {
        use base64::prelude::{BASE64_STANDARD, Engine as _};
        use ureq::http::header::AUTHORIZATION;

        #[cfg(feature = "secrecy")]
        let password = secrecy::ExposeSecret::expose_secret(password);

        self.header(
            AUTHORIZATION,
            format!(
                "Basic {}",
                BASE64_STANDARD.encode(format!("{username}:{password}"))
            ),
        )
    }

    fn bearer_auth(self, token: &Password) -> Self {
        use ureq::http::header::AUTHORIZATION;

        #[cfg(feature = "secrecy")]
        let token = secrecy::ExposeSecret::expose_secret(token);

        self.header(AUTHORIZATION, format!("Bearer {token}"))
    }
}

#[cfg(any(feature = "time"))]
pub mod serde {
    use serde::{Deserialize as _, Deserializer, Serialize as _, Serializer};

    #[cfg(feature = "time")]
    pub mod time {
        use super::*;

        /// `time::Duration` as whole seconds.
        pub mod duration {
            use ::time::Duration;

            use super::*;

            #[inline]
            pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
            where
                D: Deserializer<'de>,
            {
                i64::deserialize(deserializer).map(Duration::seconds)
            }

            /// `Option<time::Duration>` as whole seconds.
            pub mod option {
                use super::*;

                #[inline]
                pub fn serialize<S: Serializer>(
                    option: &Option<Duration>,
                    serializer: S,
                ) -> Result<S::Ok, S::Error> {
                    option.map(Duration::whole_seconds).serialize(serializer)
                }
            }
        }
    }
}
