//! Baggage: labels that travel with a trace, kept in span extensions.
//!
//! Baggage is labels only, never authorization or routing. The caps are
//! [`MAX_ENTRIES`] (24) entries, 64 bytes a key, 256 bytes a value (`labels`)
//! and 8192 bytes for the whole header (`labels` parses and formats it). An
//! invalid entry is dropped and recorded once per header. The whole
//! `concord.` namespace is reserved: `context::add` refuses it and an incoming
//! header's entries in it are dropped.
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
//! Writers hold no lock across `set_attribute`: see `BaggageMap`.
//! The layer tracks nothing else, has no `on_record`, and never runs user
//! `Debug` or `Display`.
//!
//! Limits: a span created but never entered gets no inherited labels, and
//! labels land after the span starts, so samplers and `SpanProcessor::on_start`
//! never see them (tracing-opentelemetry's `OtelData` is private). Every changed
//! label value is one stamped attribute until the span ends, so `init` raises
//! the provider's limit to 512 attributes per span; the collapse cannot
//! recover attributes the SDK has already dropped. Logs are different:
//! `BaggageLogProcessor` keeps an event's own field over a label of the same
//! name, without saying so.
//!
//! The `Registry` pushes a span onto its stack before any layer's `on_enter`
//! runs (`Layered::enter` calls the inner subscriber first), so
//! `Span::current()` is the entered span there.

use opentelemetry::{Key, Value};
use std::sync::Arc;
#[cfg(feature = "testing")]
use std::sync::{Mutex, PoisonError};
use tracing::span::Id;
use tracing::Span;
use tracing_opentelemetry::OpenTelemetrySpanExt;
use tracing_subscriber::layer::{Context as LayerContext, Layer};
use tracing_subscriber::registry::{LookupSpan, Registry};

/// The most entries one span's baggage holds. Defined in `carrier::context`.
pub use crate::carrier::context::MAX_ENTRIES;
use crate::labels::{parse_counted as parse, valid};

/// The attribute prefix under which labels are stamped. Reserved: no label
/// or incoming baggage key may start with it.
pub(crate) use concord_fields::LABEL_PREFIX;

#[cfg(feature = "testing")]
pub(crate) static STAMPS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Called by `on_enter` after it has decided to stamp and before it stamps, so
/// a test can park a thread there (the `testing` feature).
#[cfg(feature = "testing")]
pub(crate) static ENTER_HOOK: Mutex<Option<Arc<dyn Fn() + Send + Sync>>> = Mutex::new(None);

fn enter_hook() {
    #[cfg(feature = "testing")]
    {
        let hook = ENTER_HOOK
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(hook) = hook {
            hook();
        }
    }
}

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
///
/// Stamping takes no lock across `set_attribute`. A writer changes the map
/// under the extensions write lock and bumps `generation`, releases it,
/// stamps, then re-reads under a read lock: if the generation moved, another
/// writer changed the map meanwhile, and it stamps the newer values again.
/// `SpanData.attributes` keeps write order, so the last stamp of a key is the
/// one the collapse keeps, and the final stamp of every key is its latest
/// value. The extensions lock is never held while user code (`Debug`) runs
/// under it elsewhere: tracing-subscriber's fmt layer does that.
#[derive(Clone, Debug, Default)]
pub(crate) struct BaggageMap {
    pub labels: Arc<Vec<Label>>,
    /// Bumped by every change to `labels`.
    generation: u64,
    /// True once every label in `labels` has been stamped on the span.
    stamped: bool,
    /// True once the span was entered. Distinct from `stamped`: `set` stamps
    /// labels on a span that has not been entered yet.
    entered: bool,
}

type Snapshot = (u64, Arc<Vec<Label>>);

/// Stamps `first`, then keeps stamping the latest labels while the map's
/// generation moves. `read` takes the current snapshot under a read lock.
fn stamp_until_stable(
    span: &Span,
    first: &[Label],
    mut generation: u64,
    read: impl Fn() -> Option<Snapshot>,
) {
    for label in first {
        label.stamp(span);
    }
    while let Some((now, labels)) = read() {
        if now == generation {
            return;
        }
        generation = now;
        for label in labels.iter() {
            label.stamp(span);
        }
    }
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
                    generation: 0,
                    stamped: false,
                    entered: false,
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
                Some(map) if !map.entered && !map.labels.is_empty() => {}
                _ => return,
            }
        }
        // The thread's current span must be this one: the id alone can match a
        // span of another (scoped) dispatcher.
        let current = Span::current();
        if current.id().as_ref() != Some(id)
            || current.metadata().map(|m| m.callsite()) != Some(span.metadata().callsite())
        {
            return;
        }
        let (generation, labels) = {
            let mut extensions = span.extensions_mut();
            let Some(map) = extensions.get_mut::<BaggageMap>() else {
                return;
            };
            if map.entered {
                return;
            }
            map.entered = true;
            if map.stamped {
                // `set` already stamped these labels.
                return;
            }
            map.stamped = true;
            (map.generation, Arc::clone(&map.labels))
        };
        // No lock is held: set_attribute takes the extensions lock itself.
        enter_hook();
        stamp_until_stable(&current, &labels, generation, || {
            span.extensions()
                .get::<BaggageMap>()
                .map(|map| (map.generation, Arc::clone(&map.labels)))
        });
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

fn snapshot(span: &Span) -> Option<Snapshot> {
    with_extensions(span, |s| {
        s.extensions()
            .get::<BaggageMap>()
            .map(|map| (map.generation, Arc::clone(&map.labels)))
    })
    .flatten()
}

/// Replaces the span's baggage with `new` and stamps every label.
pub(crate) fn set(span: &Span, new: Vec<(String, String)>) {
    let replaced = with_extensions(span, |s| {
        let labels: Arc<Vec<Label>> = Arc::new(
            new.iter()
                .map(|(key, value)| Label::new(key, value))
                .collect(),
        );
        let mut extensions = s.extensions_mut();
        let (previous, entered) = extensions
            .get_mut::<BaggageMap>()
            .map_or((0, false), |map| (map.generation, map.entered));
        let generation = previous + 1;
        extensions.replace(BaggageMap {
            labels: Arc::clone(&labels),
            generation,
            stamped: true,
            entered,
        });
        (generation, labels)
    });
    if let Some((generation, labels)) = replaced {
        stamp_until_stable(span, &labels, generation, || snapshot(span));
    }
}

enum Added {
    Dropped,
    Unchanged,
    Changed(Label, u64),
}

/// Adds one label to `span`, subject to the caps, and stamps it. Returns
/// false when it was dropped (and recorded). A key in the reserved `concord.`
/// namespace is dropped. A value equal to the label's current one changes and
/// stamps nothing.
pub(crate) fn add(span: &Span, key: &str, value: String) -> bool {
    let added = try_add(span, key, value);
    if added == Some(false) {
        // After every guard is released: this emits a tracing event.
        crate::carrier::emit_baggage_dropped(1);
    }
    added.unwrap_or(false)
}

/// Adds the labels of a `baggage` header to `span`, in header order, each as
/// `add` would. Entries `parse` drops and entries the span refuses are
/// counted and recorded once, after every guard is released. Does nothing
/// where the span cannot hold baggage.
pub(crate) fn add_header(span: &Span, header: &str) {
    if !installed(span) {
        return;
    }
    let (entries, mut dropped) = parse(header);
    for (key, value) in entries {
        if try_add(span, &key, value) == Some(false) {
            dropped += 1;
        }
    }
    if dropped > 0 {
        crate::carrier::emit_baggage_dropped(dropped);
    }
}

/// `add` without the record: `Some(true)` when kept, `Some(false)` when
/// refused by a cap or the reserved namespace, `None` where the span cannot
/// hold baggage.
fn try_add(span: &Span, key: &str, value: String) -> Option<bool> {
    let outcome = with_extensions(span, |s| {
        let mut extensions = s.extensions_mut();
        if extensions.get_mut::<BaggageMap>().is_none() {
            // `add` runs on the entered span, so there is nothing to stamp later.
            extensions.insert(BaggageMap {
                stamped: true,
                ..BaggageMap::default()
            });
        }
        let map = extensions.get_mut::<BaggageMap>()?;
        map.entered = true;
        let fits = valid(key, &value)
            && (map.labels.len() < MAX_ENTRIES || map.labels.iter().any(|l| l.key.as_str() == key));
        if !fits {
            return Some(Added::Dropped);
        }
        if map
            .labels
            .iter()
            .any(|l| l.key.as_str() == key && *l.value == *value)
        {
            return Some(Added::Unchanged);
        }
        let label = Label::new(key, &value);
        let labels = Arc::make_mut(&mut map.labels);
        labels.retain(|l| l.key.as_str() != key);
        labels.push(label.clone());
        map.generation += 1;
        Some(Added::Changed(label, map.generation))
    })
    .flatten();
    match outcome {
        Some(Added::Changed(label, generation)) => {
            stamp_until_stable(span, &[label], generation, || snapshot(span));
            Some(true)
        }
        Some(Added::Unchanged) => Some(true),
        Some(Added::Dropped) => Some(false),
        None => None,
    }
}

/// True once `span` was entered with labels: its attributes are fixed by then.
/// Before that, a second `parent_on` takes both the new `traceparent` and the
/// new baggage.
fn entered_with_labels(span: &Span) -> bool {
    with_extensions(span, |s| {
        s.extensions()
            .get::<BaggageMap>()
            .is_some_and(|map| map.entered && !map.labels.is_empty())
    })
    .unwrap_or(false)
}

/// Parses an incoming header into `span`'s baggage, recording what was
/// dropped. A no-op where the span cannot hold baggage, and on a span that was
/// already entered with labels: like `set_parent`, it is refused.
pub(crate) fn ingest(span: &Span, header: &str) {
    if !installed(span) || entered_with_labels(span) {
        return;
    }
    let (kept, dropped) = parse(header);
    if dropped > 0 {
        crate::carrier::emit_baggage_dropped(dropped);
    }
    set(span, kept);
}

/// The entries as a `baggage` header, in order, stopping before the header
/// would pass its cap. Entries left out are recorded as one
/// `baggage_dropped`. `None` when there is nothing to send.
pub(crate) fn format(entries: &[(String, String)]) -> Option<String> {
    let (header, left_out) = crate::labels::format_counted(entries);
    if left_out > 0 {
        crate::carrier::emit_baggage_dropped(left_out);
    }
    header
}
