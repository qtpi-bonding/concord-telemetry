//! W3C trace context across process boundaries.
//!
//! A carrier is anything with string keys: gRPC metadata (see `grpc`), NATS
//! headers, or the `TRACEPARENT` environment variable for hooks and CLIs.
//! Without an OTLP exporter there is no OpenTelemetry span layer, so nothing
//! is injected and `set_parent` does nothing.

use opentelemetry::propagation::{Extractor, Injector, TextMapPropagator};
use opentelemetry::trace::TraceContextExt;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use std::collections::HashMap;
use tracing_opentelemetry::OpenTelemetrySpanExt;

/// The environment variable hooks and CLIs read and write.
pub use concord_telemetry_proto::TRACEPARENT_ENV;

const KEYS: [&str; 2] = ["traceparent", "tracestate"];

/// The current span's context as a W3C `traceparent`, if it has one.
pub fn traceparent() -> Option<String> {
    let mut found = None;
    inject(|key, value| {
        if key == "traceparent" {
            found = Some(value);
        }
    });
    found
}

/// Writes the current span's `traceparent`, and `tracestate` when it is not
/// empty, through `set`.
pub fn inject(mut set: impl FnMut(&str, String)) {
    let context = tracing::Span::current().context();
    if !context.span().span_context().is_valid() {
        return;
    }
    let mut carrier = HashMap::new();
    TraceContextPropagator::new().inject_context(&context, &mut Carrier(&mut carrier));
    for key in KEYS {
        if let Some(value) = carrier.remove(key).filter(|value| !value.is_empty()) {
            set(key, value);
        }
    }
}

/// Parents `span` on the context read through `get`. Call it before `span` is
/// first entered. A carrier without a valid `traceparent` leaves the span's
/// contextual parent in place.
pub fn set_parent(span: &tracing::Span, get: impl Fn(&str) -> Option<String>) {
    let carrier: HashMap<String, String> = KEYS
        .iter()
        .filter_map(|key| get(key).map(|value| ((*key).to_owned(), value)))
        .collect();
    let context = TraceContextPropagator::new().extract(&Fields(&carrier));
    if context.span().span_context().is_valid() {
        // Every error means no OpenTelemetry parent can be set (no layer,
        // span disabled, already started); none is worth reporting.
        let _ = span.set_parent(context);
    }
}

/// `set_parent` from the `TRACEPARENT` environment variable.
pub fn set_parent_from_env(span: &tracing::Span) {
    set_parent(span, |key| {
        (key == "traceparent")
            .then(|| std::env::var(TRACEPARENT_ENV).ok())
            .flatten()
    });
}

struct Carrier<'a>(&'a mut HashMap<String, String>);

impl Injector for Carrier<'_> {
    fn set(&mut self, key: &str, value: String) {
        self.0.insert(key.to_owned(), value);
    }
}

struct Fields<'a>(&'a HashMap<String, String>);

impl Extractor for Fields<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(String::as_str).collect()
    }
}
