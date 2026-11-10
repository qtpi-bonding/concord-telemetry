//! Baggage: labels that travel with a trace, kept in span extensions.
//!
//! Baggage is labels only, never authorization or routing. The caps are
//! [`MAX_ENTRIES`] (24) entries, `MAX_KEY` (64) bytes a key, `MAX_VALUE` (256)
//! bytes a value and `MAX_HEADER` (8192) bytes for the whole header. An
//! invalid entry is dropped and recorded once per header. The prefix
//! `concord.label.` is reserved: `context::add` refuses it and an incoming
//! header's entries with it are dropped.
//!
//! `BaggageLayer` copies a span's labels to its children by sharing one
//! `Arc`. tracing-opentelemetry keeps its span data private, so the labels
//! reach OpenTelemetry as span attributes through the public
//! `OpenTelemetrySpanExt::set_attribute`, under the key
//! `concord.label.<key>`, never the bare key. They are written when the span
//! is first entered (inherited labels) and on the spot by `context::add` and
//! `carrier::parent_on`. The `LabelCollapse` span processor resolves them
//! when the span ends: see `label_collapse`.
//!
//! The layer keeps one flag per span, `stamped`. After the first entry
//! `on_enter` reads it and returns: no clone, no allocation, no write lock.
//! The layer tracks nothing else, has no `on_record`, and never runs user
//! `Debug` or `Display`.
//!
//! Limits: a span created but never entered gets no inherited labels, and
//! labels land after the span starts, so samplers and `SpanProcessor::on_start`
//! never see them (tracing-opentelemetry's `OtelData` is private). Stamped
//! labels count against the SDK's 128 attributes per span, and the collapse
//! cannot recover attributes the SDK has already dropped. Logs are different:
//! `BaggageLogProcessor` keeps an event's own field over a label of the same
//! name, without saying so.
//!
//! The `Registry` pushes a span onto its stack before any layer's `on_enter`
//! runs (`Layered::enter` calls the inner subscriber first), so
//! `Span::current()` is the entered span there.

use opentelemetry::baggage::BaggageExt;
use opentelemetry::propagation::{Extractor, Injector, TextMapPropagator};
use opentelemetry::{Context, Key, Value};
use opentelemetry_sdk::propagation::BaggagePropagator;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use tracing::span::Id;
use tracing::Span;
use tracing_opentelemetry::OpenTelemetrySpanExt;
use tracing_subscriber::layer::{Context as LayerContext, Layer};
use tracing_subscriber::registry::{LookupSpan, Registry};

/// The most entries one span's baggage holds.
pub const MAX_ENTRIES: usize = 24;
pub(crate) const MAX_KEY: usize = 64;
pub(crate) const MAX_VALUE: usize = 256;
pub(crate) const MAX_HEADER: usize = 8192;

/// The attribute prefix under which labels are stamped. Reserved: no label
/// or incoming baggage key may start with it.
pub(crate) const LABEL_PREFIX: &str = "concord.label.";

#[cfg(feature = "testing")]
pub(crate) static STAMPS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// One label. Cloning it bumps reference counts and allocates nothing.
#[derive(Clone, Debug)]
pub(crate) struct Label {
    /// The key as the carrier and logs have it.
    pub key: Key,
    /// `concord.label.<key>`, the key the span attribute is stamped under.
    stamp_key: Key,
    pub value: Arc<str>,
}

impl Label {
    fn new(key: &str, value: &str) -> Self {
        let stamp_key: Arc<str> = Arc::from(format!("{LABEL_PREFIX}{key}"));
        Self {
            key: Key::from(Arc::<str>::from(key)),
            stamp_key: Key::from(stamp_key),
            value: Arc::from(value),
        }
    }

    fn stamp(&self, span: &Span) {
        #[cfg(feature = "testing")]
        STAMPS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        span.set_attribute(self.stamp_key.clone(), Value::from(Arc::clone(&self.value)));
    }
}

/// The baggage of one span, in insertion order. A child shares its parent's
/// labels until either changes them.
#[derive(Clone, Debug, Default)]
pub(crate) struct BaggageMap {
    pub labels: Arc<Vec<Label>>,
    /// True once every label in `labels` has been stamped on the span.
    stamped: bool,
}

/// Labels written to a span's attributes in the order the map changes: a static
/// array of locks indexed by span id, held from the label-map update until
/// `set_attribute` returns. `SpanData.attributes` keeps write order, so the
/// last stamp of a key is the one the collapse keeps. Lock order: this lock,
/// then the span's extensions, then `OtelData`. No user code runs under it.
static STAMP_LOCKS: [Mutex<()>; 64] = [const { Mutex::new(()) }; 64];

fn stamp_lock(id: &Id) -> MutexGuard<'static, ()> {
    STAMP_LOCKS[(id.into_u64() % 64) as usize]
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

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
        let inherited = span.parent().and_then(|parent| {
            parent
                .extensions()
                .get::<BaggageMap>()
                .map(|map| BaggageMap {
                    labels: Arc::clone(&map.labels),
                    stamped: false,
                })
        });
        if let Some(map) = inherited {
            span.extensions_mut().replace(map);
        }
    }

    fn on_enter(&self, id: &Id, ctx: LayerContext<'_, S>) {
        let Some(span) = ctx.span(id) else { return };
        {
            let extensions = span.extensions();
            match extensions.get::<BaggageMap>() {
                Some(map) if !map.stamped && !map.labels.is_empty() => {}
                _ => return,
            }
        }
        let current = Span::current();
        if current.id().as_ref() != Some(id) {
            return;
        }
        let _stamping = stamp_lock(id);
        let labels = {
            let mut extensions = span.extensions_mut();
            let Some(map) = extensions.get_mut::<BaggageMap>() else {
                return;
            };
            if map.stamped {
                return;
            }
            map.stamped = true;
            Arc::clone(&map.labels)
        };
        // The extensions lock is released: set_attribute takes the same lock.
        for label in labels.iter() {
            label.stamp(&current);
        }
    }
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
            .map(|map| {
                map.labels
                    .iter()
                    .map(|label| (label.key.to_string(), label.value.to_string()))
                    .collect()
            })
            .unwrap_or_default()
    })
    .unwrap_or_default()
}

/// Replaces the span's baggage with `new` and stamps every label.
pub(crate) fn set(span: &Span, new: Vec<(String, String)>) {
    let Some(id) = span.id() else { return };
    let _stamping = stamp_lock(&id);
    let labels = with_extensions(span, |s| {
        let labels: Arc<Vec<Label>> = Arc::new(
            new.iter()
                .map(|(key, value)| Label::new(key, value))
                .collect(),
        );
        s.extensions_mut().replace(BaggageMap {
            labels: Arc::clone(&labels),
            stamped: true,
        });
        labels
    })
    .unwrap_or_default();
    for label in labels.iter() {
        label.stamp(span);
    }
}

/// Adds one label to `span`, subject to the caps, and stamps it. Returns
/// false when it was dropped (and recorded). A key with the reserved prefix
/// is dropped.
pub(crate) fn add(span: &Span, key: &str, value: String) -> bool {
    let Some(id) = span.id() else { return false };
    let _stamping = stamp_lock(&id);
    let outcome = with_extensions(span, |s| {
        let mut extensions = s.extensions_mut();
        if extensions.get_mut::<BaggageMap>().is_none() {
            extensions.insert(BaggageMap::default());
        }
        let map = extensions.get_mut::<BaggageMap>()?;
        let fits = key.len() <= MAX_KEY
            && !key.is_empty()
            && !key.starts_with(LABEL_PREFIX)
            && value.len() <= MAX_VALUE
            && (map.labels.len() < MAX_ENTRIES || map.labels.iter().any(|l| l.key.as_str() == key));
        if !fits {
            return Some(None);
        }
        let label = Label::new(key, &value);
        let labels = Arc::make_mut(&mut map.labels);
        labels.retain(|l| l.key.as_str() != key);
        labels.push(label.clone());
        Some(Some(label))
    })
    .flatten();
    match outcome {
        Some(Some(label)) => {
            label.stamp(span);
            true
        }
        Some(None) => {
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
                if !key.starts_with(LABEL_PREFIX)
                    && (kept.len() < MAX_ENTRIES || kept.iter().any(|(k, _)| *k == key)) =>
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
