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
