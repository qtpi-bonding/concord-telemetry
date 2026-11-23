//! Causes: the registered ids (an action, a turn, a message, an instance) a
//! request was caused by, captured once at its entry point and carried on
//! every record and span beneath it.
//!
//! Always built, without an exporter. Causes are not labels: they are never
//! baggage and never stamped as `concord.label.*`. The entry point snapshots
//! the causes its carrier held, once, into the span's extensions (the
//! `Captured` snapshot). A later `context::add` changes the labels but never
//! the snapshot, so a handler that writes its own `action_id` still reports the
//! caller's cause. `capture_incoming` reads the nearest snapshot at or above the
//! current span; an empty snapshot is a boundary, so a span whose carrier had
//! no baggage has no causes even under an entry that did.
//!
//! The snapshot travels with the task, not the thread: `scope` holds it as a
//! task-local across awaits, and a task that `tokio::spawn`s does not inherit
//! it (the spawned future starts with none). Hand a spawned task its causes
//! explicitly with `scope`. `scope_with` extends the incoming causes for one
//! inner call only.
//!
//! `excluding` removes exactly the (kind, id) pairs it is given. A record that
//! must not name its own cause (a cause that is the subject of the record) takes
//! its containers by excluding it; which records contain which is the caller's
//! table, not this module's.
//!
//! Limits: at most `MAX_CAUSES` (8) causes; a kind must be in the shared
//! `[causes]` table of `concord-fields`; an id is 1..=`MAX_ID` (256) bytes. Any
//! drop is recorded once per capture or construction, as `CAUSES_DROPPED_CODE`.
//! The JSON layout (`to_json`) is the `causes` span attribute, which exists only
//! with the `otel` feature.

use crate::baggage;
use crate::coded::emit_coded;
use crate::labels;
use crate::TraceCarrier;
use coded_error::ErrorClass;
use concord_fields::Parsed;
use pin_project_lite::pin_project;
use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};
use std::task::{Context, Poll};
use tracing::Span;

pub use concord_telemetry_proto::v1::Cause;

/// The most causes one snapshot holds.
pub const MAX_CAUSES: usize = 8;
/// The most bytes one cause id holds.
pub const MAX_ID: usize = 256;
/// One coded WARN per capture or construction that dropped anything; field `dropped`.
pub const CAUSES_DROPPED_CODE: &str = "concord.trace.causes_dropped";

tokio::task_local! {
    static INCOMING: Causes;
}

/// An immutable, ordered set of causes. Cheap to clone.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Causes(Arc<[Cause]>);

impl Causes {
    /// Keeps order. Drops: a kind not in the shared [causes] table, an empty id,
    /// an id over MAX_ID bytes, a repeat of (kind, id), and every cause past
    /// MAX_CAUSES. One WARN with the count of everything dropped except repeats.
    pub fn new(causes: impl IntoIterator<Item = Cause>) -> Causes {
        let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
        let mut kept: Vec<Cause> = Vec::new();
        let mut dropped = 0usize;
        for cause in causes {
            if kept.len() >= MAX_CAUSES {
                // Past the cap nothing is kept: move the pair into `seen` for the repeat check, no clone.
                if seen.insert((cause.kind, cause.id)) {
                    dropped += 1;
                }
                continue;
            }
            if !seen.insert((cause.kind.clone(), cause.id.clone())) {
                continue;
            }
            let registered = registry().causes.contains_key(&cause.kind);
            if !registered || cause.id.is_empty() || cause.id.len() > MAX_ID {
                dropped += 1;
                continue;
            }
            kept.push(cause);
        }
        if dropped > 0 {
            // After every guard is released: this emits a tracing event.
            emit_coded!(
                tracing::Level::WARN,
                CAUSES_DROPPED_CODE,
                ErrorClass::Logical,
                module_path!(),
                dropped = dropped as i64
            );
        }
        Causes(Arc::from(kept))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Cause> {
        self.0.iter()
    }

    pub fn to_vec(&self) -> Vec<Cause> {
        self.0.to_vec()
    }

    pub fn contains(&self, kind: &str, id: &str) -> bool {
        self.0.iter().any(|c| c.kind == kind && c.id == id)
    }

    /// Without every listed (kind, id): the record itself and the records that contain it.
    pub fn excluding(&self, drop: &[(&str, &str)]) -> Causes {
        let kept: Vec<Cause> = self
            .0
            .iter()
            .filter(|c| !drop.iter().any(|(k, i)| c.kind == *k && c.id == *i))
            .cloned()
            .collect();
        Causes(Arc::from(kept))
    }

    /// self then extra, through `new` (dedupe, caps, one WARN).
    pub fn with(&self, extra: impl IntoIterator<Item = Cause>) -> Causes {
        Causes::new(self.0.iter().cloned().chain(extra))
    }

    /// `[{"kind":"…","id":"…"},…]`: the span attribute and the JSONB layout.
    pub fn to_json(&self) -> String {
        #[derive(serde::Serialize)]
        struct Entry<'a> {
            kind: &'a str,
            id: &'a str,
        }
        let entries: Vec<Entry<'_>> = self
            .0
            .iter()
            .map(|c| Entry {
                kind: &c.kind,
                id: &c.id,
            })
            .collect();
        serde_json::to_string(&entries).expect("causes serialize")
    }
}

/// The shared registry, parsed once.
fn registry() -> &'static Parsed {
    static REGISTRY: OnceLock<Parsed> = OnceLock::new();
    REGISTRY.get_or_init(concord_fields::shared)
}

/// Cause label (and caller label) to its kind, from the shared `[causes]` table.
fn label_kinds() -> &'static std::collections::BTreeMap<String, &'static str> {
    static LABELS: OnceLock<std::collections::BTreeMap<String, &'static str>> = OnceLock::new();
    LABELS.get_or_init(|| {
        let parsed = registry();
        concord_fields::cause_labels(parsed)
            .into_iter()
            .filter_map(|(label, kind)| {
                parsed
                    .causes
                    .get_key_value(&kind)
                    .map(|(kind, _)| (label, kind.as_str()))
            })
            .collect()
    })
}

/// The cause kind whose label (or caller label) is `label`, if any.
pub fn kind_of_label(label: &str) -> Option<&'static str> {
    label_kinds().get(label).copied()
}

/// The causes named by baggage entries: each entry whose key is a cause label
/// becomes a cause of that kind. Other entries are ignored.
pub fn from_entries(entries: &[(String, String)]) -> Causes {
    Causes::new(
        entries.iter().filter_map(|(label, id)| {
            kind_of_label(label).map(|kind| Cause::new(kind, id.clone()))
        }),
    )
}

/// The snapshot stored by the nearest entry point at or above the current span; empty
/// when there is none or no label layer. Later `context::add` calls never change it.
pub fn capture_incoming() -> Causes {
    baggage::with_extensions(&Span::current(), |span| {
        span.scope().find_map(|s| {
            let extensions = s.extensions();
            extensions.get::<Captured>().map(|c| c.0.clone())
        })
    })
    .flatten()
    .unwrap_or_default()
}

/// For services that parse their own carrier (structmux /ws envelope): `labels::parse`
/// of its baggage, then `from_entries`. No span is touched.
pub fn capture_from(carrier: &TraceCarrier) -> Causes {
    let entries = carrier
        .baggage
        .as_deref()
        .map(labels::parse)
        .unwrap_or_default();
    from_entries(&entries)
}

pin_project! {
    /// A future run with a causes task-local set; see `scope`.
    pub struct Scoped<F> {
        #[pin]
        inner: tokio::task::futures::TaskLocalFuture<Causes, F>,
    }
}

impl<F: Future> Future for Scoped<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<F::Output> {
        self.project().inner.poll(cx)
    }
}

/// Runs `fut` with `causes` as its incoming causes, across awaits. Not inherited by spawned tasks.
pub fn scope<F: Future>(causes: Causes, fut: F) -> Scoped<F> {
    Scoped {
        inner: INCOMING.scope(causes, fut),
    }
}

/// `scope(incoming().with(extra), fut)`. Never travels; task-local only.
pub fn scope_with<F: Future>(extra: impl IntoIterator<Item = Cause>, fut: F) -> Scoped<F> {
    scope(incoming().with(extra), fut)
}

pub fn sync_scope<R>(causes: Causes, f: impl FnOnce() -> R) -> R {
    INCOMING.sync_scope(causes, f)
}

/// Empty outside any scope (and in a spawned task).
pub fn incoming() -> Causes {
    INCOMING.try_with(Clone::clone).unwrap_or_default()
}

/// `incoming().excluding(drop)`.
pub fn excluding(drop: &[(&str, &str)]) -> Causes {
    incoming().excluding(drop)
}

/// Sets the `causes` span attribute (otel only; a no-op otherwise or when empty).
pub fn record(span: &Span, causes: &Causes) {
    #[cfg(feature = "otel")]
    {
        use tracing_opentelemetry::OpenTelemetrySpanExt;
        if !causes.is_empty() {
            span.set_attribute("causes", causes.to_json());
        }
    }
    #[cfg(not(feature = "otel"))]
    let _ = (span, causes);
}

/// The snapshot an entry point stores in its span.
pub(crate) struct Captured(Causes);

/// Stores the snapshot of `kept` (the entries an entry point adopted, empty when
/// the carrier had none) on `span`, replacing any earlier one, and records it.
pub(crate) fn mark_entry(span: &Span, kept: &[(String, String)]) {
    let causes = from_entries(kept);
    // With no label layer `with_extensions` is None, so `record` is skipped on purpose.
    let stored = baggage::with_extensions(span, |s| {
        s.extensions_mut().replace(Captured(causes.clone()));
    })
    .is_some();
    if stored {
        record(span, &causes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cause(kind: &str, id: &str) -> Cause {
        Cause::new(kind, id)
    }

    fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
        items
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn entries_map_labels_and_caller_labels_to_kinds() {
        let entries = pairs(&[
            ("action_id", "A"),
            ("source_instance_id", "I0"),
            ("instance_id", "I1"),
            ("session_id", "S"),
            ("agent_id", "G"),
        ]);
        let causes = from_entries(&entries);
        assert_eq!(
            causes.to_vec(),
            vec![
                cause("action", "A"),
                cause("instance", "I0"),
                cause("instance", "I1"),
            ]
        );
    }

    #[test]
    fn repeats_are_dropped() {
        let causes = Causes::new([
            cause("action", "A"),
            cause("action", "A"),
            cause("turn", "T"),
        ]);
        assert_eq!(causes.len(), 2);
    }

    #[test]
    fn unregistered_kinds_empty_and_long_ids_are_dropped() {
        let long = "x".repeat(MAX_ID + 1);
        let fits = "x".repeat(MAX_ID);
        let causes = Causes::new([
            cause("room", "R"),
            cause("action", ""),
            cause("action", &long),
            cause("action", &fits),
        ]);
        assert_eq!(causes.len(), 1);
        assert_eq!(causes.iter().next().map(|c| c.id.len()), Some(MAX_ID));
    }

    #[test]
    fn at_most_eight_causes() {
        let ids: Vec<String> = (0..10).map(|i| format!("A{i}")).collect();
        let causes = Causes::new(ids.iter().map(|id| cause("action", id)));
        assert_eq!(causes.len(), MAX_CAUSES);
        assert_eq!(
            causes.to_vec(),
            ids[..MAX_CAUSES]
                .iter()
                .map(|id| cause("action", id))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn excluding_removes_only_the_listed_pairs() {
        let causes = Causes::new([
            cause("action", "A"),
            cause("action", "P"),
            cause("turn", "T"),
        ]);
        let kept = causes.excluding(&[("action", "A"), ("action", "P")]);
        assert_eq!(kept.to_vec(), vec![cause("turn", "T")]);
    }

    #[test]
    fn with_keeps_incoming_first() {
        let causes = Causes::new([cause("turn", "T")]);
        let extended = causes.with([cause("message", "M"), cause("turn", "T")]);
        assert_eq!(
            extended.to_vec(),
            vec![cause("turn", "T"), cause("message", "M")]
        );
    }

    #[test]
    fn json_layout() {
        let causes = Causes::new([cause("action", "A")]);
        assert_eq!(causes.to_json(), r#"[{"kind":"action","id":"A"}]"#);
        assert_eq!(Causes::default().to_json(), "[]");
    }

    #[test]
    fn capture_from_reads_the_carrier_baggage() {
        let carrier = TraceCarrier {
            baggage: Some("action_id=A,turn_id=T,bad".to_owned()),
            ..TraceCarrier::default()
        };
        assert_eq!(
            capture_from(&carrier).to_vec(),
            vec![cause("action", "A"), cause("turn", "T")]
        );
    }

    #[test]
    fn incoming_is_empty_outside_a_scope() {
        assert!(incoming().is_empty());
    }
}
