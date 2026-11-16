//! Trace carriers: capturing the current trace and baggage, and joining or
//! linking to one on the other side of a boundary.
//!
//! Without an exporter there is no span layer: `capture` returns an empty
//! carrier, the extract helpers change nothing, and `concord.trace.missing`
//! is never recorded.

use crate::coded::emit_coded;
use crate::init::EVENT_TARGET;
use crate::labels;
#[cfg(feature = "otel")]
use crate::propagation::Fields;
#[cfg(feature = "otel")]
use crate::{baggage, propagation};
use coded_error::ErrorClass;
use concord_telemetry_proto::TraceCarrier;
#[cfg(feature = "otel")]
use opentelemetry::propagation::TextMapPropagator;
#[cfg(feature = "otel")]
use opentelemetry::trace::TraceContextExt;
#[cfg(feature = "otel")]
use opentelemetry_sdk::propagation::TraceContextPropagator;
#[cfg(feature = "otel")]
use std::collections::HashMap;
#[cfg(feature = "otel")]
use std::sync::atomic::AtomicBool;
#[cfg(feature = "otel")]
use std::sync::atomic::Ordering;
use tracing::Span;
#[cfg(feature = "otel")]
use tracing_opentelemetry::OpenTelemetrySpanExt;

/// Code of the record emitted when a required hop gets no trace.
pub const TRACE_MISSING_CODE: &str = "concord.trace.missing";
/// Code of the record emitted when baggage entries were dropped.
pub const BAGGAGE_DROPPED_CODE: &str = "concord.trace.baggage_dropped";

/// Code of the record emitted when an incoming `traceparent` is malformed or
/// a `tracestate` is over its cap. The field `header` names which.
pub const CARRIER_DROPPED_CODE: &str = "concord.trace.carrier_dropped";

/// Code of the record emitted when `emit_error_at` drops a runtime field whose
/// name is on the never-log list. The field `field` names it; its value is not
/// recorded.
pub const FIELD_DROPPED_CODE: &str = "concord.error.field_dropped";

/// W3C Trace Context: a `tracestate` over this many characters is shortened.
#[cfg(feature = "otel")]
const TRACESTATE_MAX: usize = 512;
/// When shortening, entries longer than this go first.
#[cfg(feature = "otel")]
const TRACESTATE_ENTRY_MAX: usize = 128;

#[cfg(feature = "otel")]
static OTEL_ACTIVE: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "otel")]
pub(crate) fn set_otel_active(active: bool) {
    OTEL_ACTIVE.store(active, Ordering::Relaxed);
}

#[cfg(feature = "otel")]
pub(crate) fn otel_active() -> bool {
    OTEL_ACTIVE.load(Ordering::Relaxed)
}

#[cfg(not(feature = "otel"))]
pub(crate) fn otel_active() -> bool {
    false
}

/// The current span's trace context and baggage.
pub fn capture() -> TraceCarrier {
    let mut carrier = TraceCarrier::default();
    crate::propagation::inject(|key, value| match key {
        "traceparent" => carrier.traceparent = Some(value),
        "tracestate" => carrier.tracestate = Some(value),
        "baggage" => carrier.baggage = Some(value),
        _ => {}
    });
    carrier
}

#[cfg(feature = "otel")]
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
/// span entered, so it does too). On a span that was already entered, a
/// `traceparent` is refused (`set_parent` cannot re-parent) and so is the
/// baggage: the labels stay as they were. A carrier without a valid
/// `traceparent` leaves the span's contextual parent in place.
#[cfg(feature = "otel")]
pub fn parent_on(span: &Span, c: &TraceCarrier) {
    propagation::parent(span, getter(c));
}

/// `parent_on` with the otel feature off: nothing to parent.
#[cfg(not(feature = "otel"))]
pub fn parent_on(_span: &Span, _c: &TraceCarrier) {}

/// Gives `span` a link to the carrier's span context, in a trace of its own.
/// A link carries span context only, as in OpenTelemetry: the span's labels
/// are untouched. A caller that wants the linked span's labels calls
/// `context::add` for each.
#[cfg(feature = "otel")]
pub fn link_from(span: &Span, c: &TraceCarrier) {
    let context = extract_context(getter(c));
    let span_context = context.span().span_context().clone();
    report_invalid(span, &getter(c), span_context.is_valid());
    if span_context.is_valid() {
        span.add_link(span_context);
    }
}

/// `link_from` with the otel feature off: no span layer, so no link.
#[cfg(not(feature = "otel"))]
pub fn link_from(_span: &Span, _c: &TraceCarrier) {}

/// `parent_on` for a hop that must carry a trace. With an exporter active and
/// no valid `traceparent` it records `concord.trace.missing` with the field
/// `hop`.
pub fn extract_required(hop: &'static str, span: &Span, c: &TraceCarrier) {
    #[cfg(feature = "otel")]
    {
        let valid = propagation::parent(span, getter(c));
        let malformed = c.traceparent.as_deref().is_some_and(|v| !v.is_empty());
        if !valid && !malformed && otel_active() {
            emit_trace_missing(hop);
        }
    }
    #[cfg(not(feature = "otel"))]
    {
        let _ = (hop, span, c);
    }
}

/// `parent_on` for an entry point that may legitimately get no trace. It is
/// silent when the carrier is empty. `name` is recorded on the span as
/// `ingress`.
#[cfg(feature = "otel")]
pub fn extract_ingress(name: &'static str, span: &Span, c: &TraceCarrier) {
    propagation::parent(span, getter(c));
    span.set_attribute("ingress", name);
}

/// `extract_ingress` with the otel feature off: no span layer, so nothing is
/// parented and no attribute is set.
#[cfg(not(feature = "otel"))]
pub fn extract_ingress(_name: &'static str, _span: &Span, _c: &TraceCarrier) {}

/// `c` with label `key` set to `value`: the entry is replaced in place when
/// the carrier's baggage has the key, otherwise appended. Everything else in
/// the carrier is kept; the baggage is rewritten from its valid entries
/// (properties and malformed entries are dropped, as `labels::parse` does).
/// A label that is not valid (empty or over-long key, key in the reserved
/// `concord.` namespace, value over 256 bytes, a 25th entry, or a header that
/// would pass its cap) leaves `c` unchanged and records
/// `concord.trace.baggage_dropped`. Pure apart from that record.
pub fn with_label(c: &TraceCarrier, key: &str, value: &str) -> TraceCarrier {
    let mut entries = c.baggage.as_deref().map(labels::parse).unwrap_or_default();
    let replaced = match entries.iter_mut().find(|(k, _)| k == key) {
        Some(entry) => {
            entry.1 = value.to_owned();
            true
        }
        None => {
            entries.push((key.to_owned(), value.to_owned()));
            false
        }
    };
    let (header, left_out) = labels::format_counted(&entries);
    if !labels::valid(key, value)
        || (!replaced && entries.len() > context::MAX_ENTRIES)
        || left_out > 0
    {
        emit_baggage_dropped(1);
        return c.clone();
    }
    TraceCarrier {
        baggage: header,
        ..c.clone()
    }
}

/// `c` without label `key`. A carrier whose baggage does not have the key is
/// returned unchanged; otherwise the baggage is rewritten from its valid
/// entries without it, and is `None` when none are left.
pub fn without_label(c: &TraceCarrier, key: &str) -> TraceCarrier {
    let Some(header) = c.baggage.as_deref() else {
        return c.clone();
    };
    let mut entries = labels::parse(header);
    let before = entries.len();
    entries.retain(|(k, _)| k != key);
    if entries.len() == before {
        return c.clone();
    }
    TraceCarrier {
        baggage: labels::format(&entries),
        ..c.clone()
    }
}

/// Parents `span` on the carrier when it has a `traceparent`, otherwise on the
/// `TRACEPARENT` and `BAGGAGE` environment variables. A carrier with a
/// `traceparent` wins even when the environment has one. Call it before
/// `span` is first entered; without the `otel` feature it does nothing.
pub fn parent_on_or_env(span: &Span, c: Option<&TraceCarrier>) {
    match c.filter(|c| c.traceparent.as_deref().is_some_and(|t| !t.is_empty())) {
        Some(c) => parent_on(span, c),
        None => crate::propagation::set_parent_from_env(span),
    }
}

/// A span with no parent: it starts a trace of its own.
pub fn root(name: &'static str) -> Span {
    tracing::info_span!(target: EVENT_TARGET, parent: None, "root", otel.name = name)
}

/// The labels of the current span's baggage. See `carrier` for carrying them.
pub mod context {
    #[cfg(feature = "otel")]
    use super::*;

    /// The most labels one span's baggage holds.
    pub const MAX_ENTRIES: usize = 24;

    /// Adds a label to the current span's baggage and records it as a span
    /// attribute. Children and outgoing carriers inherit it. Over the baggage
    /// caps it is dropped and recorded.
    #[cfg(feature = "otel")]
    pub fn add(field: &'static str, value: impl Into<String>) {
        baggage::add(&Span::current(), field, value.into());
    }

    /// `add` with the otel feature off: no baggage, so nothing is added.
    #[cfg(not(feature = "otel"))]
    pub fn add(_field: &'static str, _value: impl Into<String>) {}

    /// Adds the labels of a `baggage` header to the current span, as `add`
    /// does for each entry in header order: a key the span already holds is
    /// overwritten, an unchanged value is not stamped again, and children and
    /// outgoing carriers inherit the result. The header is parsed with the
    /// same caps and malformed-entry handling as an incoming carrier (the
    /// reserved `concord.` namespace is dropped), and the entries dropped,
    /// by the parse or by the span's caps, are recorded once as
    /// `concord.trace.baggage_dropped`. An empty header, or a span that
    /// cannot hold baggage (no exporter), does nothing.
    ///
    /// A key repeated within one header keeps its last value, positioned where
    /// `parse` puts it. At the cap this can differ from calling `add` per
    /// entry: `parse` has already applied the cap to the header.
    ///
    /// This is an explicit add, so it works on a span that was already
    /// entered. A span that is not entered is not the current span: call it
    /// inside `span.in_scope(|| ..)`, as with `add`.
    #[cfg(feature = "otel")]
    pub fn add_from_baggage(header: &str) {
        baggage::add_header(&Span::current(), header);
    }

    /// `add_from_baggage` with the otel feature off: no baggage, so nothing is added.
    #[cfg(not(feature = "otel"))]
    pub fn add_from_baggage(_header: &str) {}
}

/// Records, once each, an incoming `traceparent` that does not parse and a
/// `tracestate` over its cap. Both are dropped and the request proceeds. A
/// no-op where the span cannot hold baggage (no exporter).
#[cfg(feature = "otel")]
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

#[cfg(feature = "otel")]
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
#[cfg(feature = "otel")]
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

#[cfg(feature = "otel")]
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

pub(crate) fn emit_field_dropped(name: &str) {
    emit_coded!(
        tracing::Level::WARN,
        FIELD_DROPPED_CODE,
        ErrorClass::Logical,
        module_path!(),
        field = name
    );
}

#[cfg(feature = "otel")]
fn emit_carrier_dropped(header: &'static str) {
    emit_coded!(
        tracing::Level::WARN,
        CARRIER_DROPPED_CODE,
        ErrorClass::Logical,
        module_path!(),
        header = header
    );
}
