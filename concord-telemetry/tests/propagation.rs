use concord_telemetry::propagation::{inject, set_parent, traceparent};
use concord_telemetry::testing::capture_exclusive;
use concord_telemetry::EVENT_TARGET;
use std::collections::HashMap;
use tracing::{event, info_span, Level};

const TRACE_ID: &str = "0af7651916cd43dd8448eb211c80319c";
const PARENT: &str = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";

fn hex(bytes: [u8; 16]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn carrier(traceparent: &str) -> impl Fn(&str) -> Option<String> + '_ {
    move |key| (key == "traceparent").then(|| traceparent.to_owned())
}

#[test]
fn a_span_joins_the_trace_in_its_carrier() {
    let captured = capture_exclusive();
    let span = info_span!("joined");
    set_parent(&span, carrier(PARENT));
    span.in_scope(|| {
        event!(name: "propagation-joined", target: EVENT_TARGET, Level::INFO, marker = "joined");
        let own = traceparent().expect("the joined span has a traceparent");
        assert!(own.starts_with(&format!("00-{TRACE_ID}-")), "{own}");
        assert!(!own.contains("b7ad6b7169203331"), "{own}");
    });
    let actual = captured
        .trace_id_for("propagation-joined")
        .expect("event has a trace context");
    assert_eq!(hex(actual), TRACE_ID);
}

#[test]
fn inject_then_set_parent_carries_the_trace_id() {
    let captured = capture_exclusive();
    let mut headers = HashMap::new();
    let first = info_span!("first");
    let expected = first.in_scope(|| {
        inject(|key, value| {
            headers.insert(key.to_owned(), value);
        });
        traceparent().expect("first span has a traceparent")
    });
    assert!(
        !headers.contains_key("tracestate"),
        "empty tracestate is skipped"
    );

    let second = tracing::Span::none().in_scope(|| info_span!("second"));
    set_parent(&second, |key| headers.get(key).cloned());
    second.in_scope(|| {
        event!(name: "propagation-injected", target: EVENT_TARGET, Level::INFO, marker = "injected");
    });
    let actual = captured
        .trace_id_for("propagation-injected")
        .expect("event has a trace context");
    assert_eq!(hex(actual), expected[3..35]);
}

#[test]
fn an_invalid_carrier_keeps_the_contextual_parent() {
    let captured = capture_exclusive();
    let outer = info_span!("outer");
    let expected = outer.in_scope(|| traceparent().expect("outer has a traceparent"));
    outer.in_scope(|| {
        let inner = info_span!("inner");
        set_parent(&inner, carrier("not-a-traceparent"));
        inner.in_scope(|| {
            event!(name: "propagation-invalid", target: EVENT_TARGET, Level::INFO, marker = "invalid");
        });
    });
    let actual = captured
        .trace_id_for("propagation-invalid")
        .expect("event has a trace context");
    assert_eq!(hex(actual), expected[3..35]);
}

#[test]
fn without_a_span_nothing_is_injected() {
    let _captured = capture_exclusive();
    tracing::Span::none().in_scope(|| {
        assert_eq!(traceparent(), None);
        let mut calls = 0;
        inject(|_, _| calls += 1);
        assert_eq!(calls, 0);
    });
}
