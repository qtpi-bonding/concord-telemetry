use crate::{AttrValue, Attrs, Event, EVENT_TARGET};
use opentelemetry::logs::AnyValue;
use opentelemetry::trace::TraceId;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::logs::{InMemoryLogExporter, SdkLoggerProvider};
use opentelemetry_sdk::trace::SdkTracerProvider;
use std::ops::Deref;
use std::sync::{Mutex, MutexGuard, OnceLock};
use tracing_subscriber::filter::Targets;
use tracing_subscriber::layer::{Layer, SubscriberExt};

static INSTALL: OnceLock<InMemoryLogExporter> = OnceLock::new();
static PROVIDER: OnceLock<&'static SdkLoggerProvider> = OnceLock::new();
static SUBSCRIBER_INSTALLED: OnceLock<()> = OnceLock::new();
static EXCLUSIVE: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, PartialEq)]
pub struct ErrorRecord {
    pub code: String,
    pub class: String,
    pub fingerprint: String,
    pub source: String,
    pub fields: Attrs,
}

#[derive(Clone, Copy)]
pub struct Capture {
    exporter: &'static InMemoryLogExporter,
}

pub struct ExclusiveCapture {
    _guard: MutexGuard<'static, ()>,
    capture: Capture,
}

impl Deref for ExclusiveCapture {
    type Target = Capture;

    fn deref(&self) -> &Capture {
        &self.capture
    }
}

fn provider() -> &'static SdkLoggerProvider {
    PROVIDER.get_or_init(|| {
        let exporter = InMemoryLogExporter::default();
        let provider = Box::leak(Box::new(
            SdkLoggerProvider::builder()
                .with_simple_exporter(exporter.clone())
                .build(),
        ));
        INSTALL
            .set(exporter)
            .expect("testing exporter was initialized twice");
        provider
    })
}

fn install() -> &'static InMemoryLogExporter {
    let exporter = provider_exporter();
    SUBSCRIBER_INSTALLED.get_or_init(|| {
        if tracing::dispatcher::has_been_set() {
            panic!("testing capture could not install its subscriber; use testing::layer()")
        }
        let tracer_provider = SdkTracerProvider::builder().build();
        let span_layer =
            tracing_opentelemetry::layer().with_tracer(tracer_provider.tracer("capture"));
        let bridge =
            opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge::new(provider())
                .with_filter(target_filter::<tracing_subscriber::Registry>());
        let subscriber = tracing_subscriber::registry().with(span_layer).with(bridge);
        tracing::subscriber::set_global_default(subscriber).unwrap_or_else(|_| {
            panic!("testing capture could not install its subscriber; use testing::layer()")
        });
    });
    exporter
}

fn provider_exporter() -> &'static InMemoryLogExporter {
    provider();
    INSTALL.get().expect("testing exporter was not initialized")
}

fn target_filter<S>() -> Targets
where
    S: tracing::Subscriber,
{
    Targets::new().with_target(EVENT_TARGET, tracing::Level::TRACE)
}

pub fn capture() -> Capture {
    Capture {
        exporter: install(),
    }
}

pub fn capture_exclusive() -> ExclusiveCapture {
    let guard = EXCLUSIVE.lock().expect("capture mutex poisoned");
    let capture = capture();
    capture.clear();
    ExclusiveCapture {
        _guard: guard,
        capture,
    }
}

pub fn layer<S>() -> impl Layer<S>
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge::new(provider())
        .with_filter(target_filter::<S>())
}

impl Capture {
    fn records(&self) -> Vec<opentelemetry_sdk::logs::SdkLogRecord> {
        self.exporter
            .get_emitted_logs()
            .expect("read testing log exporter")
            .into_iter()
            .map(|log| log.record)
            .collect()
    }

    fn clear(&self) {
        self.exporter.reset();
    }

    pub fn events<E: Event>(&self) -> Vec<E> {
        self.events_where(|_| true)
    }

    pub fn events_where<E: Event>(&self, predicate: impl Fn(&E) -> bool) -> Vec<E> {
        self.records()
            .into_iter()
            .filter(|log| log.event_name() == Some(E::TYPE_NAME))
            .filter_map(|log| {
                let attrs = attrs(log.attributes_iter());
                E::from_attributes(&attrs).ok()
            })
            .filter(predicate)
            .collect()
    }

    pub fn single_event_where<E: Event>(&self, predicate: impl Fn(&E) -> bool) -> E {
        let events = self.events_where(predicate);
        assert_eq!(
            events.len(),
            1,
            "expected one matching event, found {}",
            events.len()
        );
        events
            .into_iter()
            .next()
            .expect("matching event disappeared")
    }

    pub fn errors_where(&self, predicate: impl Fn(&ErrorRecord) -> bool) -> Vec<ErrorRecord> {
        self.records()
            .into_iter()
            .filter(|log| log.event_name() == Some("error"))
            .filter_map(|log| error_record(log.attributes_iter()))
            .filter(predicate)
            .collect()
    }

    #[doc(hidden)]
    pub fn trace_id_for(&self, event_name: &'static str) -> Option<TraceId> {
        self.records()
            .into_iter()
            .find(|log| log.event_name() == Some(event_name))
            .and_then(|log| log.trace_context().map(|context| context.trace_id))
    }
}

fn attrs<'a, I>(iter: I) -> Attrs
where
    I: IntoIterator<Item = &'a (opentelemetry::Key, AnyValue)>,
{
    iter.into_iter()
        .filter_map(|(key, value)| any_value(value).map(|value| (key.to_string(), value)))
        .collect()
}

fn any_value(value: &AnyValue) -> Option<AttrValue> {
    match value {
        AnyValue::Int(value) => Some(AttrValue::I64(*value)),
        AnyValue::Double(value) => Some(AttrValue::F64(*value)),
        AnyValue::String(value) => Some(AttrValue::Str(value.to_string())),
        AnyValue::Boolean(value) => Some(AttrValue::Bool(*value)),
        _ => None,
    }
}

fn error_record<'a, I>(iter: I) -> Option<ErrorRecord>
where
    I: IntoIterator<Item = &'a (opentelemetry::Key, AnyValue)>,
{
    let attrs = attrs(iter);
    let code = string_attr(&attrs, "error.code")?;
    let class = string_attr(&attrs, "error.class")?;
    let fingerprint = string_attr(&attrs, "error.fingerprint")?;
    let source = string_attr(&attrs, "error.source")?;
    let mut fields = attrs;
    fields.remove("error.code");
    fields.remove("error.class");
    fields.remove("error.fingerprint");
    fields.remove("error.source");
    Some(ErrorRecord {
        code,
        class,
        fingerprint,
        source,
        fields,
    })
}

fn string_attr(attrs: &Attrs, name: &str) -> Option<String> {
    match attrs.get(name) {
        Some(AttrValue::Str(value)) => Some(value.clone()),
        _ => None,
    }
}
