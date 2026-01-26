// prose-pod-server
//
// Copyright: 2024–2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

pub mod prelude {
    pub use std::sync::Arc;

    pub use secrecy::SecretString;

    pub use crate::{
        auth::{
            auth_service::{self, AuthServiceImpl},
            errors::{InvalidCredentials, PasswordResetTokenExpired, PasswordValidationError},
            models::{AuthToken, Password, PasswordResetRequestInfo, PasswordResetToken, UserInfo},
        },
        errors::{Forbidden, Unauthorized},
        invitations::InvitationContact,
        models::jid::{BareJid, NodeRef},
        util::either::{Either, Either3, Either4},
    };
}

use time::Duration;

pub use self::live_auth_service::{LiveAuthService, OAuth2ClientState};
use self::prelude::*;

#[derive(Debug, Clone)]
pub struct AuthService {
    pub implem: Arc<dyn AuthServiceImpl>,
}

#[inline]
pub fn validate_password(password: &Password, min_len: u8) -> Result<(), PasswordValidationError> {
    use secrecy::ExposeSecret as _;

    let len = password.expose_secret().len();
    if len < min_len as usize {
        return Err(PasswordValidationError::TooShort { min_len, len });
    };

    Ok(())
}

#[async_trait::async_trait]
pub trait AuthServiceImpl: std::fmt::Debug + Sync + Send {
    async fn get_user_info(
        &self,
        auth: &AuthToken,
    ) -> Result<UserInfo, Either<Unauthorized, anyhow::Error>>;

    async fn register_oauth2_client(&self) -> Result<(), anyhow::Error>;
}

mod live_auth_service {
    use anyhow::Context as _;
    use arc_swap::ArcSwap;
    use prosody_http::oauth2::{self, ProsodyOAuth2};

    use super::*;

    #[derive(Debug)]
    pub struct LiveAuthService {
        pub oauth2: Arc<ProsodyOAuth2>,
        pub oauth2_client: ArcSwap<OAuth2ClientState>,
    }

    #[derive(Debug)]
    pub enum OAuth2ClientState {
        Unregistered(oauth2::ClientConfig),
        Registered(oauth2::ClientCredentials),
    }

    #[async_trait::async_trait]
    impl AuthServiceImpl for LiveAuthService {
        async fn get_user_info(
            &self,
            auth: &AuthToken,
        ) -> Result<UserInfo, Either<Unauthorized, anyhow::Error>> {
            match self.server_api.users_util_self(auth).await {
                Ok(user_info) => Ok(user_info),
                Err(ProsePodServerError::Forbidden(Forbidden(msg))) => {
                    Err(Either::E1(Unauthorized(msg)))
                }
                Err(err) => Err(Either::E2(anyhow::Error::new(err))),
            }
        }

        async fn register_oauth2_client(&self) -> Result<(), anyhow::Error> {
            match self.oauth2_client.load().as_ref() {
                OAuth2ClientState::Unregistered(client_config) => {
                    let credentials = (self.oauth2)
                        .register(client_config)
                        .await?
                        .into_credentials();

                    self.oauth2_client
                        .store(Arc::new(OAuth2ClientState::Registered(credentials)));

                    tracing::debug!("Registered OAuth 2.0 client.");
                    Ok(())
                }
                OAuth2ClientState::Registered(_credentials) => {
                    // NOTE: Do not panic or log error, as this is expected
                    //   behavior if the client credentials have been passed
                    //   via configuration.
                    tracing::debug!("OAuth 2.0 client already registered.");
                    Ok(())
                }
            }
        }
    }
}

// MARK: - Boilerplate

impl std::ops::Deref for AuthService {
    type Target = Arc<dyn AuthServiceImpl>;

    fn deref(&self) -> &Self::Target {
        &self.implem
    }
}
