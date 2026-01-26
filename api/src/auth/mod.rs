// prose-pod-server
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

mod auth_service;

pub use self::models::*;
pub mod models {
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
        pub primary_role: String,
    }

    impl CallerInfo {
        pub fn is_admin(&self) -> bool {
            let admin_roles = [
                "prosody:operator",
                "prosody:admin",
            ];
            admin_roles.contains(&self.primary_role.as_str())
        }
    }

    // MARK: Boilerplate

    impl std::ops::Deref for AuthToken {
        type Target = SecretString;

        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    impl AuthToken {
        pub fn inner(&self) -> &SecretString {
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
