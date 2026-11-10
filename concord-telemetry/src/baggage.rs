//! Baggage: labels that travel with a trace, kept in span extensions.
//!
//! Baggage is labels only, never authorization or routing. The caps are
//! `MAX_ENTRIES` entries, `MAX_KEY` bytes a key, `MAX_VALUE` bytes a value and
//! `MAX_HEADER` bytes for the whole header. An invalid entry is dropped and
//! recorded once per header.
//!
//! `BaggageLayer` copies a span's baggage to its children. tracing-opentelemetry
//! keeps its span data private, so the labels reach OpenTelemetry as span
//! attributes through the public `OpenTelemetrySpanExt::set_attribute`, written
//! once per label per span: by `BaggageLayer::on_enter` for inherited labels,
//! directly by `context::add` and `carrier::parent_on`.
//!
//! The `Registry` pushes a span onto its stack before any layer's `on_enter`
//! runs (`Layered::enter` calls the inner subscriber first), so
//! `Span::current()` is the entered span there.

use opentelemetry::baggage::BaggageExt;
use opentelemetry::propagation::{Extractor, Injector, TextMapPropagator};
use opentelemetry::Context;
use opentelemetry_sdk::propagation::BaggagePropagator;
use std::collections::HashMap;
use tracing::span::Id;
use tracing::Span;
use tracing_opentelemetry::OpenTelemetrySpanExt;
use tracing_subscriber::layer::{Context as LayerContext, Layer};
use tracing_subscriber::registry::{LookupSpan, Registry};

pub(crate) const MAX_ENTRIES: usize = 16;
pub(crate) const MAX_KEY: usize = 64;
pub(crate) const MAX_VALUE: usize = 256;
pub(crate) const MAX_HEADER: usize = 8192;

/// The baggage of one span, in insertion order.
#[derive(Clone, Debug, Default)]
pub(crate) struct BaggageMap(pub Vec<(String, String)>);

/// The labels already written to the span's OpenTelemetry attributes.
#[derive(Default)]
struct Stamped(Vec<(String, String)>);

/// Copies baggage from parent to child and stamps it onto the span's
/// attributes. Register it after the tracing-opentelemetry layer.
pub(crate) struct BaggageLayer;

impl<S> Layer<S> for BaggageLayer
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(
        &self,
        _attrs: &tracing::span::Attributes<'_>,
        id: &Id,
        ctx: LayerContext<'_, S>,
    ) {
        let Some(span) = ctx.span(id) else { return };
        let inherited = span
            .parent()
            .and_then(|parent| parent.extensions().get::<BaggageMap>().cloned());
        if let Some(map) = inherited {
            span.extensions_mut().replace(map);
        }
    }

    fn on_enter(&self, id: &Id, ctx: LayerContext<'_, S>) {
        let Some(span) = ctx.span(id) else { return };
        let pending = pending(&span.extensions());
        if pending.is_empty() {
            return;
        }
        let current = Span::current();
        if current.id().as_ref() != Some(id) {
            return;
        }
        {
            let mut extensions = span.extensions_mut();
            if let Some(stamped) = extensions.get_mut::<Stamped>() {
                stamped.0.extend(pending.iter().cloned());
            } else {
                extensions.insert(Stamped(pending.clone()));
            }
        }
        // The extensions lock is released: set_attribute takes the same lock.
        for (key, value) in pending {
            current.set_attribute(key, value);
        }
    }
}

fn pending(extensions: &tracing_subscriber::registry::Extensions<'_>) -> Vec<(String, String)> {
    let Some(map) = extensions.get::<BaggageMap>() else {
        return Vec::new();
    };
    let stamped = extensions.get::<Stamped>();
    map.0
        .iter()
        .filter(|entry| stamped.is_none_or(|s| !s.0.contains(entry)))
        .cloned()
        .collect()
}

/// Runs `f` on the span's extensions, if the subscriber is a `Registry`
/// carrying a `BaggageLayer`. Otherwise the baggage helpers are no-ops.
fn with_extensions<R>(
    span: &Span,
    f: impl FnOnce(&tracing_subscriber::registry::SpanRef<'_, Registry>) -> R,
) -> Option<R> {
    span.with_subscriber(|(id, dispatch)| {
        dispatch.downcast_ref::<BaggageLayer>()?;
        let registry = dispatch.downcast_ref::<Registry>()?;
        let span = registry.span(id)?;
        Some(f(&span))
    })
    .flatten()
}

/// True when `span` can hold baggage.
pub(crate) fn installed(span: &Span) -> bool {
    with_extensions(span, |_| ()).is_some()
}

/// The span's baggage entries, empty when there is none.
pub(crate) fn entries(span: &Span) -> Vec<(String, String)> {
    with_extensions(span, |s| {
        s.extensions()
            .get::<BaggageMap>()
            .map(|map| map.0.clone())
            .unwrap_or_default()
    })
    .unwrap_or_default()
}

/// Replaces the span's baggage with `new` and stamps the new labels.
pub(crate) fn set(span: &Span, new: Vec<(String, String)>) {
    let fresh = with_extensions(span, |s| {
        let mut extensions = s.extensions_mut();
        extensions.replace(BaggageMap(new.clone()));
        let mut stamped = extensions.remove::<Stamped>().unwrap_or_default();
        let fresh: Vec<_> = new
            .iter()
            .filter(|entry| !stamped.0.contains(entry))
            .cloned()
            .collect();
        stamped.0.extend(fresh.iter().cloned());
        extensions.insert(stamped);
        fresh
    })
    .unwrap_or_default();
    for (key, value) in fresh {
        span.set_attribute(key, value);
    }
}

/// Adds one label to `span`, subject to the caps. Returns false when it was
/// dropped (and recorded).
pub(crate) fn add(span: &Span, key: &str, value: String) -> bool {
    let outcome = with_extensions(span, |s| {
        let mut extensions = s.extensions_mut();
        let mut map = extensions.remove::<BaggageMap>().unwrap_or_default();
        let fits = key.len() <= MAX_KEY
            && !key.is_empty()
            && value.len() <= MAX_VALUE
            && (map.0.len() < MAX_ENTRIES || map.0.iter().any(|(k, _)| k == key));
        if fits {
            map.0.retain(|(k, _)| k != key);
            map.0.push((key.to_owned(), value.clone()));
        }
        extensions.insert(map);
        if fits {
            let mut stamped = extensions.remove::<Stamped>().unwrap_or_default();
            stamped.0.push((key.to_owned(), value.clone()));
            extensions.insert(stamped);
        }
        fits
    });
    match outcome {
        Some(true) => {
            span.set_attribute(key.to_owned(), value);
            true
        }
        Some(false) => {
            crate::carrier::emit_baggage_dropped(1);
            false
        }
        None => false,
    }
}

/// Parses an incoming header into `span`'s baggage, recording what was
/// dropped. A no-op where the span cannot hold baggage.
pub(crate) fn ingest(span: &Span, header: &str) {
    if !installed(span) {
        return;
    }
    let (kept, dropped) = parse(header);
    if dropped > 0 {
        crate::carrier::emit_baggage_dropped(dropped);
    }
    set(span, kept);
}

/// The valid entries of a `baggage` header, and how many were dropped.
pub(crate) fn parse(header: &str) -> (Vec<(String, String)>, usize) {
    let segments: Vec<&str> = header.split(',').filter(|s| !s.trim().is_empty()).collect();
    if header.len() > MAX_HEADER {
        return (Vec::new(), segments.len());
    }
    let mut kept: Vec<(String, String)> = Vec::new();
    let mut dropped = 0;
    for segment in segments {
        match parse_entry(segment.trim()) {
            Some((key, value))
                if kept.len() < MAX_ENTRIES || kept.iter().any(|(k, _)| *k == key) =>
            {
                kept.retain(|(k, _)| *k != key);
                kept.push((key, value));
            }
            _ => dropped += 1,
        }
    }
    (kept, dropped)
}

fn parse_entry(segment: &str) -> Option<(String, String)> {
    let mut parts = segment.split(';');
    let pair = parts.next()?;
    // Properties are not kept, but an empty one makes the entry malformed.
    if parts.any(|property| property.trim().is_empty()) {
        return None;
    }
    if pair.matches('=').count() != 1 {
        return None;
    }
    let mut fields = HashMap::new();
    fields.insert("baggage".to_owned(), pair.to_owned());
    let context = BaggagePropagator::new().extract(&Fields(&fields));
    let mut entries = context.baggage().iter();
    let (key, (value, _)) = entries.next()?;
    if entries.next().is_some() {
        return None;
    }
    let (key, value) = (key.as_str().to_owned(), value.as_str().to_owned());
    (!key.is_empty() && key.len() <= MAX_KEY && value.len() <= MAX_VALUE).then_some((key, value))
}

/// The entries as a `baggage` header, in order, stopping before the header
/// would pass its cap. Entries left out are recorded as one
/// `baggage_dropped`. `None` when there is nothing to send.
pub(crate) fn format(entries: &[(String, String)]) -> Option<String> {
    let mut header = String::new();
    let mut written = 0;
    for (key, value) in entries.iter().take(MAX_ENTRIES) {
        let context =
            Context::new().with_baggage([opentelemetry::KeyValue::new(key.clone(), value.clone())]);
        let mut out = HashMap::new();
        BaggagePropagator::new().inject_context(&context, &mut Out(&mut out));
        let Some(piece) = out.remove("baggage").filter(|p| !p.is_empty()) else {
            continue;
        };
        let extra = piece.len() + usize::from(!header.is_empty());
        if header.len() + extra > MAX_HEADER {
            break;
        }
        if !header.is_empty() {
            header.push(',');
        }
        header.push_str(&piece);
        written += 1;
    }
    let left_out = entries.len() - written;
    if left_out > 0 {
        crate::carrier::emit_baggage_dropped(left_out);
    }
    (!header.is_empty()).then_some(header)
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

struct Out<'a>(&'a mut HashMap<String, String>);

impl Injector for Out<'_> {
    fn set(&mut self, key: &str, value: String) {
        self.0.insert(key.to_owned(), value);
    }
}
