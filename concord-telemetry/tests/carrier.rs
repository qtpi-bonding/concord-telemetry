use concord_telemetry::carrier;
use concord_telemetry::context;
use concord_telemetry::testing::{capture_exclusive, set_otel_active, spans};
use concord_telemetry::{AttrValue, TraceCarrier, CARRIER_DROPPED_CODE, TRACE_MISSING_CODE};
use tracing::info_span;

fn trace_id_of(carrier: &TraceCarrier) -> String {
    carrier.traceparent.as_ref().expect("traceparent")[3..35].to_owned()
}

fn finished(name: &str) -> opentelemetry_sdk::trace::SpanData {
    spans()
        .into_iter()
        .find(|span| span.name == name)
        .unwrap_or_else(|| panic!("no finished span named {name}"))
}

fn attr(span: &opentelemetry_sdk::trace::SpanData, key: &str) -> Option<String> {
    span.attributes
        .iter()
        .find(|kv| kv.key.as_str() == key)
        .map(|kv| kv.value.to_string())
}

#[test]
fn carrier_capture_then_parent_on_keeps_trace_id_and_baggage() {
    let _captured = capture_exclusive();
    set_otel_active(false);
    let origin = info_span!("a2-origin");
    let sent = origin.in_scope(|| {
        context::add("action_id", "A1");
        carrier::capture()
    });
    assert_eq!(sent.baggage.as_deref(), Some("action_id=A1"));

    let second = tracing::Span::none().in_scope(|| info_span!("a2-second"));
    carrier::parent_on(&second, &sent);
    let again = second.in_scope(carrier::capture);
    assert_eq!(trace_id_of(&again), trace_id_of(&sent));
    assert_eq!(again.baggage.as_deref(), Some("action_id=A1"));
    second.in_scope(|| drop(info_span!("a2-child").entered()));
    drop(second);

    let child = finished("a2-child");
    assert_eq!(attr(&child, "action_id").as_deref(), Some("A1"));
    let second = finished("a2-second");
    assert_eq!(attr(&second, "action_id").as_deref(), Some("A1"));
    assert_eq!(
        second
            .attributes
            .iter()
            .filter(|kv| kv.key.as_str() == "action_id")
            .count(),
        1,
        "the attribute is written once per span"
    );
}

#[test]
fn link_from_starts_a_new_trace_with_a_link() {
    let _captured = capture_exclusive();
    set_otel_active(false);
    let origin = info_span!("a2-link-origin");
    let sent = origin.in_scope(|| {
        context::add("action_id", "A2");
        carrier::capture()
    });
    let linked = tracing::Span::none().in_scope(|| info_span!("a2-linked"));
    carrier::link_from(&linked, &sent);
    let own = linked.in_scope(carrier::capture);
    assert_ne!(trace_id_of(&own), trace_id_of(&sent));
    assert_eq!(own.baggage, None, "a link carries span context only");
    drop(linked);

    let span = finished("a2-linked");
    assert_eq!(span.links.links.len(), 1);
    let link = &span.links.links[0];
    assert_eq!(link.span_context.trace_id().to_string(), trace_id_of(&sent));
    assert_eq!(
        link.span_context.span_id().to_string(),
        sent.traceparent.as_ref().unwrap()[36..52]
    );
}

#[test]
fn extract_required_on_empty_carrier_emits_trace_missing() {
    let captured = capture_exclusive();
    set_otel_active(true);
    let span = info_span!("a2-missing");
    carrier::extract_required("test.hop", &span, &TraceCarrier::default());
    set_otel_active(false);
    let found = captured.errors_where(|e| e.code == TRACE_MISSING_CODE);
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0].fields.get("hop"),
        Some(&AttrValue::Str("test.hop".into()))
    );
}

#[test]
fn extract_required_with_a_valid_carrier_is_silent() {
    let captured = capture_exclusive();
    set_otel_active(true);
    let origin = info_span!("a2-valid-origin");
    let sent = origin.in_scope(carrier::capture);
    let span = info_span!("a2-valid");
    carrier::extract_required("test.hop", &span, &sent);
    set_otel_active(false);
    assert!(captured.errors_where(|_| true).is_empty());
}

#[test]
fn extract_ingress_on_empty_carrier_is_silent() {
    let captured = capture_exclusive();
    set_otel_active(true);
    let span = info_span!("a2-ingress");
    carrier::extract_ingress("test.ingress", &span, &TraceCarrier::default());
    span.in_scope(|| ());
    drop(span);
    set_otel_active(false);
    assert!(captured.errors_where(|_| true).is_empty());
    assert_eq!(
        attr(&finished("a2-ingress"), "ingress").as_deref(),
        Some("test.ingress")
    );
}

#[test]
fn root_has_no_parent_and_takes_the_given_name() {
    let _captured = capture_exclusive();
    set_otel_active(false);
    let outer = info_span!("a2-outer");
    let outer_id = outer.in_scope(|| trace_id_of(&carrier::capture()));
    let own = outer.in_scope(|| {
        let root = carrier::root("a2.root");
        root.in_scope(carrier::capture)
    });
    assert_ne!(trace_id_of(&own), outer_id);
}

fn dropped_headers(captured: &concord_telemetry::testing::Capture) -> Vec<AttrValue> {
    captured
        .errors_where(|e| e.code == CARRIER_DROPPED_CODE)
        .into_iter()
        .map(|e| e.fields.get("header").cloned().expect("header field"))
        .collect()
}

#[test]
fn malformed_traceparent_is_dropped_once_and_request_proceeds() {
    let captured = capture_exclusive();
    set_otel_active(true);
    let bad = TraceCarrier {
        traceparent: Some("not-a-traceparent".into()),
        ..TraceCarrier::default()
    };
    let span = info_span!("a2-bad-ingress");
    carrier::extract_ingress("test.ingress", &span, &bad);
    let own = span.in_scope(carrier::capture);
    assert!(own.traceparent.is_some(), "the request proceeds unparented");
    assert_eq!(
        dropped_headers(&captured),
        vec![AttrValue::Str("traceparent".into())]
    );

    // On a required hop it is this record, not trace.missing.
    drop(captured);
    let captured2 = capture_exclusive();
    let span = info_span!("a2-bad-required");
    carrier::extract_required("test.hop", &span, &bad);
    set_otel_active(false);
    assert_eq!(
        dropped_headers(&captured2),
        vec![AttrValue::Str("traceparent".into())]
    );
    assert!(captured2
        .errors_where(|e| e.code == TRACE_MISSING_CODE)
        .is_empty());
}

#[test]
fn oversize_tracestate_is_dropped_once() {
    let captured = capture_exclusive();
    set_otel_active(true);
    let origin = info_span!("a2-ts-origin");
    let mut sent = origin.in_scope(carrier::capture);
    // One entry over 128 characters and over the 512 cap.
    sent.tracestate = Some(format!("v={}", "x".repeat(600)));
    let span = info_span!("a2-ts");
    carrier::extract_required("test.hop", &span, &sent);
    set_otel_active(false);
    assert_eq!(
        dropped_headers(&captured),
        vec![AttrValue::Str("tracestate".into())]
    );
    let joined = span.in_scope(carrier::capture);
    assert_eq!(trace_id_of(&joined), trace_id_of(&sent));
    assert!(joined.tracestate.is_none());
    assert!(captured
        .errors_where(|e| e.code == TRACE_MISSING_CODE)
        .is_empty());
}

#[test]
fn tracestate_over_512_drops_long_entries_first_then_whole_entries_from_the_end() {
    let captured = capture_exclusive();
    set_otel_active(true);
    let origin = info_span!("a2-ts-trim-origin");
    let mut sent = origin.in_scope(carrier::capture);
    let normal: Vec<String> = (0..5)
        .map(|i| format!("k{i}={}", "v".repeat(115)))
        .collect();
    let long = format!("big={}", "y".repeat(140));
    // 5 x 119 + 144 + 5 commas: over 512 and still over without the long one.
    sent.tracestate = Some(format!(
        "{},{},{}",
        normal[..2].join(","),
        long,
        normal[2..].join(",")
    ));
    let span = info_span!("a2-ts-trim");
    carrier::extract_required("test.hop", &span, &sent);
    set_otel_active(false);
    let joined = span.in_scope(carrier::capture);
    assert_eq!(trace_id_of(&joined), trace_id_of(&sent));
    assert_eq!(
        joined.tracestate.as_deref(),
        Some(normal[..4].join(",").as_str())
    );
    assert_eq!(
        dropped_headers(&captured),
        vec![AttrValue::Str("tracestate".into())]
    );

    // Under the cap nothing is touched, long entries included.
    let mut fits = origin.in_scope(carrier::capture);
    fits.tracestate = Some(format!("a=1,{long},b=2"));
    let span = info_span!("a2-ts-fits");
    carrier::parent_on(&span, &fits);
    assert_eq!(span.in_scope(carrier::capture).tracestate, fits.tracestate);
}

#[test]
fn parent_on_replaces_the_spans_labels_with_the_carriers() {
    let _captured = capture_exclusive();
    let parent = info_span!("a2-replace-parent");
    parent.in_scope(|| context::add("action_id", "inherited"));
    let child = parent.in_scope(|| info_span!("a2-replace-child"));
    carrier::parent_on(
        &child,
        &TraceCarrier {
            baggage: Some("action_id=carried".into()),
            ..TraceCarrier::default()
        },
    );
    assert_eq!(
        child.in_scope(carrier::capture).baggage.as_deref(),
        Some("action_id=carried")
    );
}
