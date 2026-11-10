//! Trace carriers: capturing the current trace and baggage, and joining or
//! linking to one on the other side of a boundary.
//!
//! Without an exporter there is no span layer: `capture` returns an empty
//! carrier, the extract helpers change nothing, and `concord.trace.missing`
//! is never recorded.

use crate::coded::emit_coded;
use crate::propagation::Fields;
use crate::{baggage, init::EVENT_TARGET, propagation};
use coded_error::ErrorClass;
use concord_telemetry_proto::TraceCarrier;
use opentelemetry::propagation::TextMapPropagator;
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

/// Code of the record emitted when an incoming `traceparent` is malformed or
/// a `tracestate` is over its cap. The field `header` names which.
pub const CARRIER_DROPPED_CODE: &str = "concord.trace.carrier_dropped";

/// W3C Trace Context: a `tracestate` over this many characters is shortened.
const TRACESTATE_MAX: usize = 512;
/// When shortening, entries longer than this go first.
const TRACESTATE_ENTRY_MAX: usize = 128;

static OTEL_ACTIVE: AtomicBool = AtomicBool::new(false);

pub(crate) fn set_otel_active(active: bool) {
    OTEL_ACTIVE.store(active, Ordering::Relaxed);
}

pub(crate) fn otel_active() -> bool {
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

/// Makes `span` a child of the carrier's trace and replaces the span's labels
/// with the carrier's baggage, when it has a `baggage` header. Call it before
/// `span` is first entered: entering fixes its trace (`context::add` needs the
/// span entered, so it does too), and a later `parent_on` cannot re-parent,
/// though it still replaces the labels. A carrier without a valid
/// `traceparent` leaves the span's contextual parent in place.
pub fn parent_on(span: &Span, c: &TraceCarrier) {
    propagation::parent(span, getter(c));
}

/// Gives `span` a link to the carrier's span context, in a trace of its own.
/// A link carries span context only, as in OpenTelemetry: the span's labels
/// are untouched. A caller that wants the linked span's labels calls
/// `context::add` for each.
pub fn link_from(span: &Span, c: &TraceCarrier) {
    let context = extract_context(getter(c));
    let span_context = context.span().span_context().clone();
    report_invalid(span, &getter(c), span_context.is_valid());
    if span_context.is_valid() {
        span.add_link(span_context);
    }
}

/// `parent_on` for a hop that must carry a trace. With an exporter active and
/// no valid `traceparent` it records `concord.trace.missing` with the field
/// `hop`.
pub fn extract_required(hop: &'static str, span: &Span, c: &TraceCarrier) {
    let valid = propagation::parent(span, getter(c));
    let malformed = c.traceparent.as_deref().is_some_and(|v| !v.is_empty());
    if !valid && !malformed && otel_active() {
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

    /// The most labels one span's baggage holds.
    pub const MAX_ENTRIES: usize = crate::baggage::MAX_ENTRIES;

    /// Adds a label to the current span's baggage and records it as a span
    /// attribute. Children and outgoing carriers inherit it. Over the baggage
    /// caps it is dropped and recorded.
    pub fn add(field: &'static str, value: impl Into<String>) {
        baggage::add(&Span::current(), field, value.into());
    }
}

/// Records, once each, an incoming `traceparent` that does not parse and a
/// `tracestate` over its cap. Both are dropped and the request proceeds. A
/// no-op where the span cannot hold baggage (no exporter).
pub(crate) fn report_invalid(span: &Span, get: &impl Fn(&str) -> Option<String>, valid: bool) {
    if !baggage::installed(span) {
        return;
    }
    if !valid && get("traceparent").is_some_and(|v| !v.is_empty()) {
        emit_carrier_dropped("traceparent");
    }
    if get("tracestate").is_some_and(|v| v.len() > TRACESTATE_MAX) {
        emit_carrier_dropped("tracestate");
    }
}

pub(crate) fn extract_context(get: impl Fn(&str) -> Option<String>) -> opentelemetry::Context {
    let mut fields = HashMap::new();
    if let Some(value) = get("traceparent") {
        fields.insert("traceparent".to_owned(), value);
    }
    if let Some(value) = get("tracestate") {
        let value = shrink_tracestate(&value);
        if !value.is_empty() {
            fields.insert("tracestate".to_owned(), value.into_owned());
        }
    }
    TraceContextPropagator::new().extract(&Fields(&fields))
}

/// A `tracestate` within the cap, as W3C says to shorten one: over
/// `TRACESTATE_MAX`, entries longer than `TRACESTATE_ENTRY_MAX` go first, then
/// whole entries from the end. Under the cap it is returned as it is.
pub(crate) fn shrink_tracestate(value: &str) -> std::borrow::Cow<'_, str> {
    if value.len() <= TRACESTATE_MAX {
        return std::borrow::Cow::Borrowed(value);
    }
    let mut kept: Vec<&str> = value
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty() && entry.len() <= TRACESTATE_ENTRY_MAX)
        .collect();
    let joined = |entries: &[&str]| {
        entries
            .iter()
            .map(|e| e.len() + 1)
            .sum::<usize>()
            .saturating_sub(1)
    };
    while joined(&kept) > TRACESTATE_MAX {
        kept.pop();
    }
    std::borrow::Cow::Owned(kept.join(","))
}

fn emit_trace_missing(hop: &'static str) {
    emit_coded!(
        tracing::Level::WARN,
        TRACE_MISSING_CODE,
        ErrorClass::Infrastructure,
        module_path!(),
        hop = hop
    );
}

pub(crate) fn emit_baggage_dropped(count: usize) {
    emit_coded!(
        tracing::Level::WARN,
        BAGGAGE_DROPPED_CODE,
        ErrorClass::Logical,
        module_path!(),
        dropped = count as i64
    );
}

fn emit_carrier_dropped(header: &'static str) {
    emit_coded!(
        tracing::Level::WARN,
        CARRIER_DROPPED_CODE,
        ErrorClass::Logical,
        module_path!(),
        header = header
    );
}
