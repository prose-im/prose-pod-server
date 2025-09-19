// prose-pod-server-api
//
// Copyright: 2024–2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

//! Utilities.

pub use secrets::*;

pub mod secrets {
    use secrecy::SecretString;

    /// Generates a random secret string.
    #[inline]
    pub fn random_secret(length: usize) -> SecretString {
        use rand::{Rng as _, distr::Alphanumeric};

        assert!(length >= 16);

        // NOTE: Code taken from <https://rust-lang-nursery.github.io/rust-cookbook/algorithms/randomness.html#create-random-passwords-from-a-set-of-alphanumeric-characters>.
        rand::rng()
            .sample_iter(&Alphanumeric)
            .take(length)
            .map(char::from)
            .collect::<String>()
            .into()
    }

    /// Generates a very strong random password.
    #[inline]
    pub fn strong_random_password() -> SecretString {
        // 256 characters because why not.
        self::random_secret(256)
    }
}
