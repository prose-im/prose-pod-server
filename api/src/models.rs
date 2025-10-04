// prose-pod-server-api
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

pub use jid::*;
pub mod jid {
    // MARK: Bare JID

    // TODO: Parse `BareJid`.
    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    #[derive(serde_with::DeserializeFromStr, serde_with::SerializeDisplay)]
    #[repr(transparent)]
    pub struct BareJid(String);

    impl BareJid {
        pub fn new(node: &JidNode, domain: &JidDomain) -> Self {
            Self(format!("{node}@{domain}"))
        }

        pub fn node(&self) -> JidNode {
            let marker_idx = self.0.find("@").expect("A bare JID should contain a ‘@’");
            JidNode(self.0[..marker_idx].to_owned())
        }

        pub fn domain(&self) -> JidDomain {
            let marker_idx = self.0.find("@").expect("A bare JID should contain a ‘@’");
            JidDomain(self.0[(marker_idx + 1)..].to_owned())
        }
    }

    impl std::str::FromStr for BareJid {
        type Err = &'static str;

        fn from_str(string: &str) -> Result<Self, Self::Err> {
            if !string.contains("@") {
                Err("Missing '@'.")
            } else if string.contains("/") {
                Err("Resource part not permitted.")
            } else {
                Ok(Self(string.to_owned()))
            }
        }
    }

    impl std::ops::Deref for BareJid {
        type Target = str;

        fn deref(&self) -> &Self::Target {
            self.0.as_str()
        }
    }

    impl std::fmt::Display for BareJid {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            std::fmt::Display::fmt(&self.0, f)
        }
    }

    // MARK: JID node

    // TODO: Parse `JidNode`.
    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    #[derive(serde_with::DeserializeFromStr, serde_with::SerializeDisplay)]
    #[repr(transparent)]
    pub struct JidNode(String);

    impl std::str::FromStr for JidNode {
        type Err = &'static str;

        fn from_str(string: &str) -> Result<Self, Self::Err> {
            if string.contains("@") {
                Err("'@' not permitted.")
            } else if string.contains("/") {
                Err("'/' not permitted.")
            } else {
                Ok(Self(string.to_owned()))
            }
        }
    }

    impl std::ops::Deref for JidNode {
        type Target = str;

        fn deref(&self) -> &Self::Target {
            self.0.as_str()
        }
    }

    impl std::fmt::Display for JidNode {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            std::fmt::Display::fmt(&self.0, f)
        }
    }

    // MARK: JID domain

    // TODO: Parse `JidDomain`.
    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    #[derive(serde_with::DeserializeFromStr, serde_with::SerializeDisplay)]
    #[repr(transparent)]
    pub struct JidDomain(String);

    impl std::str::FromStr for JidDomain {
        type Err = &'static str;

        fn from_str(string: &str) -> Result<Self, Self::Err> {
            if string.contains("@") {
                Err("'@' not permitted.")
            } else if string.contains("/") {
                Err("'/' not permitted.")
            } else {
                Ok(Self(string.to_owned()))
            }
        }
    }

    impl std::ops::Deref for JidDomain {
        type Target = str;

        fn deref(&self) -> &Self::Target {
            self.0.as_str()
        }
    }

    impl std::fmt::Display for JidDomain {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            std::fmt::Display::fmt(&self.0, f)
        }
    }
}

pub use password::*;
pub mod password {
    use secrecy::SecretString;

    use crate::errors::ERROR_KIND_VALIDATION;

    #[derive(Debug, Clone)]
    #[derive(serde_with::DeserializeFromStr)]
    #[repr(transparent)]
    pub struct Password(SecretString);

    impl Password {
        pub const MIN_PASSWORD_LENGTH: usize = 16;

        // NOTE: Not just in `Default` to allow more explicit code.
        pub fn random() -> Self {
            Self(crate::util::random_strong_password())
        }
    }

    // NOTE: Allows public creation while keeping `.0` private.
    impl TryFrom<SecretString> for Password {
        type Error = crate::responders::Error;

        fn try_from(secret: SecretString) -> Result<Self, Self::Error> {
            use secrecy::ExposeSecret as _;

            if secret.expose_secret().len() >= Self::MIN_PASSWORD_LENGTH {
                Ok(Self(secret))
            } else {
                Err(crate::responders::Error::new(
                    ERROR_KIND_VALIDATION,
                    "PASSWORD_TOO_SHORT",
                    axum::http::StatusCode::BAD_REQUEST,
                    "Password too short",
                    format!(
                        "Passwords must be at least {min_length} characters long.",
                        min_length = Self::MIN_PASSWORD_LENGTH
                    ),
                ))
            }
        }
    }

    // NOTE: Allows access to `.0` (e.g. to wrap in another type) but without
    //   allowing mutability (which would bypass the minimum password length).
    impl From<Password> for SecretString {
        fn from(password: Password) -> Self {
            password.0
        }
    }

    impl Default for Password {
        fn default() -> Self {
            Self::random()
        }
    }

    impl std::ops::Deref for Password {
        type Target = SecretString;

        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    impl std::str::FromStr for Password {
        type Err = <Self as TryFrom<SecretString>>::Error;

        fn from_str(str: &str) -> Result<Self, Self::Err> {
            Self::try_from(SecretString::from(str))
        }
    }
}

pub use auth::*;
pub mod auth {
    use secrecy::SecretString;
    use serde::Serialize;

    use crate::models::BareJid;

    #[derive(Debug, Clone)]
    #[repr(transparent)]
    pub struct AuthToken(pub SecretString);

    /// Information about who made the API call.
    #[derive(Debug, Clone)]
    #[derive(Serialize)]
    pub struct CallerInfo {
        pub jid: BareJid,
        #[serde(rename = "role")]
        pub primary_role: String,
    }

    // MARK: Boilerplate

    impl std::ops::Deref for AuthToken {
        type Target = SecretString;

        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    impl<T> From<T> for AuthToken
    where
        SecretString: From<T>,
    {
        fn from(value: T) -> Self {
            Self(SecretString::from(value))
        }
    }
}
