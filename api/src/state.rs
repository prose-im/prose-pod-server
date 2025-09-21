// prose-pod-server-api
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::{collections::HashMap, sync::Arc};

use prosody_http::mod_http_oauth2::ProsodyOAuth2Client;
use prosodyctl::Prosodyctl;
use tokio::sync::RwLock;

use crate::{
    AppConfig,
    models::{BareJid, JidDomain, Password},
};

#[derive(Debug, Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub prosodyctl: Arc<RwLock<Prosodyctl>>,
    pub service_accounts_credentials: Arc<ServiceAccountsCredentials>,
    pub oauth2_client: Arc<ProsodyOAuth2Client>,
}

#[derive(Debug)]
#[repr(transparent)]
pub struct ServiceAccountsCredentials(HashMap<BareJid, Password>);

impl ServiceAccountsCredentials {
    pub fn new(config: &crate::config::ServiceAccountsConfig, server_domain: &JidDomain) -> Self {
        let mut data: HashMap<BareJid, Password> = HashMap::with_capacity(1);

        data.insert(
            config.prose_workspace_jid(server_domain),
            Password::random(),
        );

        Self(data)
    }
}

// MARK: - Boilerplate

impl std::ops::Deref for ServiceAccountsCredentials {
    type Target = HashMap<BareJid, Password>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl axum::extract::FromRef<AppState> for Arc<AppConfig> {
    fn from_ref(app_state: &AppState) -> Self {
        app_state.config.clone()
    }
}
