//! W3C trace context and labels across process boundaries.
//!
//! A carrier is anything with string keys: gRPC metadata (see `grpc`), NATS
//! headers, or the `TRACEPARENT` environment variable for hooks and CLIs.
//! The `baggage` header always carries the labels. The `traceparent` and
//! `tracestate` headers need the `otel` feature and an exporter; without them
//! `inject` writes only `baggage` and `set_parent` takes only the baggage.

use crate::baggage;
#[cfg(feature = "otel")]
use crate::carrier;
use crate::causes;
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
/// empty, through `set` (with the `otel` feature and a valid context), then its
/// `baggage` when it has any. Writes only to the map `set` is given.
pub fn inject(mut set: impl FnMut(&str, String)) {
    let current = tracing::Span::current();
    #[cfg(feature = "otel")]
    {
        const KEYS: [&str; 2] = ["traceparent", "tracestate"];
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
    }
    if let Some(header) = baggage::format(&baggage::entries(&current)) {
        set("baggage", header);
    }
}

/// Parents `span` on the context read through `get`, and takes its `baggage`.
/// Call it before `span` is first entered. A carrier without a valid
/// `traceparent` leaves the span's contextual parent in place. A span that was
/// already started refuses the carrier's baggage too.
pub fn set_parent(span: &tracing::Span, get: impl Fn(&str) -> Option<String>) {
    parent(span, get);
}

/// `set_parent`, saying whether the carrier held a valid `traceparent`. The
/// baggage is taken unless the span was already started. Without the `otel`
/// feature no carrier is valid, so the baggage is always taken.
pub(crate) fn parent(span: &tracing::Span, get: impl Fn(&str) -> Option<String>) -> bool {
    #[cfg(feature = "otel")]
    let valid = {
        let context = carrier::extract_context(&get);
        let valid = context.span().span_context().is_valid();
        carrier::report_invalid(span, &get, valid);
        // Only `AlreadyStarted` refuses the baggage. Other errors (no layer,
        // span disabled) leave the span unparented but still take its labels.
        let refused = valid
            && matches!(
                span.set_parent(context),
                Err(tracing_opentelemetry::SetParentError::AlreadyStarted)
            );
        if !refused {
            adopt_baggage(span, &get);
        }
        valid
    };
    #[cfg(not(feature = "otel"))]
    let valid = {
        adopt_baggage(span, &get);
        false
    };
    valid
}

/// Takes the carrier's baggage into `span` and snapshots its causes as the
/// entry point's. A carrier with no baggage is still an entry, with none. A
/// span that refuses the baggage (already entered with labels) keeps its
/// earlier snapshot.
fn adopt_baggage(span: &tracing::Span, get: &impl Fn(&str) -> Option<String>) {
    let kept = match get("baggage") {
        Some(header) => baggage::ingest(span, &header),
        None => Some(Vec::new()),
    };
    if let Some(kept) = kept {
        causes::mark_entry(span, &kept);
    }
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
