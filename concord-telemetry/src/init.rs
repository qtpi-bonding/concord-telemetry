use crate::{deadline::run_with_deadline, panic, Principal, PrincipalKind};
use opentelemetry::KeyValue;
use opentelemetry_otlp::{Protocol, WithExportConfig};
use opentelemetry_sdk::{logs::SdkLoggerProvider, trace::SdkTracerProvider, Resource};
use std::hash::{BuildHasher, Hasher};
use std::{fs::OpenOptions, io::IsTerminal, path::PathBuf, sync::Mutex, time::Duration};
use tracing_subscriber::{
    filter::{FilterExt, LevelFilter, Targets},
    layer::{Layer, SubscriberExt},
    registry::LookupSpan,
    util::SubscriberInitExt,
    EnvFilter,
};

pub const EVENT_TARGET: &str = "concord_telemetry::event";

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

fn filter<S>() -> impl tracing_subscriber::layer::Filter<S>
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    let environment = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    environment.or(Targets::new().with_target(EVENT_TARGET, tracing::Level::TRACE))
}

fn export_filter<S>() -> impl tracing_subscriber::layer::Filter<S>
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    let denied = [
        "hyper",
        "hyper_util",
        "h2",
        "reqwest",
        "rustls",
        "tower",
        "opentelemetry",
        "opentelemetry_sdk",
        "opentelemetry_otlp",
        "opentelemetry_http",
        "opentelemetry_proto",
    ]
    .into_iter()
    .fold(
        Targets::new().with_default(LevelFilter::TRACE),
        |targets, name| targets.with_target(name, LevelFilter::OFF),
    );
    filter::<S>().and(denied)
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
                .with_ansi(std::io::stderr().is_terminal())
                .with_filter(filter::<S>()),
        ),
        LocalOutput::Json => Box::new(
            tracing_subscriber::fmt::layer()
                .json()
                .with_writer(std::io::stderr)
                .with_ansi(false)
                .with_filter(filter::<S>()),
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
                    .with_ansi(false)
                    .with_filter(filter::<S>()),
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
    Box::new(
        opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge::new(provider)
            .with_filter(export_filter::<S>()),
    )
}

fn span_layer<S>(provider: &SdkTracerProvider) -> BoxedLayer<S>
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a> + Send + Sync,
{
    use opentelemetry::trace::TracerProvider;
    Box::new(
        tracing_opentelemetry::layer()
            .with_tracer(provider.tracer("concord-telemetry"))
            .with_filter(export_filter::<S>()),
    )
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
    #[test]
    fn export_filter_allows_application_and_event_records_only() {
        use opentelemetry_sdk::logs::{InMemoryLogExporter, SdkLoggerProvider};
        use tracing_subscriber::layer::{Layer, SubscriberExt};

        let exporter = InMemoryLogExporter::default();
        let provider = SdkLoggerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let layer =
            opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge::new(&provider)
                .with_filter(super::export_filter::<tracing_subscriber::Registry>());
        let subscriber = tracing_subscriber::registry().with(layer);
        tracing::subscriber::with_default(subscriber, || {
            tracing::event!(name: "domain-event", target: super::EVENT_TARGET, tracing::Level::INFO, marker = "event");
            tracing::info!(target: "application", "application record");
            tracing::event!(name: "hyper-record", target: "hyper::client", tracing::Level::TRACE, marker = "denied");
            tracing::event!(name: "reqwest-record", target: "reqwest::connect", tracing::Level::TRACE, marker = "denied");
            tracing::event!(name: "h2-record", target: "h2::codec", tracing::Level::TRACE, marker = "denied");
            tracing::event!(name: "otel-record", target: "opentelemetry_sdk::logs", tracing::Level::TRACE, marker = "denied");
        });
        provider.force_flush().expect("flush in-memory exporter");
        let emitted = exporter
            .get_emitted_logs()
            .expect("read in-memory exporter");
        assert_eq!(emitted.len(), 2);
        assert!(emitted
            .iter()
            .any(|log| log.record.event_name() == Some("domain-event")));
        assert!(emitted
            .iter()
            .any(|log| log.record.target().map(|target| target.as_ref()) == Some("application")));
        for denied_target in [
            "hyper::client",
            "reqwest::connect",
            "h2::codec",
            "opentelemetry_sdk::logs",
        ] {
            assert!(emitted.iter().all(|log| {
                log.record.target().map(|target| target.as_ref()) != Some(denied_target)
            }));
        }
    }
}
