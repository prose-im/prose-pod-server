// prosody-rest-rs
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

pub(crate) trait RequestBuilderExt {
    fn bearer_auth(self, token: &secrecy::SecretString) -> Self;
}

impl<T> RequestBuilderExt for ureq::RequestBuilder<T> {
    fn bearer_auth(self, token: &secrecy::SecretString) -> Self {
        use ureq::http::header::AUTHORIZATION;

        let token = secrecy::ExposeSecret::expose_secret(token);

        self.header(AUTHORIZATION, format!("Bearer {token}"))
    }
}
