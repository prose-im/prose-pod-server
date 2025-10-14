// prose-pod-server-api
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::{
    path::{Path, PathBuf},
    sync::LazyLock,
};

use figment::Figment;
use serde::Deserialize;

pub const API_CONFIG_DIR: &'static str = "/etc/prose";
pub const CONFIG_FILE_NAME: &'static str = "prose.toml";

static CONFIG_FILE_PATH: LazyLock<PathBuf> =
    LazyLock::new(|| (Path::new(API_CONFIG_DIR).join(CONFIG_FILE_NAME)).to_path_buf());

pub mod defaults {
    pub(super) const SERVER_HTTP_PORT: u16 = 5280;

    pub(super) const SERVER_API_PORT: u16 = 8080;

    pub(super) const SERVER_LOCAL_HOSTNAME: &'static str = "prose-pod-server";

    pub const MAIN_TEAM_GROUP_ID: &'static str = "team";

    pub(super) const DEFAULT_MAIN_TEAM_NAME: &'static str = "Team";
}

// TODO: Remove default server values from here and use the ones defined in
//   `prose-pod-server` to avoid discrepancies.
fn default_config_static() -> Figment {
    use self::defaults::*;
    use figment::providers::{Format as _, Toml};
    use secrecy::{ExposeSecret as _, SecretString};
    use toml::toml;

    let random_oauth2_registration_key: SecretString =
        crate::util::random_oauth2_registration_key();
    let random_oauth2_registration_key: &str = random_oauth2_registration_key.expose_secret();

    let static_defaults = toml! {
        [teams]
        main_team_name = DEFAULT_MAIN_TEAM_NAME

        [auth]
        token_ttl = "PT3H"
        password_reset_token_ttl = "PT15M"
        invitation_ttl = "P1W"
        oauth2_registration_key = random_oauth2_registration_key

        [server]
        local_hostname = SERVER_LOCAL_HOSTNAME
        http_port = SERVER_HTTP_PORT
        log_level = "info"

        [server_api]
        address = "0.0.0.0"
        port = SERVER_API_PORT

        [service_accounts.prose_workspace]
        xmpp_node = "prose-workspace"
    }
    .to_string();

    Figment::from(Toml::string(&static_defaults))
}

fn with_dynamic_defaults(figment: Figment) -> anyhow::Result<Figment> {
    // NOTE: At the moment, the Server API doesn’t add dynamic defaults.

    Ok(figment)
}

impl AppConfig {
    pub fn figment() -> Figment {
        Self::figment_at_path(CONFIG_FILE_PATH.as_path())
    }

    pub fn figment_at_path(path: impl AsRef<Path>) -> Figment {
        use figment::providers::{Env, Format, Toml};

        // NOTE: See what's possible at <https://docs.rs/figment/latest/figment/>.
        default_config_static()
            .merge(Toml::file(path))
            .merge(Env::prefixed("PROSE_").split("__"))
    }

    pub fn from_figment(figment: Figment) -> anyhow::Result<Self> {
        use anyhow::Context as _;

        with_dynamic_defaults(figment)?
            .extract()
            .context(format!("Invalid '{CONFIG_FILE_NAME}' configuration file"))
    }

    #[allow(unused)]
    pub fn from_path(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        Self::from_figment(Self::figment_at_path(path))
    }

    pub fn from_default_figment() -> anyhow::Result<Self> {
        Self::from_figment(Self::figment())
    }
}

#[derive(Debug)]
#[derive(Deserialize)]
pub(crate) struct AppConfig {
    pub auth: AuthConfig,
    pub server: ServerConfig,
    pub server_api: ServerApiConfig,
    pub service_accounts: ServiceAccountsConfig,
    pub teams: TeamsConfig,
}

pub use auth::*;
pub mod auth {
    use secrecy::SecretString;
    use serde::Deserialize;
    use tokio::time::Duration;

    #[derive(Debug)]
    #[derive(Deserialize)]
    pub struct AuthConfig {
        #[serde(with = "crate::util::serde::iso8601_duration")]
        pub token_ttl: Duration,

        pub oauth2_registration_key: SecretString,
    }
}

pub use server::*;
pub mod server {
    use serde::Deserialize;

    use crate::models::JidDomain;

    #[derive(Debug)]
    #[derive(Deserialize)]
    pub struct ServerConfig {
        pub domain: JidDomain,

        pub local_hostname: String,

        pub http_port: u16,
    }

    impl ServerConfig {
        pub fn http_url(&self) -> String {
            format!("http://{}:{}", self.local_hostname, self.http_port)
        }
    }
}

pub use server_api::*;
pub mod server_api {
    use std::net::{IpAddr, SocketAddr};

    use serde::Deserialize;

    #[derive(Debug)]
    #[derive(Deserialize)]
    pub struct ServerApiConfig {
        /// IP address to serve on.
        pub address: IpAddr,

        /// Port to serve on.
        pub port: u16,
    }

    impl ServerApiConfig {
        pub fn address(&self) -> SocketAddr {
            SocketAddr::new(self.address, self.port)
        }
    }
}

pub use service_accounts::*;
pub mod service_accounts {
    use serde::Deserialize;

    use crate::models::{BareJid, JidDomain, JidNode, Password};

    use super::AppConfig;

    #[derive(Debug)]
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct ServiceAccountsConfig {
        pub prose_workspace: ServiceAccountConfig,
    }

    #[derive(Debug)]
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct ServiceAccountConfig {
        pub xmpp_node: JidNode,
        #[serde(default)]
        pub password: Option<Password>,
    }

    impl ServiceAccountsConfig {
        pub const PROSE_WORKSPACE_USERNAME: &'static str = "prose-workspace";

        pub fn prose_workspace_jid(&self, server_domain: &JidDomain) -> BareJid {
            BareJid::from_parts(Some(&self.prose_workspace.xmpp_node), server_domain)
        }
    }

    impl Default for ServiceAccountsConfig {
        fn default() -> Self {
            use std::str::FromStr as _;

            Self {
                prose_workspace: ServiceAccountConfig {
                    xmpp_node: JidNode::from_str(Self::PROSE_WORKSPACE_USERNAME)
                        .expect("The `PROSE_WORKSPACE_USERNAME` constant should be valid."),
                    password: None,
                },
            }
        }
    }

    impl AppConfig {
        pub fn workspace_jid(&self) -> BareJid {
            self.service_accounts
                .prose_workspace_jid(&self.server.domain)
        }
    }
}

pub use teams::*;
pub mod teams {
    use serde::Deserialize;

    #[derive(Debug)]
    #[derive(Deserialize)]
    pub struct TeamsConfig {
        pub main_team_name: String,
    }
}

// MARK: - Boilerplate

impl AsRef<ServiceAccountsConfig> for AppConfig {
    fn as_ref(&self) -> &ServiceAccountsConfig {
        &self.service_accounts
    }
}

impl AsRef<TeamsConfig> for AppConfig {
    fn as_ref(&self) -> &TeamsConfig {
        &self.teams
    }
}
