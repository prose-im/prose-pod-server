// prose-pod-server
//
// Copyright: 2025–2026, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::LazyLock,
};

use anyhow::anyhow;
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

#[derive(Debug, thiserror::Error)]
#[error("Invalid '{CONFIG_FILE_NAME}' configuration file: {0}")]
#[repr(transparent)]
pub struct InvalidConfiguration(anyhow::Error);

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

        [vendor_analytics]
        preset = "default"

        [vendor_analytics.presets.all]
        enabled = true
        // Product usage analytics
        usage.enabled = true
        usage.meta_user_count.enabled = true
        usage.pod_version.enabled = true
        usage.user_app_version.enabled = true
        usage.user_lang.enabled = true
        usage.user_platform.enabled = true
        // Acquisition analytics
        acquisition.enabled = true
        acquisition.pod_domain.enabled = true

        [vendor_analytics.presets.default]
        inherits = "all"
        // Make identifying data points opt-in
        acquisition.pod_domain.enabled = false

        [vendor_analytics.presets.gdpr]
        inherits = "default"
        // Disable all analytics events if the Prose Workspace has less than 20
        // users. After that, companies are forced to provide KYC information
        // and our per-seat billing system has to know the exact user count.
        min_cohort_size = 20
        // Limit locales to reduce identifiability.
        usage.user_lang.max_locales = 2

        [vendor_analytics.presets.lgpd]
        inherits = "gdpr"

        [proxy]
        cloud_api_url = "https://prose.org/_api/cloud"
        prose_files_url = "https://files.prose.org"
    }
    .to_string();

    Figment::from(Toml::string(&static_defaults))
}

fn with_dynamic_defaults(mut figment: Figment) -> Result<Figment, InvalidConfiguration> {
    use figment::providers::*;

    let server_domain = figment.extract_inner::<String>("server.domain")?;

    let PodAddress {
        domain: pod_domain,
        ipv4: pod_ipv4,
        ipv6: pod_ipv6,
        ..
    } = figment
        .extract_inner::<PodAddress>("pod.address")
        .unwrap_or_default();
    if (pod_ipv4, pod_ipv6) == (None, None) {
        // If no static address has been defined, add a default for the Pod domain.
        let default_server_domain = format!("prose.{server_domain}");
        figment = figment.join(Serialized::default(
            "pod.address.domain",
            &default_server_domain,
        ));

        // If possible, add a default for the Dashboard URL.
        let pod_domain = pod_domain.map_or(default_server_domain, |name| name.to_string());
        figment = figment.join(Serialized::default(
            "dashboard.url",
            format!("https://admin.{pod_domain}"),
        ));
    }

    // Apply analytics presets.
    if let Ok(preset_name) = figment.extract_inner::<String>("vendor_analytics.preset") {
        figment = apply_analytics_preset(preset_name.as_str(), figment)?;
    }

    // Apply analytics defaults.
    figment = apply_analytics_preset("default", figment)?;

    // Remove analytics presets (useless afterwards).
    // NOTE: `Figments` are additive by construction so we cannot remove a key.
    figment = figment.merge(("vendor_analytics.presets", ()));

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
            .map_err(InvalidConfiguration::from)
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
    pub dashboard: DashboardConfig,
    pub log: LogConfig,
    #[serde(default)]
    pub policies: PoliciesConfig,
    pub proxy: ProxyConfig,
    pub server: ServerConfig,
    pub server_api: ServerApiConfig,
    pub service_accounts: ServiceAccountsConfig,
    pub teams: TeamsConfig,
    pub vendor_analytics: VendorAnalyticsConfig,
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

use dashboard::*;
pub mod dashboard {
    use axum::http::Uri;
    use serde::Deserialize;

    #[derive(Debug)]
    #[serde_with::serde_as]
    #[derive(Deserialize)]
    pub struct DashboardConfig {
        #[serde_as(as = "serde_with::DisplayFromStr")]
        pub url: Uri,
    }

    impl super::AppConfig {
        pub fn dashboard_url(&self) -> &Uri {
            &self.dashboard.url
        }

        pub fn pod_api_url(&self) -> Result<Uri, anyhow::Error> {
            use anyhow::Context as _;

            crate::util::append_path_segment(&self.dashboard.url, "api")
                .context("Failed creating Pod API URL")
        }
    }
}

pub use policies::*;
pub mod policies {
    use serde::Deserialize;

    /// NOTE: Read optional values instead of having defaults to properly
    ///   interpret user intentions.
    #[derive(Debug, Default)]
    #[derive(Deserialize)]
    pub struct PoliciesConfig {
        #[serde(default)]
        pub auto_update_enabled: Option<bool>,
    }
}

pub use proxy::*;
pub mod proxy {
    use axum::http::Uri;
    use serde::Deserialize;

    #[derive(Debug)]
    #[serde_with::serde_as]
    #[derive(Deserialize)]
    pub struct ProxyConfig {
        #[serde_as(as = "serde_with::DisplayFromStr")]
        pub cloud_api_url: Uri,

        #[serde_as(as = "serde_with::DisplayFromStr")]
        pub prose_files_url: Uri,
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

pub use vendor_analytics::*;
pub mod vendor_analytics {
    use std::collections::HashSet;

    #[derive(Debug, PartialEq, Eq)]
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct VendorAnalyticsConfig {
        pub enabled: bool,

        pub preset: String,

        // NOTE: Just so `deny_unknown_fields` doesn’t complain about it
        //   but still catches other unknown keys. Also so `serde` generates
        //   correct error messages.
        #[serde(default)]
        pub presets: (),

        #[serde(default)]
        pub min_cohort_size: Option<u8>,

        pub usage: VendorAnalyticsUsageConfig,
        pub acquisition: VendorAnalyticsAcquisitionConfig,
        // pub performance: todo,
        // pub operations: todo,
        // pub security: todo,
        // pub crash_reports: todo,
        // pub diagnostics: todo,
    }

    #[derive(Debug, PartialEq, Eq)]
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct EnabledConfig {
        pub enabled: bool,
    }

    #[derive(Debug, PartialEq, Eq)]
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct VendorAnalyticsUsageConfig {
        pub enabled: bool,

        pub meta_user_count: EnabledConfig,

        pub pod_version: EnabledConfig,

        pub user_app_version: EnabledConfig,

        pub user_lang: UserLangConfig,

        pub user_platform: UserPlatformConfig,
    }

    #[derive(Debug, PartialEq, Eq)]
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct UserLangConfig {
        pub enabled: bool,

        #[serde(default)]
        pub max_locales: Option<usize>,
    }

    #[derive(Debug, PartialEq, Eq)]
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct UserPlatformConfig {
        pub enabled: bool,

        #[serde(default)]
        pub allow_list: Option<HashSet<String>>,

        #[serde(default)]
        pub deny_list: Option<HashSet<String>>,
    }

    #[derive(Debug, PartialEq, Eq)]
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct VendorAnalyticsAcquisitionConfig {
        pub enabled: bool,

        pub pod_domain: EnabledConfig,
    }

    #[cfg(test)]
    mod tests {
        use figment::providers::{Format, Toml};
        use toml::toml;

        use crate::app_config::*;

        #[test]
        fn test_analytics_defaults() {
            let minimal_config = config_from_toml(&toml! {
                [server]
                domain = "example.org"
            })
            .unwrap();

            let expected = VendorAnalyticsConfig {
                enabled: true,
                preset: "default".to_owned(),
                presets: (),
                min_cohort_size: None,
                usage: VendorAnalyticsUsageConfig {
                    enabled: true,
                    meta_user_count: EnabledConfig { enabled: true },
                    pod_version: EnabledConfig { enabled: true },
                    user_app_version: EnabledConfig { enabled: true },
                    user_lang: UserLangConfig {
                        enabled: true,
                        max_locales: None,
                    },
                    user_platform: UserPlatformConfig {
                        enabled: true,
                        allow_list: None,
                        deny_list: None,
                    },
                },
                acquisition: VendorAnalyticsAcquisitionConfig {
                    enabled: true,
                    pod_domain: EnabledConfig { enabled: false },
                },
            };

            assert_eq!(minimal_config.vendor_analytics, expected);
        }

        #[test]
        fn test_analytics_defaults_overridable() {
            let config = config_from_toml(&toml! {
                [server]
                domain = "example.org"
            })
            .unwrap();

            assert_eq!(config.vendor_analytics.usage.user_lang.enabled, true);

            let config = config_from_toml(&toml! {
                [server]
                domain = "example.org"

                [vendor_analytics]
                usage.user_lang.enabled = false
            })
            .unwrap();

            assert_eq!(config.vendor_analytics.usage.user_lang.enabled, false);
        }

        #[test]
        fn test_analytics_presets_override_defaults() {
            let config = config_from_toml(&toml! {
                [server]
                domain = "example.org"
            })
            .unwrap();

            assert_eq!(config.vendor_analytics.usage.user_lang.enabled, true);

            let config = config_from_toml(&toml! {
                [server]
                domain = "example.org"

                [vendor_analytics]
                preset = "test"

                [vendor_analytics.presets.test]
                usage.user_lang.enabled = false
            })
            .unwrap();

            assert_eq!(config.vendor_analytics.usage.user_lang.enabled, false);
        }

        /// To detect cycles, we remove already used presets. If `default`
        /// inherits the same preset as a custom one that’s used, we might
        /// encounter an error. This test ensures our code is written in a
        /// way that supports this case.
        #[test]
        fn test_analytics_presets_can_reuse_default_inherited_preset() {
            let config = config_from_toml(&toml! {
                [server]
                domain = "example.org"

                [vendor_analytics.presets.default]
                inherits = "all"
            })
            .unwrap();

            assert_eq!(config.vendor_analytics.usage.user_lang.enabled, true);

            let config = config_from_toml(&toml! {
                [server]
                domain = "example.org"

                [vendor_analytics.presets.default]
                inherits = "all"

                [vendor_analytics]
                preset = "test"

                [vendor_analytics.presets.test]
                inherits = "all"
                usage.user_lang.enabled = false
            })
            .unwrap();

            assert_eq!(config.vendor_analytics.usage.user_lang.enabled, false);
        }

        #[test]
        fn test_analytics_presets_overridable() {
            let config = config_from_toml(&toml! {
                [server]
                domain = "example.org"

                [vendor_analytics]
                preset = "gdpr"
            })
            .unwrap();

            assert_eq!(config.vendor_analytics.usage.user_lang.enabled, true);

            let config = config_from_toml(&toml! {
                [server]
                domain = "example.org"

                [vendor_analytics]
                preset = "gdpr"

                [vendor_analytics.presets.gdpr]
                usage.user_lang.enabled = false
            })
            .unwrap();

            assert_eq!(config.vendor_analytics.usage.user_lang.enabled, false);
        }

        #[test]
        fn test_analytics_presets_inheritance_nested() {
            let config = config_from_toml(&toml! {
                [server]
                domain = "example.org"

                [vendor_analytics.presets.default]
                usage.user_lang.max_locales = 1
            })
            .unwrap();

            assert_eq!(config.vendor_analytics.usage.user_platform.enabled, true);
            assert_eq!(config.vendor_analytics.usage.user_lang.max_locales, Some(1));

            let config = config_from_toml(&toml! {
                [server]
                domain = "example.org"

                [vendor_analytics.presets.default]
                usage.user_lang.max_locales = 3

                [vendor_analytics]
                preset = "custom2"

                [vendor_analytics.presets.custom2]
                inherits = "custom1"
                usage.user_lang.max_locales = 2

                [vendor_analytics.presets.custom1]
                usage.user_platform.enabled = false
                usage.user_lang.max_locales = 1
            })
            .unwrap();

            assert_eq!(config.vendor_analytics.usage.user_platform.enabled, false);
            assert_eq!(config.vendor_analytics.usage.user_lang.max_locales, Some(2));
        }

        #[test]
        fn test_error_on_unknown_analytics_key() {
            let res1 = config_from_toml(&toml! {
                [server]
                domain = "example.org"

                [vendor_analytics]
                foo = "bar"
            });

            assert_eq!(res1.err(), Some(r#"Invalid 'prose.toml' configuration file: unknown field: found `foo`, expected `one of `enabled`, `preset`, `presets`, `min_cohort_size`, `usage`, `acquisition`` for key "default.vendor_analytics.foo" in TOML source string"#.to_owned()));

            let res2 = config_from_toml(&toml! {
                [server]
                domain = "example.org"

                [vendor_analytics]
                usage = { foo = "bar" }
            });

            assert_eq!(res2.err(), Some(r#"Invalid 'prose.toml' configuration file: unknown field: found `foo`, expected `one of `enabled`, `meta_user_count`, `pod_version`, `user_app_version`, `user_lang`, `user_platform`` for key "default.vendor_analytics.usage.foo" in TOML source string"#.to_owned()));

            let res3 = config_from_toml(&toml! {
                [server]
                domain = "example.org"

                [vendor_analytics]
                acquisition = { foo = "bar" }
            });

            assert_eq!(res3.err(), Some(r#"Invalid 'prose.toml' configuration file: unknown field: found `foo`, expected ``enabled` or `pod_domain`` for key "default.vendor_analytics.acquisition.foo" in TOML source string"#.to_owned()));
        }

        #[test]
        fn test_error_on_unknown_analytics_preset() {
            let res1 = config_from_toml(&toml! {
                [server]
                domain = "example.org"

                [vendor_analytics]
                preset = "custom"
            });

            assert_eq!(res1.err(), Some(r#"Invalid 'prose.toml' configuration file: Invalid preset 'custom'. Expected one of: ["all", "default", "gdpr", "lgpd"]"#.to_owned()));

            let res1 = config_from_toml(&toml! {
                [server]
                domain = "example.org"

                [vendor_analytics]
                preset = "custom3"

                [vendor_analytics.presets.custom3]
                inherits = "custom2"

                [vendor_analytics.presets.custom2]
                inherits = "custom"
            });

            assert_eq!(res1.err(), Some(r#"Invalid 'prose.toml' configuration file: Invalid preset 'custom' inherited by 'custom3 -> custom2'. Expected one of: ["all", "default", "gdpr", "lgpd"] (removing cyclic references)"#.to_owned()));
        }

        #[inline]
        fn config_from_toml(toml: &toml::Table) -> Result<AppConfig, String> {
            let toml = toml::to_string(&toml).unwrap();

            let figment = default_config_static().merge(Toml::string(&toml));

            match AppConfig::from_figment(figment) {
                Ok(app_config) => Ok(app_config),
                Err(err) => Err(format!("{err:#}")),
            }
        }
    }
}

pub use pod::PodAddress;
pub mod pod {
    use serde::Deserialize;

    #[derive(Debug, Clone, Default)]
    #[derive(Deserialize)]
    pub struct PodAddress {
        pub domain: Option<String>,

        pub ipv4: Option<String>,

        pub ipv6: Option<String>,
    }
}

// MARK: - Helpers

#[must_use]
fn apply_analytics_preset(
    preset_name: &str,
    figment: Figment,
) -> Result<Figment, InvalidConfiguration> {
    use figment::providers::Serialized;

    let presets = figment
        .extract_inner::<HashMap<String, toml::Table>>("vendor_analytics.presets")
        .unwrap_or_default();

    fn get_preset(
        preset_name: &str,
        mut presets: HashMap<String, toml::Table>,
        mut stack: Vec<String>,
    ) -> Result<Figment, InvalidConfiguration> {
        // NOTE: Remove preset to avoid reference cycles.
        if let Some(mut preset) = presets.remove(preset_name) {
            if let Some(inherited_preset_name) = preset.remove("inherits") {
                let Some(inherited_preset_name) = inherited_preset_name.as_str() else {
                    return Err(InvalidConfiguration(anyhow!(
                        "Invalid preset '{preset_name}'. `inherits` should be a string."
                    )));
                };

                stack.push(preset_name.to_owned());
                let inherited_preset = get_preset(inherited_preset_name, presets, stack)?;
                Ok(Figment::from(Serialized::defaults(preset)).join(inherited_preset))
            } else {
                Ok(Figment::from(Serialized::defaults(preset)))
            }
        } else {
            let mut available_presets = presets.keys().collect::<Vec<_>>();
            available_presets.sort();
            if stack.is_empty() {
                Err(InvalidConfiguration(anyhow!(
                    "Invalid preset '{preset_name}'. \
                    Expected one of: {available_presets:?}"
                )))
            } else {
                Err(InvalidConfiguration(anyhow!(
                    "Invalid preset '{preset_name}' inherited by '{stack}'. \
                    Expected one of: {available_presets:?} (removing cyclic references)",
                    stack = stack.join(" -> ")
                )))
            }
        }
    }

    let preset = get_preset(preset_name, presets, Vec::new())?.extract::<toml::Table>()?;

    Ok(figment.join(Serialized::default("vendor_analytics", preset)))
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

impl From<figment::Error> for InvalidConfiguration {
    fn from(error: figment::Error) -> Self {
        Self(anyhow::Error::from(error))
    }
}
