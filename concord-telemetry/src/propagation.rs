//! W3C trace context across process boundaries.
//!
//! A carrier is anything with string keys: gRPC metadata (see `grpc`), NATS
//! headers, or the `TRACEPARENT` environment variable for hooks and CLIs.
//! Without an OTLP exporter there is no OpenTelemetry span layer, so nothing
//! is injected and `set_parent` does nothing.

#[cfg(feature = "otel")]
use crate::{baggage, carrier};
#[cfg(feature = "otel")]
use opentelemetry::propagation::{Extractor, Injector, TextMapPropagator};
#[cfg(feature = "otel")]
use opentelemetry::trace::TraceContextExt;
#[cfg(feature = "otel")]
use opentelemetry_sdk::propagation::TraceContextPropagator;
#[cfg(feature = "otel")]
use std::collections::HashMap;
#[cfg(feature = "otel")]
use tracing_opentelemetry::OpenTelemetrySpanExt;

/// The environment variables hooks and CLIs read and write.
pub use concord_telemetry_proto::{BAGGAGE_ENV, TRACEPARENT_ENV};

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
/// empty, through `set`, then its `baggage` when it has any.
#[cfg(feature = "otel")]
pub fn inject(mut set: impl FnMut(&str, String)) {
    const KEYS: [&str; 2] = ["traceparent", "tracestate"];
    let current = tracing::Span::current();
    let context = current.context();
    if context.span().span_context().is_valid() {
        let mut carrier = HashMap::new();
        TraceContextPropagator::new().inject_context(&context, &mut Carrier(&mut carrier));
        for key in KEYS {
            if let Some(value) = carrier.remove(key).filter(|value| !value.is_empty()) {
                set(key, value);
            }
        }
    }
    if let Some(header) = baggage::format(&baggage::entries(&current)) {
        set("baggage", header);
    }
}

/// `inject` with the otel feature off: nothing is written.
#[cfg(not(feature = "otel"))]
pub fn inject(_set: impl FnMut(&str, String)) {}

/// Parents `span` on the context read through `get`, and takes its `baggage`.
/// Call it before `span` is first entered. A carrier without a valid
/// `traceparent` leaves the span's contextual parent in place.
pub fn set_parent(span: &tracing::Span, get: impl Fn(&str) -> Option<String>) {
    parent(span, get);
}

/// `set_parent`, saying whether the carrier held a valid `traceparent`.
#[cfg(feature = "otel")]
pub(crate) fn parent(span: &tracing::Span, get: impl Fn(&str) -> Option<String>) -> bool {
    let context = carrier::extract_context(&get);
    let valid = context.span().span_context().is_valid();
    carrier::report_invalid(span, &get, valid);
    // An error means no OpenTelemetry parent can be set (no layer, span
    // disabled, already started); none is worth reporting. A span that was
    // already started refuses the carrier's baggage too.
    let parented = !valid || span.set_parent(context).is_ok();
    if parented {
        if let Some(header) = get("baggage") {
            baggage::ingest(span, &header);
        }
    }
    valid
}

/// `parent` with the otel feature off: no parent is set and nothing is
/// reported, so no carrier counts as valid.
#[cfg(not(feature = "otel"))]
pub(crate) fn parent(_span: &tracing::Span, _get: impl Fn(&str) -> Option<String>) -> bool {
    false
}

/// `set_parent` from the `TRACEPARENT` and `BAGGAGE` environment variables.
/// `tracestate` is not carried this way.
pub fn set_parent_from_env(span: &tracing::Span) {
    set_parent(span, |key| match key {
        "traceparent" => std::env::var(TRACEPARENT_ENV).ok(),
        "baggage" => std::env::var(BAGGAGE_ENV).ok(),
        _ => None,
    });
}

/// The environment variables a child process needs to continue a carrier's
/// trace; defined in `concord-telemetry-proto` so OpenTelemetry-free crates
/// can use it. The counterpart of `set_parent_from_env`.
pub use concord_telemetry_proto::inject_env;

/// Injects into a map.
#[cfg(feature = "otel")]
pub(crate) struct Carrier<'a>(pub &'a mut HashMap<String, String>);

#[cfg(feature = "otel")]
impl Injector for Carrier<'_> {
    fn set(&mut self, key: &str, value: String) {
        self.0.insert(key.to_owned(), value);
    }
}

/// Extracts from a map.
#[cfg(feature = "otel")]
pub(crate) struct Fields<'a>(pub &'a HashMap<String, String>);

#[cfg(feature = "otel")]
impl Extractor for Fields<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(String::as_str).collect()
    }
}
