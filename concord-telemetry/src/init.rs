use crate::logfmt::{BaggageBridge, BaggageLogProcessor, TraceFormat};
use crate::{deadline::run_with_deadline, panic, Principal, PrincipalKind};
use opentelemetry::KeyValue;
use opentelemetry_otlp::{Protocol, WithExportConfig};
use opentelemetry_sdk::{
    logs::{BatchLogProcessor, SdkLoggerProvider},
    trace::SdkTracerProvider,
    Resource,
};
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
pub(crate) const HTTP_TARGET: &str = "concord_telemetry::http";

pub struct Config {
    pub service_name: &'static str,
    pub service_version: &'static str,
    pub principal: Option<Principal>,
    pub local: LocalOutput,
    pub flush_deadline: Duration,
    /// Where the binary was built from; see [`BuildInfo`] and
    /// [`Config::with_build_info`].
    pub build: BuildInfo,
}

/// The repository and commit a binary was built from, taken from the build
/// environment of the caller's crate (see `build_info!`). Empty means unknown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BuildInfo {
    /// `CONCORD_GIT_REPO` at build time: the resource's `vcs.repository.name`.
    pub repository: &'static str,
    /// `CONCORD_GIT_SHA` at build time: the resource's `vcs.ref.head.revision`.
    pub revision: &'static str,
}

impl Config {
    /// Records where the binary was built from: `Config::new(name,
    /// version!()).with_build_info(build_info!())`. Empty fields are omitted.
    pub fn with_build_info(mut self, build: BuildInfo) -> Self {
        self.build = build;
        self
    }

    pub fn new(service_name: &'static str, service_version: &'static str) -> Self {
        Self {
            service_name,
            service_version,
            principal: None,
            local: LocalOutput::Text,
            flush_deadline: Duration::from_secs(2),
            build: BuildInfo::default(),
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
/// error records and the gRPC and HTTP trace spans always on, and the exporter's own
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
    filter = filter.add_directive(directive(HTTP_TARGET, LevelFilter::INFO));
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

/// Resource keys the environment cannot set: the service's identity is the
/// caller's, the instance id is generated, and principals come from `Config`.
fn reserved(key: &str) -> bool {
    key == "service.name" || key == "service.instance.id" || key.starts_with("principal.")
}

/// Percent-decodes an `OTEL_RESOURCE_ATTRIBUTES` value; `None` on a malformed
/// escape or bytes that are not UTF-8.
fn percent_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = value.get(i + 1..i + 3)?;
            if hex.len() != 2 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Parses `OTEL_RESOURCE_ATTRIBUTES` (`k1=v1,k2=v2`, values percent-encoded).
/// An entry that is malformed (no `=`, empty key, bad escape) or reserved is
/// skipped; the rest still apply.
fn env_attributes(raw: &str) -> Vec<(String, String)> {
    raw.split(',')
        .filter_map(|entry| {
            let (key, value) = entry.split_once('=')?;
            let key = key.trim();
            if key.is_empty() || reserved(key) {
                return None;
            }
            Some((key.to_owned(), percent_decode(value.trim())?))
        })
        .collect()
}

/// The machine's hostname, if it can be read.
fn hostname() -> Option<String> {
    #[cfg(unix)]
    {
        let mut buf = [0u8; 256];
        // SAFETY: the buffer is valid for `buf.len()` bytes; gethostname writes
        // at most that many and we only read up to the first NUL.
        let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
        if rc == 0 {
            let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            return String::from_utf8(buf[..end].to_vec())
                .ok()
                .filter(|name| !name.is_empty());
        }
        None
    }
    #[cfg(not(unix))]
    {
        None
    }
}

fn resource(config: &Config) -> Resource {
    resource_with(
        config,
        std::env::var("OTEL_RESOURCE_ATTRIBUTES").ok().as_deref(),
        hostname(),
    )
}

/// `service.version` from `OTEL_RESOURCE_ATTRIBUTES` overrides
/// `Config::service_version`; `host.name` is the machine hostname unless the
/// environment sets it. Other environment attributes are added as they come.
fn resource_with(config: &Config, env: Option<&str>, host: Option<String>) -> Resource {
    let env = env.map(env_attributes).unwrap_or_default();
    let from_env = |key: &str| {
        env.iter()
            .rev()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.clone())
            .filter(|value| !value.is_empty())
    };
    let mut attributes = vec![
        KeyValue::new("service.name", config.service_name),
        KeyValue::new(
            "service.version",
            from_env("service.version").unwrap_or_else(|| config.service_version.to_owned()),
        ),
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
    if let Some(host) = from_env("host.name").or(host) {
        attributes.push(KeyValue::new("host.name", host));
    }
    let non_empty = |value: &str| (!value.is_empty()).then(|| value.to_owned());
    if let Some(repository) =
        from_env("vcs.repository.name").or_else(|| non_empty(config.build.repository))
    {
        attributes.push(KeyValue::new("vcs.repository.name", repository));
    }
    if let Some(revision) =
        from_env("vcs.ref.head.revision").or_else(|| non_empty(config.build.revision))
    {
        attributes.push(KeyValue::new("vcs.ref.head.revision", revision));
    }
    let mut extra: Vec<(String, String)> = Vec::new();
    for (key, value) in &env {
        if !matches!(
            key.as_str(),
            "service.version" | "host.name" | "vcs.repository.name" | "vcs.ref.head.revision"
        ) {
            extra.retain(|(name, _)| name != key);
            extra.push((key.clone(), value.clone()));
        }
    }
    attributes.extend(
        extra
            .into_iter()
            .map(|(key, value)| KeyValue::new(key, value)),
    );
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

/// The JSON line layer: the stock JSON format plus the trace fields.
fn json_layer<S, W>(writer: W) -> impl Layer<S>
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    W: for<'a> tracing_subscriber::fmt::MakeWriter<'a> + 'static,
{
    tracing_subscriber::fmt::layer()
        .fmt_fields(tracing_subscriber::fmt::format::JsonFields::new())
        .event_format(TraceFormat::json(
            tracing_subscriber::fmt::format().json(),
            crate::carrier::otel_active,
        ))
        .with_writer(writer)
        .with_ansi(false)
}

fn local_layer<S>(local: LocalOutput) -> Result<Option<BoxedLayer<S>>, InitError>
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a> + Send + Sync,
{
    let layer: BoxedLayer<S> = match local {
        LocalOutput::Text => Box::new(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr)
                .with_ansi(std::io::stderr().is_terminal())
                .event_format(TraceFormat::text(
                    tracing_subscriber::fmt::format().with_ansi(std::io::stderr().is_terminal()),
                    crate::carrier::otel_active,
                )),
        ),
        LocalOutput::Json => Box::new(json_layer(std::io::stderr)),
        LocalOutput::JsonFile(path) => {
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .map_err(InitError::JsonFile)?;
            Box::new(json_layer(Mutex::new(file)))
        }
        LocalOutput::Off => return Ok(None),
    };
    Ok(Some(layer))
}

fn log_bridge_layer<S>(provider: &SdkLoggerProvider) -> BoxedLayer<S>
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a> + Send + Sync,
{
    Box::new(BaggageBridge(
        opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge::new(provider),
    ))
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
    let unknown_error_text = crate::error::set_error_text_from_env();

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
            .with_log_processor(BaggageLogProcessor(
                BatchLogProcessor::builder(log_exporter).build(),
            ))
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
    let baggage_layer = traces.as_ref().map(|_| crate::baggage::BaggageLayer);

    let subscriber = tracing_subscriber::registry()
        .with(filter(logs.is_some()))
        .with(trace_layer)
        .with(baggage_layer)
        .with(log_layer)
        .with(local);
    if subscriber.try_init().is_err() {
        return Err(InitError::AlreadyInitialized);
    }

    crate::carrier::set_otel_active(traces.is_some());
    panic::install(logs.clone(), traces.clone());
    if let Some(value) = unknown_error_text.as_deref() {
        crate::error::warn_unknown_error_text(value);
    }
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

#[cfg(test)]
mod resource_tests {
    use super::*;
    use opentelemetry::Key;

    fn get(resource: &Resource, key: &str) -> Option<String> {
        resource
            .get(&Key::new(key.to_owned()))
            .map(|value| value.to_string())
    }

    fn config() -> Config {
        Config::new("svc", "1.2.3")
    }

    #[test]
    fn env_values_merge_and_service_version_overrides() {
        let resource = resource_with(
            &config(),
            Some("service.version=abc123,deployment.environment=prod%2Fblue,host.name=box"),
            Some("machine".into()),
        );
        assert_eq!(get(&resource, "service.version").as_deref(), Some("abc123"));
        assert_eq!(
            get(&resource, "deployment.environment").as_deref(),
            Some("prod/blue")
        );
        assert_eq!(get(&resource, "host.name").as_deref(), Some("box"));
        assert_eq!(get(&resource, "service.name").as_deref(), Some("svc"));
        assert!(get(&resource, "service.instance.id").is_some());
    }

    #[test]
    fn env_unset_keeps_config_values_and_adds_host_name() {
        let resource = resource_with(&config(), None, Some("machine".into()));
        assert_eq!(get(&resource, "service.version").as_deref(), Some("1.2.3"));
        assert_eq!(get(&resource, "host.name").as_deref(), Some("machine"));
        assert_eq!(get(&resource, "service.name").as_deref(), Some("svc"));
        assert_eq!(resource.len(), 4);
    }

    #[test]
    fn malformed_entries_are_skipped_not_fatal() {
        let resource = resource_with(
            &config(),
            Some("novalue,=nokey,bad=%zz,bad2=%4,bad3=%ff,,good=yes, spaced = v "),
            None,
        );
        assert_eq!(get(&resource, "good").as_deref(), Some("yes"));
        assert_eq!(get(&resource, "spaced").as_deref(), Some("v"));
        for key in ["novalue", "bad", "bad2", "bad3", ""] {
            assert_eq!(get(&resource, key), None, "{key}");
        }
        assert_eq!(get(&resource, "service.version").as_deref(), Some("1.2.3"));
        assert_eq!(get(&resource, "host.name"), None);
    }

    #[test]
    fn build_info_sets_vcs_keys_unless_empty_and_env_overrides() {
        let built = config().with_build_info(BuildInfo {
            repository: "grorg",
            revision: "abc123",
        });
        let resource = resource_with(&built, None, None);
        assert_eq!(
            get(&resource, "vcs.repository.name").as_deref(),
            Some("grorg")
        );
        assert_eq!(
            get(&resource, "vcs.ref.head.revision").as_deref(),
            Some("abc123")
        );

        let resource = resource_with(
            &built,
            Some("vcs.repository.name=other,vcs.ref.head.revision=fff"),
            None,
        );
        assert_eq!(
            get(&resource, "vcs.repository.name").as_deref(),
            Some("other")
        );
        assert_eq!(
            get(&resource, "vcs.ref.head.revision").as_deref(),
            Some("fff")
        );

        // Empty env values are ignored; empty build info omits the keys.
        let resource = resource_with(&built, Some("vcs.repository.name="), None);
        assert_eq!(
            get(&resource, "vcs.repository.name").as_deref(),
            Some("grorg")
        );
        let resource = resource_with(&config(), None, None);
        assert_eq!(get(&resource, "vcs.repository.name"), None);
        assert_eq!(get(&resource, "vcs.ref.head.revision"), None);
        let empty = config().with_build_info(BuildInfo::default());
        assert_eq!(
            get(&resource_with(&empty, None, None), "vcs.repository.name"),
            None
        );
    }

    #[test]
    fn percent_escapes_need_exactly_two_hex_digits() {
        let resource = resource_with(&config(), Some("a=%+1,b=%-1,c=%41,d=%4"), None);
        assert_eq!(get(&resource, "a"), None);
        assert_eq!(get(&resource, "b"), None);
        assert_eq!(get(&resource, "c").as_deref(), Some("A"));
        assert_eq!(get(&resource, "d"), None);
    }

    #[test]
    fn empty_service_version_keeps_the_config_value() {
        let resource = resource_with(&config(), Some("service.version="), Some("m".into()));
        assert_eq!(get(&resource, "service.version").as_deref(), Some("1.2.3"));
        let resource = resource_with(&config(), Some("host.name="), Some("m".into()));
        assert_eq!(get(&resource, "host.name").as_deref(), Some("m"));
    }

    #[test]
    fn identity_and_principal_keys_are_not_settable_from_env() {
        let resource = resource_with(
            &config(),
            Some("service.name=evil,service.instance.id=x,principal.id=y"),
            None,
        );
        assert_eq!(get(&resource, "service.name").as_deref(), Some("svc"));
        assert_ne!(get(&resource, "service.instance.id").as_deref(), Some("x"));
        assert_eq!(get(&resource, "principal.id"), None);
    }

    #[test]
    #[cfg(unix)]
    fn real_hostname_is_nonempty() {
        assert!(hostname().is_some_and(|name| !name.is_empty()));
    }
}
