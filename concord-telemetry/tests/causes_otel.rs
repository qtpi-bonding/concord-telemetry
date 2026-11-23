// The OpenTelemetry side of causes: the one WARN, the `causes` span attribute,
// and that causes never become labels.
#![cfg(feature = "testing")]

use concord_telemetry::carrier::{self, context};
use concord_telemetry::causes::{capture_incoming, Causes, CAUSES_DROPPED_CODE};
use concord_telemetry::labels;
use concord_telemetry::testing::{capture_exclusive, check_attributes, span_attr, spans, Registry};
use concord_telemetry::{AttrValue, Cause, TraceCarrier};
use tracing::info_span;

const TP: &str = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";

fn cause(kind: &str, id: &str) -> Cause {
    Cause::new(kind, id)
}

#[test]
fn one_warn_per_capping() {
    let captured = capture_exclusive();
    let long = "x".repeat(257);
    let mut all: Vec<Cause> = (0..10).map(|i| cause("action", &format!("A{i}"))).collect();
    all.push(cause("action", &long));
    let _ = Causes::new(all);
    let records = captured.errors_where(|e| e.code == CAUSES_DROPPED_CODE);
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(records[0].fields.get("dropped"), Some(&AttrValue::I64(3)));
}

#[test]
fn no_warn_when_nothing_dropped() {
    let captured = capture_exclusive();
    let _ = Causes::new([cause("action", "A"), cause("turn", "T")]);
    let records = captured.errors_where(|e| e.code == CAUSES_DROPPED_CODE);
    assert!(records.is_empty(), "{records:?}");
}

#[test]
fn the_entry_span_carries_the_causes_attribute() {
    let _captured = capture_exclusive();
    let span = info_span!("causes-attr-entry");
    carrier::parent_on(
        &span,
        &TraceCarrier {
            traceparent: Some(TP.to_owned()),
            baggage: Some("action_id=A1".to_owned()),
            ..TraceCarrier::default()
        },
    );
    drop(span);
    let exported = spans()
        .into_iter()
        .find(|s| s.name == "causes-attr-entry")
        .expect("exported entry span");
    assert_eq!(
        span_attr(&exported, "causes").as_deref(),
        Some(r#"[{"kind":"action","id":"A1"}]"#)
    );
    check_attributes(
        &Registry::shared(),
        "causes-attr-entry",
        &[(
            "causes".to_owned(),
            r#"[{"kind":"action","id":"A1"}]"#.to_owned(),
        )],
    );
}

#[test]
fn causes_is_never_a_label() {
    let _captured = capture_exclusive();
    let span = info_span!("causes-not-label");
    let _entered = span.enter();
    carrier::parent_on(
        &span,
        &TraceCarrier {
            baggage: Some("action_id=A1".to_owned()),
            ..TraceCarrier::default()
        },
    );
    context::add("action_id", "A2");
    let sent = carrier::capture();
    assert!(!sent.baggage.unwrap_or_default().contains("causes="));
    for exported in spans() {
        assert_eq!(span_attr(&exported, "concord.label.causes"), None);
    }
}

#[test]
fn an_entry_that_already_started_keeps_its_snapshot() {
    let _captured = capture_exclusive();
    let span = info_span!("causes-started-entry");
    carrier::parent_on(
        &span,
        &TraceCarrier {
            traceparent: Some(TP.to_owned()),
            baggage: Some("action_id=A1".to_owned()),
            ..TraceCarrier::default()
        },
    );
    let _entered = span.enter();
    // The span has started: its trace and baggage are refused, so the snapshot stays.
    carrier::parent_on(
        &span,
        &TraceCarrier {
            traceparent: Some("00-1af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01".to_owned()),
            baggage: Some("action_id=A2".to_owned()),
            ..TraceCarrier::default()
        },
    );
    assert_eq!(capture_incoming().to_vec(), vec![cause("action", "A1")]);
    assert_eq!(labels::of(&span, "action_id").as_deref(), Some("A1"));
}
