//! Labels resolve once, when the span ends (design A). Every test holds the
//! exclusive capture, so the spans it reads are its own.

use concord_telemetry::testing::{
    assert_no_label_conflict, capture_exclusive, span_attr, spans, spans_in_trace,
};
use concord_telemetry::{carrier, context, error_anyhow, TraceCarrier};
use opentelemetry_sdk::trace::SpanData;
use std::sync::mpsc;
use std::sync::{Arc, Barrier};
use std::time::Duration;
use tracing::info_span;

#[derive(Debug, thiserror::Error, coded_error::CodedError)]
#[coded(namespace = "test.labels")]
enum Failure {
    #[error("not found")]
    #[coded(code = "not_found", class = Logical)]
    NotFound,
}

fn named(name: &str) -> Vec<SpanData> {
    spans().into_iter().filter(|s| s.name == name).collect()
}

fn one(name: &str) -> SpanData {
    let mut found = named(name);
    assert_eq!(found.len(), 1, "spans named {name}: {}", found.len());
    found.remove(0)
}

fn conflicts(span: &SpanData) -> Option<String> {
    span_attr(span, "concord.label_conflicts")
}

#[test]
fn two_error_calls_in_one_span_export_one_error_code() {
    let _c = capture_exclusive();
    info_span!("two-errors").in_scope(|| {
        error_anyhow!(&coded_error::into_anyhow(Failure::NotFound));
        error_anyhow!(&anyhow::anyhow!("boom"));
    });
    let span = one("two-errors");
    assert_eq!(span_attr(&span, "error.code").as_deref(), Some("unknown"));
    assert_eq!(span_attr(&span, "error.class").as_deref(), Some("unknown"));
}

#[test]
fn a_declared_field_and_an_inherited_label_export_once() {
    let _c = capture_exclusive();
    let parent = info_span!("declared-parent");
    parent.in_scope(|| {
        context::add("action_id", "same");
        info_span!("declared-child", action_id = "same").in_scope(|| {});
    });
    drop(parent);
    let child = one("declared-child");
    assert_eq!(span_attr(&child, "action_id").as_deref(), Some("same"));
    assert_eq!(conflicts(&child), None);
}

#[test]
fn a_re_record_is_not_a_conflict() {
    let _c = capture_exclusive();
    let parent = info_span!("rerecord-parent");
    parent.in_scope(|| {
        context::add("action_id", "A");
        let child = info_span!("rerecord-child", attempts = 0, action_id = "A");
        child.in_scope(|| {
            child.record("attempts", 1);
            child.record("action_id", "A");
        });
    });
    drop(parent);
    let child = one("rerecord-child");
    assert_eq!(span_attr(&child, "attempts").as_deref(), Some("1"));
    assert_eq!(span_attr(&child, "action_id").as_deref(), Some("A"));
    assert_eq!(conflicts(&child), None);
}

#[test]
fn a_different_own_value_is_one_key_and_one_reported_conflict() {
    let _c = capture_exclusive();
    let parent = info_span!("clash-parent");
    parent.in_scope(|| {
        context::add("request_id", "caller");
        info_span!("clash-child", request_id = "own").in_scope(|| {});
    });
    drop(parent);
    let child = one("clash-child");
    assert_eq!(span_attr(&child, "request_id").as_deref(), Some("own"));
    let reported = child
        .attributes
        .iter()
        .find(|kv| kv.key.as_str() == "concord.label_conflicts")
        .expect("a conflict is reported");
    assert_eq!(format!("{}", reported.value), r#"["request_id"]"#);
    assert!(!format!("{}", reported.value).contains("caller"));
}

#[test]
fn an_empty_field_never_recorded_exports_the_label() {
    let _c = capture_exclusive();
    let parent = info_span!("empty-parent");
    parent.in_scope(|| {
        context::add("action_id", "A");
        info_span!("empty-child", action_id = tracing::field::Empty).in_scope(|| {});
    });
    drop(parent);
    let child = one("empty-child");
    assert_eq!(span_attr(&child, "action_id").as_deref(), Some("A"));
    assert_eq!(conflicts(&child), None);
}

#[test]
fn context_add_twice_exports_the_last_value_without_conflict() {
    let _c = capture_exclusive();
    info_span!("add-twice").in_scope(|| {
        context::add("action_id", "a");
        context::add("action_id", "b");
    });
    let span = one("add-twice");
    assert_eq!(span_attr(&span, "action_id").as_deref(), Some("b"));
    assert_eq!(conflicts(&span), None);
}

#[test]
fn context_add_after_the_span_recorded_a_different_value_is_a_conflict() {
    let _c = capture_exclusive();
    info_span!("add-late", action_id = "own").in_scope(|| context::add("action_id", "other"));
    let span = one("add-late");
    assert_eq!(span_attr(&span, "action_id").as_deref(), Some("own"));
    assert_eq!(conflicts(&span).as_deref(), Some(r#"["action_id"]"#));
}

#[test]
fn a_late_context_add_reaches_children_created_after_it() {
    let _c = capture_exclusive();
    let parent = info_span!("late-parent");
    parent.in_scope(|| {
        info_span!("late-before").in_scope(|| {});
        context::add("turn_id", "t1");
        info_span!("late-after").in_scope(|| {});
    });
    drop(parent);
    assert_eq!(span_attr(&one("late-after"), "turn_id").as_deref(), Some("t1"));
    assert_eq!(span_attr(&one("late-before"), "turn_id"), None);
    assert_eq!(span_attr(&one("late-parent"), "turn_id").as_deref(), Some("t1"));
}

#[test]
fn no_exported_span_carries_a_concord_label_key() {
    let _c = capture_exclusive();
    let root = info_span!("prefix-root", request_id = "own");
    root.in_scope(|| {
        context::add("action_id", "A");
        context::add("request_id", "other");
        context::add("turn_id", "t");
        info_span!("prefix-child", turn_id = "t").in_scope(|| {
            info_span!("prefix-grandchild").in_scope(|| {});
        });
    });
    let linked = info_span!("prefix-linked");
    carrier::parent_on(
        &linked,
        &TraceCarrier {
            baggage: Some("session_id=s".into()),
            ..TraceCarrier::default()
        },
    );
    linked.in_scope(|| {});
    drop(root);
    drop(linked);
    let exported: Vec<SpanData> = spans()
        .into_iter()
        .filter(|s| s.name.starts_with("prefix-"))
        .collect();
    assert_eq!(exported.len(), 4);
    for span in &exported {
        for kv in &span.attributes {
            assert!(
                !kv.key.as_str().starts_with("concord.label."),
                "{} exports {}",
                span.name,
                kv.key
            );
        }
    }
    let linked = one("prefix-linked");
    assert_eq!(span_attr(&linked, "session_id").as_deref(), Some("s"));
}

#[test]
fn a_debug_impl_that_logs_while_recorded_does_not_deadlock() {
    struct Loud;
    impl std::fmt::Debug for Loud {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            tracing::info!("Debug ran");
            f.write_str("loud")
        }
    }
    let _c = capture_exclusive();
    let (done, finished) = mpsc::channel();
    std::thread::spawn(move || {
        let parent = info_span!("loud-parent");
        parent.in_scope(|| {
            context::add("action_id", "A");
            let child = info_span!("loud-child", action_id = ?Loud);
            child.in_scope(|| {
                child.record("action_id", tracing::field::debug(Loud));
            });
        });
        drop(parent);
        let _ = done.send(());
    });
    finished
        .recv_timeout(Duration::from_secs(20))
        .expect("recording a user Debug that logs must not deadlock");
    let child = one("loud-child");
    assert_eq!(span_attr(&child, "action_id").as_deref(), Some("loud"));
}

#[test]
fn an_incoming_baggage_key_with_the_reserved_prefix_is_dropped() {
    let captured = capture_exclusive();
    let span = info_span!("reserved-in");
    carrier::parent_on(
        &span,
        &TraceCarrier {
            baggage: Some("concord.label.action_id=evil,ok=1".into()),
            ..TraceCarrier::default()
        },
    );
    let sent = span.in_scope(carrier::capture);
    assert_eq!(sent.baggage.as_deref(), Some("ok=1"));
    let dropped = captured.errors_where(|e| e.code == concord_telemetry::BAGGAGE_DROPPED_CODE);
    assert_eq!(dropped.len(), 1);
    span.in_scope(|| {});
    drop(span);
    for kv in &one("reserved-in").attributes {
        assert!(!kv.key.as_str().starts_with("concord.label."));
    }
}

#[test]
fn context_add_rejects_the_reserved_prefix() {
    let captured = capture_exclusive();
    let span = info_span!("reserved-add");
    let sent = span.in_scope(|| {
        context::add("concord.label.action_id", "x");
        carrier::capture()
    });
    assert_eq!(sent.baggage, None);
    assert_eq!(
        captured
            .errors_where(|e| e.code == concord_telemetry::BAGGAGE_DROPPED_CODE)
            .len(),
        1
    );
    drop(span);
    assert_eq!(span_attr(&one("reserved-add"), "concord.label.action_id"), None);
}

#[test]
fn the_entry_cap_is_24() {
    assert_eq!(context::MAX_ENTRIES, 24);
}

#[test]
fn capture_sees_only_this_tests_spans() {
    let _c = capture_exclusive();
    // A non-exclusive user emits spans while this test holds the exclusive capture.
    let barrier = Arc::new(Barrier::new(2));
    let other_barrier = Arc::clone(&barrier);
    let other = std::thread::spawn(move || {
        let _other = concord_telemetry::testing::capture();
        other_barrier.wait();
        for _ in 0..20 {
            info_span!(parent: None, "foreign-span").in_scope(|| {});
        }
    });
    barrier.wait();
    let mine = info_span!(parent: None, "own-root");
    mine.in_scope(|| info_span!("own-child").in_scope(|| {}));
    other.join().expect("other thread");
    let seen = spans_in_trace(&mine);
    let names: Vec<String> = seen.iter().map(|s| s.name.to_string()).collect();
    assert_eq!(names, ["own-child"], "{names:?}");
    drop(mine);
    let own: Vec<_> = spans().into_iter().filter(|s| s.name.starts_with("own-")).collect();
    assert_eq!(own.len(), 2);
}

#[test]
fn assert_no_label_conflict_fails_on_a_conflict() {
    let _c = capture_exclusive();
    let parent = info_span!("assert-parent");
    parent.in_scope(|| {
        context::add("action_id", "a");
        info_span!("assert-child", action_id = "b").in_scope(|| {});
    });
    drop(parent);
    let child = one("assert-child");
    let result = std::panic::catch_unwind(|| assert_no_label_conflict(&[child]));
    assert!(result.is_err());
}
