// prose-pod-server-api
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use serde::Deserialize;

#[derive(Debug)]
#[derive(Deserialize)]
pub struct AppConfig {
    pub auth: AuthConfig,
    pub server: ServerConfig,
    pub service_accounts: ServiceAccountsConfig,
    pub teams: TeamsConfig,
}

pub use auth::*;
pub mod auth {
    use serde::Deserialize;
    use tokio::time::Duration;

    #[derive(Debug)]
    #[derive(Deserialize)]
    pub struct AuthConfig {
        #[serde(with = "crate::util::serde::iso8601_duration")]
        pub token_ttl: Duration,
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

    impl TeamsConfig {
        pub const MAIN_TEAM_GROUP_ID: &'static str = "team";
        pub const DEFAULT_MAIN_TEAM_NAME: &'static str = "Team";
    }

    impl Default for TeamsConfig {
        fn default() -> Self {
            Self {
                main_team_name: Self::DEFAULT_MAIN_TEAM_NAME.to_owned(),
            }
        }
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
