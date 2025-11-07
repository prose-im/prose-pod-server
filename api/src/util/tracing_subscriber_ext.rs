// prose-pod-server-api
//
// Copyright: 2022–2025, David Bernard <david.bernard.31@gmail.com> (via <https://github.com/davidB/tracing-opentelemetry-instrumentation-sdk>)
// Copyright: 2025, Rémi Bardon <remi@remibardon.name>
// License: Mozilla Public License v2.0 (MPL v2.0)
// Inspired by: https://github.com/davidB/tracing-opentelemetry-instrumentation-sdk/blob/f53cdecfbfe1eca6ebfb307212e5e51fc0bca677/init-tracing-opentelemetry/src/tracing_subscriber_ext.rs#L106

use std::collections::HashMap;

use init_tracing_opentelemetry::{
    Error, opentelemetry_sdk::trace::SdkTracerProvider,
    tracing_subscriber_ext::build_tracer_layer as build_otel_layer,
};
use tracing::{Level, Subscriber};
use tracing_subscriber::{
    EnvFilter, Layer, Registry,
    filter::{FilterExt, filter_fn},
    fmt::format::FmtSpan,
    layer::{Context, Filter, Layered, SubscriberExt},
    registry::LookupSpan,
    reload,
};

use crate::app_config::{LogConfig, LogLevel};

type BoxedLayer<T> = Box<dyn Layer<T> + Send + Sync + 'static>;
type BoxedFilter<T> = Box<dyn Filter<T> + Send + Sync + 'static>;

#[must_use]
fn build_filter_layer<S>(log_level: &LogLevel) -> BoxedFilter<S>
where
    S: Subscriber,
    for<'span> S: LookupSpan<'span>,
{
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

    let not_prosody_filter = filter_fn(|meta| meta.target() != "prosody");
    let filter = EnvFilter::builder().parse_lossy(&directives.join(","));

    not_prosody_filter.and(filter).boxed()
}

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

type BoxLayered<T> = Layered<BoxedLayer<T>, T>;

/// Slight modification of
/// `init_tracing_opentelemetry::tracing_subscriber_ext::init_subscribers`
/// to support dynamic reloading of the log level filter.
///
/// NOTE: Can only be called once.
pub fn init_tracing(
    log_config: &LogConfig,
    server_log_level: LogLevel,
) -> Result<
    (
        SdkTracerProvider,
        TracingReloadHandles<BoxLayered<Registry>, BoxLayered<BoxLayered<Registry>>>,
    ),
    Error,
> {
    // Setup a temporary subscriber to log output during setup.
    let _guard = {
        let layer = init_tracing_opentelemetry::TracingConfig::from(log_config)
            .with_otel(false)
            .build_layer()?
            .with_filter(build_filter_layer(&log_config.level));
        let subscriber = tracing_subscriber::registry().with(layer);
        tracing::subscriber::set_default(subscriber)
    };
    tracing::info!("Init logging & tracing.");

    let (otel_layer, guard) = build_otel_layer()?;
    let otel_layer = otel_layer
        .with_filter(build_filter_layer(&log_config.level))
        .boxed();

    let (prosody_layer, prosody_layer_handle) = {
        let layer = init_tracing_opentelemetry::TracingConfig::from(log_config)
            .with_thread_names(false)
            .with_file_names(false)
            .with_line_numbers(false)
            .build_layer()?
            .with_filter(ProsodyFilter::new(&server_log_level))
            .boxed();

        reload::Layer::new(layer)
    };

    let (server_api_layer, server_api_layer_handle) = {
        let layer = init_tracing_opentelemetry::TracingConfig::from(log_config)
            .build_layer()?
            .with_filter(build_filter_layer(&log_config.level))
            .boxed();

        reload::Layer::new(layer)
    };

    let subscriber = tracing_subscriber::registry()
        .with(otel_layer)
        .with(server_api_layer.boxed())
        .with(prosody_layer.boxed());
    tracing::subscriber::set_global_default(subscriber)?;

    Ok((
        guard,
        TracingReloadHandles {
            server_api: server_api_layer_handle,
            prosody: prosody_layer_handle,
        },
    ))
}

// MARK: - Filtering

struct ProsodyFilter {
    directives: HashMap<String, Level>,
    default_level: Level,
}

impl ProsodyFilter {
    #[must_use]
    fn new(log_level: &LogLevel) -> Self {
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

        let default_level = tracing::Level::from(log_level);

        ProsodyFilter {
            directives,
            default_level,
        }
    }
}

impl<S> Filter<S> for ProsodyFilter {
    fn enabled(&self, meta: &tracing::Metadata<'_>, _cx: &Context<'_, S>) -> bool {
        meta.target() == "prosody"
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

        if let Some(ref module) = visitor.module {
            let level = event.metadata().level();
            let allowed = self.directives.get(module).unwrap_or(&self.default_level);

            // NOTE: Level ordering is counter-intuitive:
            debug_assert!(Level::INFO < Level::DEBUG);
            level <= allowed
        } else {
            false
        }
    }
}

// MARK: - Reload

pub struct TracingReloadHandles<S1, S2> {
    pub server_api: reload::Handle<BoxedLayer<S1>, S1>,
    pub prosody: reload::Handle<BoxedLayer<S2>, S2>,
}

impl<S1, S2> Clone for TracingReloadHandles<S1, S2> {
    fn clone(&self) -> Self {
        Self {
            server_api: self.server_api.clone(),
            prosody: self.prosody.clone(),
        }
    }
}

/// NOTE: Can be called multiple times.
pub fn update_tracing_config(
    log_config: &LogConfig,
    tracing_reload_handles: &TracingReloadHandles<
        impl Subscriber + for<'a> LookupSpan<'a>,
        impl Subscriber,
    >,
) {
    tracing::info!("Updating global tracing configuration…");
    let TracingReloadHandles {
        server_api: filter,
        prosody: logger,
    } = tracing_reload_handles;
    // filter
    //     .modify(|filter| *filter = build_filter_layer(&log_config.level))
    //     .inspect_err(|err| tracing::warn!("Error when updating global log level filter: {err}"))
    //     .unwrap_or_default();
    // logger
    //     .modify(|logger| *logger = build_logger_layer(log_config))
    //     .inspect_err(|err| tracing::warn!("Error when updating global logger: {err}"))
    //     .unwrap_or_default();
}
