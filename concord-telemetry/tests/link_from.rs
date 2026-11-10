//! `link_from` carries span context only; it never touches the span's labels.

use concord_telemetry::testing::{capture_exclusive, span_attr, spans};
use concord_telemetry::{carrier, context};
use tracing::info_span;

fn one(name: &str) -> opentelemetry_sdk::trace::SpanData {
    let mut found: Vec<_> = spans().into_iter().filter(|s| s.name == name).collect();
    assert_eq!(found.len(), 1);
    found.remove(0)
}

#[test]
fn link_from_does_not_change_the_spans_labels() {
    let _c = capture_exclusive();
    let origin = info_span!("link-origin");
    let sent = origin.in_scope(|| {
        context::add("action_id", "origin");
        carrier::capture()
    });
    let linked = tracing::Span::none().in_scope(|| info_span!("link-linked"));
    let own = linked.in_scope(|| {
        context::add("action_id", "mine");
        carrier::capture()
    });
    carrier::link_from(&linked, &sent);
    let after = linked.in_scope(carrier::capture);
    assert_eq!(after.baggage, own.baggage);
    assert_eq!(after.baggage.as_deref(), Some("action_id=mine"));
    drop(linked);
    let span = one("link-linked");
    assert_eq!(span_attr(&span, "action_id").as_deref(), Some("mine"));
    assert_eq!(span.links.links.len(), 1);

    // A span with no labels of its own gets none from the link.
    let bare = tracing::Span::none().in_scope(|| info_span!("link-bare"));
    carrier::link_from(&bare, &sent);
    assert_eq!(bare.in_scope(carrier::capture).baggage, None);
}
