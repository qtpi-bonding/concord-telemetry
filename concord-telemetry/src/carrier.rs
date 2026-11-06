//! Trace carriers: capturing the current trace and baggage, and joining or
//! linking to one on the other side of a boundary.
//!
//! Without an exporter there is no span layer: `capture` returns an empty
//! carrier, the extract helpers change nothing, and `concord.trace.missing`
//! is never recorded.

use crate::{baggage, init::EVENT_TARGET, propagation};
use coded_error::{fingerprint, ErrorClass};
use concord_telemetry_proto::TraceCarrier;
use opentelemetry::propagation::{Extractor, TextMapPropagator};
use opentelemetry::trace::TraceContextExt;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::Span;
use tracing_opentelemetry::OpenTelemetrySpanExt;

/// Code of the record emitted when a required hop gets no trace.
pub const TRACE_MISSING_CODE: &str = "concord.trace.missing";
/// Code of the record emitted when baggage entries were dropped.
pub const BAGGAGE_DROPPED_CODE: &str = "concord.trace.baggage_dropped";

static OTEL_ACTIVE: AtomicBool = AtomicBool::new(false);

pub(crate) fn set_otel_active(active: bool) {
    OTEL_ACTIVE.store(active, Ordering::Relaxed);
}

fn otel_active() -> bool {
    OTEL_ACTIVE.load(Ordering::Relaxed)
}

pub mod v1 {
    /// `concord_telemetry.v1.TraceCarrier`.
    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct TraceCarrier {
        #[prost(string, optional, tag = "1")]
        pub traceparent: ::core::option::Option<::prost::alloc::string::String>,
        #[prost(string, optional, tag = "2")]
        pub tracestate: ::core::option::Option<::prost::alloc::string::String>,
        #[prost(string, optional, tag = "3")]
        pub baggage: ::core::option::Option<::prost::alloc::string::String>,
    }
}

impl From<&TraceCarrier> for v1::TraceCarrier {
    fn from(c: &TraceCarrier) -> Self {
        Self {
            traceparent: c.traceparent.clone(),
            tracestate: c.tracestate.clone(),
            baggage: c.baggage.clone(),
        }
    }
}

impl From<&v1::TraceCarrier> for TraceCarrier {
    fn from(c: &v1::TraceCarrier) -> Self {
        Self {
            traceparent: c.traceparent.clone(),
            tracestate: c.tracestate.clone(),
            baggage: c.baggage.clone(),
        }
    }
}

/// The current span's trace context and baggage.
pub fn capture() -> TraceCarrier {
    let mut carrier = TraceCarrier::default();
    propagation::inject(|key, value| match key {
        "traceparent" => carrier.traceparent = Some(value),
        "tracestate" => carrier.tracestate = Some(value),
        "baggage" => carrier.baggage = Some(value),
        _ => {}
    });
    carrier
}

fn getter(c: &TraceCarrier) -> impl Fn(&str) -> Option<String> + '_ {
    |key| match key {
        "traceparent" => c.traceparent.clone(),
        "tracestate" => c.tracestate.clone(),
        "baggage" => c.baggage.clone(),
        _ => None,
    }
}

/// Makes `span` a child of the carrier's trace and takes its baggage. Call it
/// before `span` is first entered. A carrier without a valid `traceparent`
/// leaves the span's contextual parent in place.
pub fn parent_on(span: &Span, c: &TraceCarrier) {
    propagation::parent(span, getter(c));
}

/// Gives `span` a link to the carrier's span context, in a trace of its own,
/// and takes the carrier's baggage.
pub fn link_from(span: &Span, c: &TraceCarrier) {
    let context = extract_context(getter(c));
    let span_context = context.span().span_context().clone();
    if span_context.is_valid() {
        span.add_link(span_context);
    }
    if let Some(header) = &c.baggage {
        baggage::ingest(span, header);
    }
}

/// `parent_on` for a hop that must carry a trace. With an exporter active and
/// no valid `traceparent` it records `concord.trace.missing` with the field
/// `hop`.
pub fn extract_required(hop: &'static str, span: &Span, c: &TraceCarrier) {
    let valid = propagation::parent(span, getter(c));
    if !valid && otel_active() {
        emit_trace_missing(hop);
    }
}

/// `parent_on` for an entry point that may legitimately get no trace. It is
/// silent when the carrier is empty. `name` is recorded on the span as
/// `ingress`.
pub fn extract_ingress(name: &'static str, span: &Span, c: &TraceCarrier) {
    propagation::parent(span, getter(c));
    span.set_attribute("ingress", name);
}

/// A span with no parent: it starts a trace of its own.
pub fn root(name: &'static str) -> Span {
    tracing::info_span!(target: EVENT_TARGET, parent: None, "root", otel.name = name)
}

/// The labels of the current span's baggage. See `carrier` for carrying them.
pub mod context {
    use super::*;

    /// Adds a label to the current span's baggage and records it as a span
    /// attribute. Children and outgoing carriers inherit it. Over the baggage
    /// caps it is dropped and recorded.
    pub fn add(field: &'static str, value: impl Into<String>) {
        baggage::add(&Span::current(), field, value.into());
    }
}

pub(crate) fn extract_context(get: impl Fn(&str) -> Option<String>) -> opentelemetry::Context {
    const TRACESTATE_MAX: usize = 256;
    let mut fields = HashMap::new();
    if let Some(value) = get("traceparent") {
        fields.insert("traceparent".to_owned(), value);
    }
    if let Some(value) = get("tracestate").filter(|v| v.len() <= TRACESTATE_MAX) {
        fields.insert("tracestate".to_owned(), value);
    }
    TraceContextPropagator::new().extract(&Fields(&fields))
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

fn emit_trace_missing(hop: &'static str) {
    let class = ErrorClass::Infrastructure;
    let source = module_path!();
    let digest = fingerprint(TRACE_MISSING_CODE, class, source);
    tracing::event!(
        name: "error",
        target: EVENT_TARGET,
        tracing::Level::WARN,
        error.code = TRACE_MISSING_CODE,
        error.class = class.as_str(),
        error.fingerprint = digest.as_str(),
        error.source = source,
        hop = hop,
    );
}

pub(crate) fn emit_baggage_dropped(count: usize) {
    let class = ErrorClass::Logical;
    let source = module_path!();
    let digest = fingerprint(BAGGAGE_DROPPED_CODE, class, source);
    tracing::event!(
        name: "error",
        target: EVENT_TARGET,
        tracing::Level::WARN,
        error.code = BAGGAGE_DROPPED_CODE,
        error.class = class.as_str(),
        error.fingerprint = digest.as_str(),
        error.source = source,
        dropped = count as i64,
    );
}
