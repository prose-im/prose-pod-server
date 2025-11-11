// prose-pod-server-api
//
// Copyright: 2022–2025, David Bernard <david.bernard.31@gmail.com> (via <https://github.com/davidB/tracing-opentelemetry-instrumentation-sdk>)
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)
// Inspired by: https://github.com/davidB/tracing-opentelemetry-instrumentation-sdk/blob/f53cdecfbfe1eca6ebfb307212e5e51fc0bca677/init-tracing-opentelemetry/src/tracing_subscriber_ext.rs#L106

use std::collections::HashMap;

use init_tracing_opentelemetry::opentelemetry_sdk::trace::SdkTracerProvider;
use tracing::{Subscriber, level_filters::LevelFilter, subscriber::Interest};
use tracing_subscriber::{
    EnvFilter, Layer, Registry,
    filter::Filtered,
    fmt::format::FmtSpan,
    layer::{Context, Filter, Layered, SubscriberExt},
    registry::LookupSpan,
    reload,
};

use crate::app_config::{LogConfig, LogLevel};

type BoxedLayer<T> = Box<dyn Layer<T> + Send + Sync + 'static>;

/// NOTE: Can only be called once. Later, use [`update_tracing_config`].
pub fn init_tracing(
    log_config: &LogConfig,
    server_log_level: &LogLevel,
) -> Result<(SdkTracerProvider, TracingReloadHandles<Registry>), init_tracing_opentelemetry::Error>
{
    use init_tracing_opentelemetry::tracing_subscriber_ext::build_tracer_layer as build_otel_layer;

    let server_api_filter = ServerApiFilter::new(&log_config.level);

    // Setup a temporary subscriber to log output during setup.
    let temp_log_guard = {
        let server_api_layer = build_server_api_logger(log_config)?;
        let subscriber = tracing_subscriber::registry()
            .with(server_api_layer.with_filter(server_api_filter.clone()));
        tracing::subscriber::set_default(subscriber)
    };
    tracing::info!("Init logging & tracing.");

    let (otel_layer, otel_guard) = build_otel_layer()?;

    let (server_api_layer, server_api_layer_handle) = {
        let layer = build_server_api_logger(log_config)?;
        reload::Layer::new(layer.with_filter(server_api_filter.clone()))
    };

    let (prosody_layer, prosody_layer_handle) = {
        let layer = build_prosody_logger(log_config)?;
        reload::Layer::new(layer.with_filter(ProsodyFilter::new(server_log_level)))
    };

    let subscriber = tracing_subscriber::registry()
        .with(server_api_layer)
        .with(prosody_layer)
        .with(otel_layer);
    tracing::subscriber::set_global_default(subscriber)?;
    drop(temp_log_guard);

    Ok((
        otel_guard,
        TracingReloadHandles {
            server_api: server_api_layer_handle,
            prosody: prosody_layer_handle,
        },
    ))
}

fn build_server_api_logger<S>(
    log_config: &LogConfig,
) -> Result<BoxedLayer<S>, init_tracing_opentelemetry::Error>
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    let layer = init_tracing_opentelemetry::TracingConfig::from(log_config).build_layer()?;
    Ok(layer)
}

fn build_prosody_logger<S>(
    log_config: &LogConfig,
) -> Result<BoxedLayer<S>, init_tracing_opentelemetry::Error>
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    let layer = init_tracing_opentelemetry::TracingConfig::from(log_config)
        .with_thread_names(false)
        .with_file_names(false)
        .with_line_numbers(false)
        .build_layer()?;
    Ok(layer)
}

// MARK: - Filtering

#[derive(Debug, Clone)]
pub struct ServerApiFilter {
    env_filter: EnvFilter,
}

impl ServerApiFilter {
    fn is_enabled(&self, meta: &tracing::Metadata<'_>) -> bool {
        meta.target() != "prosody"
    }
}

impl<S> Filter<S> for ServerApiFilter {
    fn enabled(&self, meta: &tracing::Metadata<'_>, cx: &Context<'_, S>) -> bool {
        self.is_enabled(meta) && Filter::enabled(&self.env_filter, meta, cx)
    }

    fn callsite_enabled(&self, meta: &'static tracing::Metadata<'static>) -> Interest {
        if self.is_enabled(meta) {
            Interest::sometimes()
        } else {
            Interest::never()
        }
    }

    fn event_enabled(&self, event: &tracing::Event<'_>, cx: &Context<'_, S>) -> bool {
        Filter::event_enabled(&self.env_filter, event, cx)
    }

    fn max_level_hint(&self) -> Option<LevelFilter> {
        self.env_filter.max_level_hint()
    }
}

impl ServerApiFilter {
    #[must_use]
    fn new(log_level: &LogLevel) -> Self {
        // NOTE: Last values take precedence in directives (i.e.
        //   `trace,info` logs >`info`, while `info,trace` logs >`trace`),
        //   so important values must be added last.
        let mut directives: Vec<String> = vec![];

        // TODO: Support enabling wire-level `ureq` tracing via debug_only conf.
        // TODO: Prohibit usage of wire-level `ureq` tracing in release builds.

        directives.extend(
            match log_level {
                LogLevel::Trace => vec![
                    "trace",
                    "ureq_proto::util=debug", // `trace` enables wire-level tracing
                    "ureq::unversioned=debug",
                    "ureq_proto::client=debug",
                    "ureq::pool=debug",
                    "h2=debug",
                    "hyper_util::client=debug",
                    "tower=debug",
                ],
                LogLevel::Debug => vec![
                    "debug",
                    "prose_pod_server_api=debug",
                    "prosodyctl=debug",
                    "ureq=debug",
                    "ureq_proto::util=debug",
                    "ureq::unversioned=info",
                    "ureq_proto::client=info",
                    "ureq::pool=info",
                    "h2=info",
                    "hyper_util::client=info",
                    "tower=info",
                ],
                LogLevel::Info => vec!["info"],
                LogLevel::Warn => vec!["warn"],
                LogLevel::Error => vec!["error"],
            }
            .into_iter()
            .map(ToOwned::to_owned),
        );

        directives.extend(std::env::var("RUST_LOG").ok().into_iter());

        // NOTE: `otel::tracing` must be at level info to emit OTel traces & spans.
        directives.push("otel::tracing=trace".to_owned());

        Self {
            env_filter: EnvFilter::builder().parse_lossy(&directives.join(",")),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProsodyFilter {
    directives: HashMap<String, tracing::Level>,
    default_level: tracing::Level,
}

impl ProsodyFilter {
    #[must_use]
    fn new(log_level: &LogLevel) -> Self {
        use tracing::Level;

        // NOTE: Last values take precedence in directives (i.e.
        //   `trace,info` logs >`info`, while `info,trace` logs >`trace`),
        //   so important values must be added last.
        let mut directives: HashMap<String, tracing::Level> = HashMap::new();

        // NOTE: `rostermanager` is very verbose in debug mode. We’ll use the
        //   Prosody-unknown “trace” level to reduce its verbosity.
        directives.extend(
            match log_level {
                LogLevel::Trace => vec![("rostermanager", Level::DEBUG)],
                LogLevel::Debug => vec![("rostermanager", Level::INFO)],
                LogLevel::Info => vec![],
                LogLevel::Warn => vec![],
                LogLevel::Error => vec![],
            }
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v)),
        );

        ProsodyFilter {
            directives,
            default_level: log_level.into(),
        }
    }
}

impl ProsodyFilter {
    fn is_enabled(&self, meta: &tracing::Metadata<'_>) -> bool {
        meta.target() == "prosody"
    }
}

impl<S> Filter<S> for ProsodyFilter {
    fn enabled(&self, meta: &tracing::Metadata<'_>, _cx: &Context<'_, S>) -> bool {
        self.is_enabled(meta)
    }

    fn callsite_enabled(&self, meta: &'static tracing::Metadata<'static>) -> Interest {
        if self.is_enabled(meta) {
            Interest::sometimes()
        } else {
            Interest::never()
        }
    }

    fn event_enabled(&self, event: &tracing::Event<'_>, _cx: &Context<'_, S>) -> bool {
        struct Visitor {
            module: Option<String>,
        }

        impl tracing::field::Visit for Visitor {
            fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
                if field.name() == "module" {
                    self.module = Some(value.to_owned());
                }
            }

            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                if field.name() == "module" {
                    self.module = Some(format!("{value:?}"));
                }
            }
        }

        let mut visitor = Visitor { module: None };
        event.record(&mut visitor);

        let event_enabled = if let Some(ref module) = visitor.module {
            let level = event.metadata().level();
            let allowed = self.directives.get(module).unwrap_or(&self.default_level);

            // NOTE: Level ordering is counter-intuitive:
            debug_assert!(tracing::Level::INFO < tracing::Level::DEBUG);
            level <= allowed
        } else {
            false
        };

        event_enabled
    }

    fn max_level_hint(&self) -> Option<LevelFilter> {
        // NOTE: Level ordering is counter-intuitive:
        debug_assert!(tracing::Level::INFO < tracing::Level::DEBUG);

        let max_level = self.default_level.max(
            self.directives
                .iter()
                .map(|(_, level)| level)
                .reduce(std::cmp::max)
                .cloned()
                .unwrap_or(self.default_level),
        );

        Some(LevelFilter::from_level(max_level))
    }
}

// MARK: - Reload

pub struct TracingReloadHandles<S = Registry> {
    server_api: reload::Handle<Filtered<BoxedLayer<S>, ServerApiFilter, S>, S>,
    prosody: reload::Handle<
        Filtered<
            BoxedLayer<Layered<reload::Layer<Filtered<BoxedLayer<S>, ServerApiFilter, S>, S>, S>>,
            ProsodyFilter,
            Layered<reload::Layer<Filtered<BoxedLayer<S>, ServerApiFilter, S>, S>, S>,
        >,
        Layered<reload::Layer<Filtered<BoxedLayer<S>, ServerApiFilter, S>, S>, S>,
    >,
}

/// NOTE: Can be called multiple times.
pub fn update_tracing_config(
    log_config: &LogConfig,
    server_log_level: &LogLevel,
    tracing_reload_handles: &TracingReloadHandles<impl Subscriber + for<'a> LookupSpan<'a>>,
) -> Result<(), init_tracing_opentelemetry::Error> {
    tracing::info!("Updating global tracing configuration…");

    let TracingReloadHandles {
        server_api,
        prosody,
    } = tracing_reload_handles;

    let new_server_api_filter = ServerApiFilter::new(&log_config.level);
    server_api
        .modify(|layer| *layer.filter_mut() = new_server_api_filter)
        .unwrap_or_else(|err| {
            tracing::warn!("Error when updating global Server API logger: {err:?}");
        });

    let new_prosody_filter = ProsodyFilter::new(server_log_level);
    prosody
        .modify(|layer| *layer.filter_mut() = new_prosody_filter)
        .unwrap_or_else(|err| {
            tracing::warn!("Error when updating global Prosody logger: {err:?}");
        });

    Ok(())
}

// MARK: - Conversions

impl From<&LogConfig> for init_tracing_opentelemetry::TracingConfig {
    fn from(
        _log_config @ LogConfig {
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
        }: &LogConfig,
    ) -> Self {
        let config = Self::minimal()
            .with_format(format.into())
            .with_timer(timer.into())
            .with_file_names(*with_file)
            .with_target_display(*with_target)
            .with_thread_ids(*with_thread_ids)
            .with_thread_names(*with_thread_names)
            .with_line_numbers(*with_line_number)
            .with_span_events(if *with_span_events {
                FmtSpan::NEW | FmtSpan::CLOSE
            } else {
                FmtSpan::NONE
            })
            .with_otel(opentelemetry.enabled);

        config
    }
}

// MARK: - Boilerplate

impl<S> Clone for TracingReloadHandles<S> {
    #[inline(always)]
    fn clone(&self) -> Self {
        // NOTE: `#[derive(Clone)]` doesn’t work here,
        //   we have to do it manually :/
        Self {
            server_api: self.server_api.clone(),
            prosody: self.prosody.clone(),
        }
    }
}

impl<S> std::fmt::Debug for TracingReloadHandles<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TracingReloadHandles")
            .finish_non_exhaustive()
    }
}
