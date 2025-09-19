// prose-pod-server-api
//
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

#[derive(Debug)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub service_accounts: ServiceAccountsConfig,
    pub teams: TeamsConfig,
}

pub use server::*;
pub mod server {
    use crate::models::JidDomain;

    #[derive(Debug)]
    pub struct ServerConfig {
        pub domain: JidDomain,
    }
}

pub use service_accounts::*;
pub mod service_accounts {
    use crate::models::{BareJid, JidDomain, JidNode};

    #[derive(Debug)]
    pub struct ServiceAccountsConfig {
        prose_workspace_username: JidNode,
    }

    impl ServiceAccountsConfig {
        pub const PROSE_WORKSPACE_USERNAME: &'static str = "prose-workspace";

        pub fn prose_workspace_jid(&self, server_domain: &JidDomain) -> BareJid {
            BareJid::new(&self.prose_workspace_username, server_domain)
        }
    }

    impl Default for ServiceAccountsConfig {
        fn default() -> Self {
            use std::str::FromStr as _;

            Self {
                prose_workspace_username: JidNode::from_str(Self::PROSE_WORKSPACE_USERNAME)
                    .expect("The `PROSE_WORKSPACE_USERNAME` constant should be valid."),
            }
        }
    }
}

pub use teams::*;
pub mod teams {
    #[derive(Debug)]
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
