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

pub(crate) static CONFIG_FILE_PATH: LazyLock<PathBuf> =
    LazyLock::new(|| (Path::new(API_CONFIG_DIR).join(CONFIG_FILE_NAME)).to_path_buf());

pub mod defaults {
    pub(super) const SERVER_HTTP_PORT: u16 = 5280;

    pub(super) const SERVER_API_PORT: u16 = 8080;

    pub(super) const SERVER_LOCAL_HOSTNAME: &'static str = "prose-pod-server";

    pub const MAIN_TEAM_GROUP_ID: &'static str = "team";

    pub(super) const DEFAULT_MAIN_TEAM_NAME: &'static str = "Team";
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("Invalid '{CONFIG_FILE_NAME}' configuration file: {0}")]
#[repr(transparent)]
pub struct InvalidConfiguration(figment::Error);

fn default_config_static() -> Figment {
    use self::defaults::*;
    use figment::providers::{Format as _, Toml};
    use secrecy::{ExposeSecret as _, SecretString};
    use toml::toml;

    let default_log_format = if cfg!(debug_assertions) {
        "pretty"
    } else {
        "json"
    };
    let default_log_timer = if cfg!(debug_assertions) {
        "uptime"
    } else {
        "time"
    };

    let true_in_debug = cfg!(debug_assertions);

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

        [log]
        level = "info"
        format = default_log_format
        timer = default_log_timer
        with_file = true_in_debug
        with_target = true
        with_thread_ids = false
        with_line_number = true_in_debug
        with_span_events = false
        with_thread_names = false

        [log.opentelemetry]
        enabled = true_in_debug

        [service_accounts.prose_workspace]
        xmpp_node = "prose-workspace"
    }
    .to_string();

    Figment::from(Toml::string(&static_defaults))
}

fn with_dynamic_defaults(figment: Figment) -> Result<Figment, InvalidConfiguration> {
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

    pub fn from_figment(figment: Figment) -> Result<Self, InvalidConfiguration> {
        with_dynamic_defaults(figment)?
            .extract()
            .map_err(InvalidConfiguration)
    }

    #[allow(unused)]
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, InvalidConfiguration> {
        Self::from_figment(Self::figment_at_path(path))
    }

    pub fn from_default_figment() -> Result<Self, InvalidConfiguration> {
        Self::from_figment(Self::figment())
    }
}

impl AppConfig {
    /// When reloading the configuration at runtime, validate that the changes
    /// can be applied. For example, one cannot change the server domain at
    /// runtime.
    ///
    /// Note that this function doesn’t always return errors, sometimes only
    /// printing a warning to avoid unnecessary downtime.
    pub(crate) fn validate_config_changes(
        old_config: &AppConfig,
        new_config: &AppConfig,
    ) -> Result<(), anyhow::Error> {
        use anyhow::anyhow;

        if new_config.server.domain != old_config.server.domain {
            // TODO: Support domain migrations.
            return Err(anyhow!(
                "Once set, the server domain cannot be changed. \
                Such migrations are planned, but are not a priority. \
                If that’s a feature you need, contact us at <https://prose.org/contact/>. \
                Until then, you need to perform a factory reset or host a second \
                Prose instance if you want to use another domain."
            ));
        }

        if new_config.server_api.address() != old_config.server_api.address() {
            // TODO: Support frontend restarts.
            tracing::warn!(
                "The Prose Pod Server API address cannot be changed at runtime. \
                You need to restart the Prose Pod Server for this change to be effective. \
                If you need no-downtime restarts, contact us at <https://prose.org/contact/>."
            );
        }

        // NOTE: We can’t reload logging layers because of a bug in `tracing`
        //   (see https://github.com/tokio-rs/tracing/issues/1629). Until
        //   `tracing` 0.4 is released, we’ll just reload the filters, and
        //   require a restart to change the logging format. This shouldn’t
        //   bother anyone, as there is little point in changing the format
        //   of logs at runtime. Filters, on the other hand, might be changed
        //   at runtime and need to be reloadable.
        #[derive(PartialEq, Eq)]
        struct StaticLogConfig {
            pub format: LogFormat,
            pub timer: LogTimer,
            pub with_file: bool,
            pub with_target: bool,
            pub with_thread_ids: bool,
            pub with_line_number: bool,
            pub with_span_events: bool,
            pub with_thread_names: bool,
            pub opentelemetry_enabled: bool,
        }
        impl From<&LogConfig> for StaticLogConfig {
            fn from(value: &LogConfig) -> Self {
                let LogConfig {
                    level: _level,
                    format,
                    timer,
                    with_file,
                    with_target,
                    with_thread_ids,
                    with_line_number,
                    with_span_events,
                    with_thread_names,
                    opentelemetry,
                } = value;
                Self {
                    format: *format,
                    timer: *timer,
                    with_file: *with_file,
                    with_target: *with_target,
                    with_thread_ids: *with_thread_ids,
                    with_line_number: *with_line_number,
                    with_span_events: *with_span_events,
                    with_thread_names: *with_thread_names,
                    opentelemetry_enabled: opentelemetry.enabled,
                }
            }
        }
        if StaticLogConfig::from(&new_config.log) != StaticLogConfig::from(&old_config.log) {
            let static_keys = [
                "format",
                "timer",
                "with_file",
                "with_target",
                "with_thread_ids",
                "with_line_number",
                "with_span_events",
                "with_thread_names",
                "opentelemetry.enabled",
            ];
            tracing::warn!(
                "For technical reasons, logging configuration keys related to \
                formatting (`{static_keys}`) need a restart to be applied. \
                You need to restart the Prose Pod Server for this change to be effective. \
                If this bothers you, feel free to contact us at <https://prose.org/contact/>.",
                static_keys = static_keys.join("`, `")
            );
        }

        Ok(())
    }
}

#[derive(Debug)]
#[derive(Deserialize)]
pub(crate) struct AppConfig {
    pub auth: AuthConfig,
    pub log: LogConfig,
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

    use crate::{app_config::LogLevel, models::JidDomain};

    #[derive(Debug)]
    #[serde_with::serde_as]
    #[derive(Deserialize)]
    pub struct ServerConfig {
        #[serde_as(as = "serde_with::DisplayFromStr")]
        pub domain: JidDomain,

        pub local_hostname: String,

        pub http_port: u16,

        pub log_level: LogLevel,
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

pub use log::*;
pub mod log {
    use serde::Deserialize;

    #[derive(Debug)]
    #[derive(Deserialize)]
    pub struct LogConfig {
        pub level: LogLevel,

        pub format: LogFormat,

        pub timer: LogTimer,

        pub with_file: bool,

        pub with_target: bool,

        pub with_thread_ids: bool,

        pub with_line_number: bool,

        pub with_span_events: bool,

        pub with_thread_names: bool,

        pub opentelemetry: OpenTelemetryConfig,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[derive(serde_with::SerializeDisplay, serde_with::DeserializeFromStr)]
    #[derive(strum::Display, strum::EnumString)]
    #[strum(serialize_all = "snake_case")]
    pub enum LogLevel {
        Trace,
        Debug,
        Info,
        Warn,
        Error,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[derive(serde_with::SerializeDisplay, serde_with::DeserializeFromStr)]
    #[derive(strum::Display, strum::EnumString)]
    #[strum(serialize_all = "snake_case")]
    pub enum LogFormat {
        Full,
        Compact,
        Json,
        Pretty,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[derive(serde_with::SerializeDisplay, serde_with::DeserializeFromStr)]
    #[derive(strum::Display, strum::EnumString)]
    #[strum(serialize_all = "snake_case")]
    pub enum LogTimer {
        None,
        Time,
        Uptime,
    }

    #[derive(Debug)]
    #[derive(Deserialize)]
    pub struct OpenTelemetryConfig {
        pub enabled: bool,
    }

    // MARK: Conversions

    impl From<&LogLevel> for tracing::Level {
        fn from(level: &LogLevel) -> Self {
            match level {
                LogLevel::Trace => Self::TRACE,
                LogLevel::Debug => Self::DEBUG,
                LogLevel::Info => Self::INFO,
                LogLevel::Warn => Self::WARN,
                LogLevel::Error => Self::ERROR,
            }
        }
    }

    impl From<&LogFormat> for init_tracing_opentelemetry::LogFormat {
        fn from(format: &LogFormat) -> Self {
            match format {
                LogFormat::Full => Self::Full,
                LogFormat::Compact => Self::Compact,
                LogFormat::Json => Self::Json,
                LogFormat::Pretty => Self::Pretty,
            }
        }
    }

    impl From<&LogTimer> for init_tracing_opentelemetry::LogTimer {
        fn from(format: &LogTimer) -> Self {
            match format {
                LogTimer::None => Self::None,
                LogTimer::Time => Self::Time,
                LogTimer::Uptime => Self::Uptime,
            }
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
    #[serde_with::serde_as]
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct ServiceAccountConfig {
        #[serde_as(as = "serde_with::DisplayFromStr")]
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
