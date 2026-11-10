//! `context::add_from_baggage` adds a header's labels to the current span, like
//! `context::add` for each entry in header order.

use concord_telemetry::testing::{
    assert_no_label_conflict, capture_exclusive, span_attr, spans, stamp_count,
};
use concord_telemetry::{carrier, context, AttrValue, BAGGAGE_DROPPED_CODE};
use opentelemetry_sdk::trace::SpanData;
use tracing::info_span;

fn one(name: &str) -> SpanData {
    let mut found: Vec<_> = spans().into_iter().filter(|s| s.name == name).collect();
    assert_eq!(found.len(), 1, "spans named {name}: {}", found.len());
    found.remove(0)
}

/// How many exported attributes carry `key`, bare or stamped.
fn count_of(span: &SpanData, key: &str) -> usize {
    let stamped = format!("concord.label.{key}");
    span.attributes
        .iter()
        .filter(|kv| kv.key.as_str() == key || kv.key.as_str() == stamped)
        .count()
}

fn dropped_counts(captured: &concord_telemetry::testing::ExclusiveCapture) -> Vec<i64> {
    captured
        .errors_where(|e| e.code == BAGGAGE_DROPPED_CODE)
        .into_iter()
        .map(|e| match e.fields.get("dropped") {
            Some(AttrValue::I64(n)) => *n,
            other => panic!("dropped count missing: {other:?}"),
        })
        .collect()
}

#[test]
fn add_from_baggage_adds_every_entry_to_the_current_span() {
    let _c = capture_exclusive();
    let span = info_span!("afb-every");
    let sent = span.in_scope(|| {
        context::add_from_baggage("agent_id=A1, turn_id=T1 ,action_id=X");
        carrier::capture()
    });
    assert_eq!(
        sent.baggage.as_deref(),
        Some("agent_id=A1,turn_id=T1,action_id=X")
    );
    drop(span);
    let exported = one("afb-every");
    for (key, value) in [("agent_id", "A1"), ("turn_id", "T1"), ("action_id", "X")] {
        assert_eq!(span_attr(&exported, key).as_deref(), Some(value));
        assert_eq!(count_of(&exported, key), 1, "{key} exported once");
    }
}

#[test]
fn add_from_baggage_overwrites_an_inherited_label() {
    let _c = capture_exclusive();
    let parent = info_span!(parent: None, "afb-parent");
    parent.in_scope(|| context::add("turn_id", "a"));
    let child = parent.in_scope(|| info_span!("afb-child"));
    let grandchild = child.in_scope(|| {
        child.in_scope(|| context::add_from_baggage("turn_id=b"));
        info_span!("afb-grandchild")
    });
    grandchild.in_scope(|| {});
    drop(grandchild);
    drop(child);
    drop(parent);
    let child = one("afb-child");
    assert_eq!(span_attr(&child, "turn_id").as_deref(), Some("b"));
    assert_eq!(count_of(&child, "turn_id"), 1);
    assert_eq!(
        span_attr(&child, "concord.label_conflicts"),
        None,
        "overwriting is not a conflict"
    );
    let grandchild = one("afb-grandchild");
    assert_eq!(span_attr(&grandchild, "turn_id").as_deref(), Some("b"));
    assert_no_label_conflict(&[child, grandchild]);
}

#[test]
fn add_from_baggage_drops_the_reserved_namespace_and_malformed_entries() {
    let c = capture_exclusive();
    let span = info_span!("afb-drops");
    let sent = span.in_scope(|| {
        context::add_from_baggage("concord.label.x=evil,ok=1,no_equals,a=b=c,concord.y=z");
        carrier::capture()
    });
    assert_eq!(sent.baggage.as_deref(), Some("ok=1"));
    assert_eq!(dropped_counts(&c), vec![4]);
    drop(span);
    let exported = one("afb-drops");
    assert_eq!(span_attr(&exported, "ok").as_deref(), Some("1"));
    assert_eq!(span_attr(&exported, "concord.label.x"), None);
}

#[test]
fn add_from_baggage_respects_the_entry_cap() {
    let c = capture_exclusive();
    let header: Vec<String> = (0..context::MAX_ENTRIES + 3)
        .map(|i| format!("k{i}=v"))
        .collect();
    let span = info_span!("afb-cap");
    let sent = span.in_scope(|| {
        context::add_from_baggage(&header.join(","));
        carrier::capture()
    });
    assert_eq!(
        sent.baggage.unwrap().split(',').count(),
        context::MAX_ENTRIES
    );
    assert_eq!(dropped_counts(&c), vec![3], "counted once");
}

#[test]
fn add_from_baggage_counts_entries_the_span_refuses_with_the_parse_drops() {
    let c = capture_exclusive();
    let span = info_span!("afb-refused");
    span.in_scope(|| {
        for i in 0..context::MAX_ENTRIES - 1 {
            context::add(Box::leak(format!("own{i}").into_boxed_str()), "v");
        }
    });
    let before = dropped_counts(&c).len();
    span.in_scope(|| context::add_from_baggage("new1=a,new2=b,concord.x=y"));
    // new2 is refused by the span's cap, the reserved key by parse.
    assert_eq!(dropped_counts(&c)[before..], [2]);
}

#[test]
fn add_from_baggage_on_an_unentered_span_via_in_scope() {
    let _c = capture_exclusive();
    let span = tracing::Span::none().in_scope(|| info_span!("afb-unentered"));
    span.in_scope(|| context::add_from_baggage("agent_id=A9"));
    let sent = span.in_scope(carrier::capture);
    assert_eq!(sent.baggage.as_deref(), Some("agent_id=A9"));
    drop(span);
    let exported = one("afb-unentered");
    assert_eq!(span_attr(&exported, "agent_id").as_deref(), Some("A9"));
}

#[test]
fn an_empty_header_does_nothing() {
    let c = capture_exclusive();
    let span = info_span!("afb-empty");
    let sent = span.in_scope(|| {
        context::add_from_baggage("");
        context::add_from_baggage(" , ");
        carrier::capture()
    });
    assert_eq!(sent.baggage, None);
    assert!(dropped_counts(&c).is_empty());
}

#[test]
fn an_unchanged_value_is_not_stamped_again() {
    let _c = capture_exclusive();
    let span = info_span!("afb-unchanged");
    span.in_scope(|| {
        let before = stamp_count();
        context::add_from_baggage("a=1,b=2");
        assert_eq!(stamp_count() - before, 2);
        let after_first = stamp_count();
        context::add_from_baggage("a=1,b=2");
        assert_eq!(stamp_count(), after_first, "same values stamp nothing");
        context::add_from_baggage("a=1,b=3");
        assert_eq!(stamp_count() - after_first, 1, "only the changed one");
    });
}
