// prose-pod-server-api
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::sync::Arc;

use prosody_child_process::ProsodyChildProcess;
use prosody_http::mod_http_oauth2::ProsodyOAuth2Client;
use prosody_rest::ProsodyRest;
use prosodyctl::Prosodyctl;
use tokio::sync::RwLock;

use crate::{AppConfig, secrets_service::SecretsService};

#[derive(Debug, Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub prosody: Arc<RwLock<ProsodyChildProcess>>,
    pub prosodyctl: Arc<RwLock<Prosodyctl>>,
    pub prosody_rest: ProsodyRest,
    pub oauth2_client: Arc<ProsodyOAuth2Client>,
    pub secrets_service: SecretsService,
}

// MARK: - Boilerplate

impl axum::extract::FromRef<AppState> for Arc<AppConfig> {
    fn from_ref(app_state: &AppState) -> Self {
        app_state.config.clone()
    }
}
