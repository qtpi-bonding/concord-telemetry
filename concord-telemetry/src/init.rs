use crate::{deadline::run_with_deadline, panic, Principal, PrincipalKind};
use opentelemetry::KeyValue;
use opentelemetry_otlp::{Protocol, WithExportConfig};
use opentelemetry_sdk::{logs::SdkLoggerProvider, trace::SdkTracerProvider, Resource};
use std::hash::{BuildHasher, Hasher};
use std::{fs::OpenOptions, io::IsTerminal, path::PathBuf, sync::Mutex, time::Duration};
use tracing_subscriber::{
    filter::{Directive, LevelFilter},
    layer::{Layer, SubscriberExt},
    registry::LookupSpan,
    util::SubscriberInitExt,
    EnvFilter,
};

pub const EVENT_TARGET: &str = "concord_telemetry::event";
/// Target of the gRPC trace spans (`grpc` feature).
pub(crate) const GRPC_TARGET: &str = "concord_telemetry::grpc";

pub struct Config {
    pub service_name: &'static str,
    pub service_version: &'static str,
    pub principal: Option<Principal>,
    pub local: LocalOutput,
    pub flush_deadline: Duration,
}

impl Config {
    pub fn new(service_name: &'static str, service_version: &'static str) -> Self {
        Self {
            service_name,
            service_version,
            principal: None,
            local: LocalOutput::Text,
            flush_deadline: Duration::from_secs(2),
        }
    }
}

pub enum LocalOutput {
    Text,
    Json,
    JsonFile(PathBuf),
    Off,
}

#[derive(Debug, thiserror::Error)]
pub enum InitError {
    #[error("a global tracing subscriber is already set")]
    AlreadyInitialized,
    #[error("OTLP exporter: {0}")]
    Exporter(String),
    #[error("opening the JSON log file: {0}")]
    JsonFile(#[source] std::io::Error),
}

pub struct Guard {
    logs: Option<SdkLoggerProvider>,
    traces: Option<SdkTracerProvider>,
    deadline: Duration,
}

impl Drop for Guard {
    fn drop(&mut self) {
        let logs = self.logs.take();
        let traces = self.traces.take();
        if logs.is_none() && traces.is_none() {
            return;
        }
        run_with_deadline(self.deadline, move || {
            if let Some(provider) = logs {
                let _ = provider.force_flush();
                let _ = provider.shutdown();
            }
            if let Some(provider) = traces {
                let _ = provider.force_flush();
                let _ = provider.shutdown();
            }
        });
    }
}

/// Crates the exporter itself logs through. Exporting their records would feed
/// every export back into the next one. EnvFilter matches a target by prefix,
/// so `hyper` also covers `hyper_util`. `tower` is left out on purpose: the
/// prefix would also take `tower_http`, a server's request log.
const EXPORTER_TARGETS: [&str; 10] = [
    "hyper",
    "hyper_util",
    "h2",
    "reqwest",
    "rustls",
    "opentelemetry",
    "opentelemetry_sdk",
    "opentelemetry_otlp",
    "opentelemetry_http",
    "opentelemetry_proto",
];

/// The one filter for every output: RUST_LOG (default `info`), domain events,
/// error records and the gRPC trace spans always on, and the exporter's own
/// crates off while exporting. A disabled gRPC span would silently break the
/// trace between services.
///
/// It is a global filter, never a per-layer one. tracing-subscriber's
/// per-layer filtering drops the next event on a thread after an `enabled!`
/// check that a per-layer filter refused, and sqlx makes such a check for
/// every query, so per-layer filters lost most of a service's records.
fn filter(exporting: bool) -> EnvFilter {
    let mut filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    filter = filter.add_directive(directive(EVENT_TARGET, LevelFilter::TRACE));
    filter = filter.add_directive(directive(GRPC_TARGET, LevelFilter::INFO));
    if exporting {
        for target in EXPORTER_TARGETS {
            filter = filter.add_directive(directive(target, LevelFilter::OFF));
        }
    }
    filter
}

fn directive(target: &str, level: LevelFilter) -> Directive {
    format!("{target}={level}")
        .parse()
        .expect("a target and level always form a directive")
}

fn resource(config: &Config) -> Resource {
    let mut attributes = vec![
        KeyValue::new("service.name", config.service_name),
        KeyValue::new("service.version", config.service_version),
        KeyValue::new(
            "service.instance.id",
            format!(
                "{:016x}",
                std::collections::hash_map::RandomState::new()
                    .build_hasher()
                    .finish()
            ),
        ),
    ];
    if let Some(principal) = &config.principal {
        let kind = PrincipalKind::try_from(principal.kind)
            .unwrap_or(PrincipalKind::Unspecified)
            .as_str_name();
        attributes.push(KeyValue::new("principal.kind", kind));
        attributes.push(KeyValue::new("principal.id", principal.id.clone()));
        attributes.push(KeyValue::new(
            "principal.verified_by",
            principal.verified_by_attr(),
        ));
        if let Some(via) = &principal.via {
            attributes.push(KeyValue::new("principal.via", via.clone()));
        }
    }
    Resource::builder_empty()
        .with_attributes(attributes)
        .build()
}

type BoxedLayer<S> = Box<dyn Layer<S> + Send + Sync>;

fn local_layer<S>(local: LocalOutput) -> Result<Option<BoxedLayer<S>>, InitError>
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a> + Send + Sync,
{
    let layer: BoxedLayer<S> = match local {
        LocalOutput::Text => Box::new(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr)
                .with_ansi(std::io::stderr().is_terminal()),
        ),
        LocalOutput::Json => Box::new(
            tracing_subscriber::fmt::layer()
                .json()
                .with_writer(std::io::stderr)
                .with_ansi(false),
        ),
        LocalOutput::JsonFile(path) => {
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .map_err(InitError::JsonFile)?;
            Box::new(
                tracing_subscriber::fmt::layer()
                    .json()
                    .with_writer(Mutex::new(file))
                    .with_ansi(false),
            )
        }
        LocalOutput::Off => return Ok(None),
    };
    Ok(Some(layer))
}

fn log_bridge_layer<S>(provider: &SdkLoggerProvider) -> BoxedLayer<S>
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a> + Send + Sync,
{
    Box::new(opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge::new(provider))
}

fn span_layer<S>(provider: &SdkTracerProvider) -> BoxedLayer<S>
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a> + Send + Sync,
{
    use opentelemetry::trace::TracerProvider;
    Box::new(tracing_opentelemetry::layer().with_tracer(provider.tracer("concord-telemetry")))
}

pub fn init(config: Config) -> Result<Guard, InitError> {
    if tracing::dispatcher::has_been_set() {
        return Err(InitError::AlreadyInitialized);
    }

    let resource = resource(&config);
    let (logs, traces) = if std::env::var_os("OTEL_EXPORTER_OTLP_ENDPOINT").is_some() {
        let log_exporter = opentelemetry_otlp::LogExporter::builder()
            .with_http()
            .with_protocol(Protocol::HttpBinary)
            .build()
            .map_err(|error| InitError::Exporter(error.to_string()))?;
        let span_exporter = opentelemetry_otlp::SpanExporter::builder()
            .with_http()
            .with_protocol(Protocol::HttpBinary)
            .build()
            .map_err(|error| InitError::Exporter(error.to_string()))?;
        let logs = SdkLoggerProvider::builder()
            .with_resource(resource.clone())
            .with_batch_exporter(log_exporter)
            .build();
        let traces = SdkTracerProvider::builder()
            .with_resource(resource)
            .with_batch_exporter(span_exporter)
            .build();
        (Some(logs), Some(traces))
    } else {
        (None, None)
    };

    let local = local_layer(config.local)?;
    let log_layer = logs.as_ref().map(log_bridge_layer);
    let trace_layer = traces.as_ref().map(span_layer);

    let subscriber = tracing_subscriber::registry()
        .with(filter(logs.is_some()))
        .with(trace_layer)
        .with(log_layer)
        .with(local);
    if subscriber.try_init().is_err() {
        return Err(InitError::AlreadyInitialized);
    }

    panic::install(logs.clone(), traces.clone());
    Ok(Guard {
        logs,
        traces,
        deadline: config.flush_deadline,
    })
}

#[cfg(all(test, feature = "testing"))]
mod tests {
    use opentelemetry_sdk::logs::{InMemoryLogExporter, SdkLoggerProvider};
    use tracing_subscriber::layer::SubscriberExt;

    /// Runs `body` under the exporting filter with an in-memory log exporter
    /// and returns what it exported. RUST_LOG is unset, so the default `info`
    /// applies.
    fn exported(
        body: impl FnOnce(),
    ) -> Vec<opentelemetry_sdk::logs::in_memory_exporter::LogDataWithResource> {
        std::env::remove_var("RUST_LOG");
        let exporter = InMemoryLogExporter::default();
        let provider = SdkLoggerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let subscriber = tracing_subscriber::registry()
            .with(super::filter(true))
            .with(
                opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge::new(&provider),
            );
        tracing::subscriber::with_default(subscriber, body);
        provider.force_flush().expect("flush in-memory exporter");
        exporter
            .get_emitted_logs()
            .expect("read in-memory exporter")
    }

    #[test]
    fn exporting_filter_allows_application_and_event_records_only() {
        let emitted = exported(|| {
            tracing::event!(name: "domain-event", target: super::EVENT_TARGET, tracing::Level::INFO, marker = "event");
            tracing::info!(target: "application", "application record");
            tracing::event!(name: "hyper-record", target: "hyper::client", tracing::Level::INFO, marker = "denied");
            tracing::event!(name: "reqwest-record", target: "reqwest::connect", tracing::Level::INFO, marker = "denied");
            tracing::event!(name: "h2-record", target: "h2::codec", tracing::Level::INFO, marker = "denied");
            tracing::event!(name: "otel-record", target: "opentelemetry_sdk::logs", tracing::Level::INFO, marker = "denied");
        });
        assert_eq!(emitted.len(), 2);
        assert!(emitted
            .iter()
            .any(|log| log.record.event_name() == Some("domain-event")));
        assert!(emitted
            .iter()
            .any(|log| log.record.target().map(|target| target.as_ref()) == Some("application")));
    }
}
