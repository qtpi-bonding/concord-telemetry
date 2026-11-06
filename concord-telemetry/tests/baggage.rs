use concord_telemetry::carrier;
use concord_telemetry::testing::{capture_exclusive, spans};
use concord_telemetry::{AttrValue, TraceCarrier, BAGGAGE_DROPPED_CODE};
use tracing::info_span;

fn entry(i: usize) -> String {
    format!("k{i}=v{i}")
}

/// Parents a fresh span on `header`, returns what its baggage became and the
/// dropped counts recorded.
fn ingest(header: &str) -> (Vec<String>, Vec<i64>) {
    let captured = capture_exclusive();
    let span = info_span!("a2-baggage");
    let c = TraceCarrier {
        baggage: Some(header.to_owned()),
        ..TraceCarrier::default()
    };
    carrier::parent_on(&span, &c);
    let kept = span
        .in_scope(carrier::capture)
        .baggage
        .map(|b| b.split(',').map(str::to_owned).collect())
        .unwrap_or_default();
    let dropped = captured
        .errors_where(|e| e.code == BAGGAGE_DROPPED_CODE)
        .into_iter()
        .map(|e| match e.fields.get("dropped") {
            Some(AttrValue::I64(n)) => *n,
            other => panic!("dropped count missing: {other:?}"),
        })
        .collect();
    (kept, dropped)
}

#[test]
fn oversize_and_malformed_baggage_is_dropped_once() {
    // 9 entries: the ninth is over the cap.
    let nine: Vec<String> = (0..9).map(entry).collect();
    let (kept, dropped) = ingest(&nine.join(","));
    assert_eq!(kept.len(), 8, "{kept:?}");
    assert_eq!(dropped, vec![1]);

    // A 129-byte value.
    let (kept, dropped) = ingest(&format!("big={},ok=1", "v".repeat(129)));
    assert_eq!(kept, vec!["ok=1"]);
    assert_eq!(dropped, vec![1]);

    // A 65-byte key.
    let (kept, dropped) = ingest(&format!("{}=v,ok=1", "k".repeat(65)));
    assert_eq!(kept, vec!["ok=1"]);
    assert_eq!(dropped, vec![1]);

    // Malformed entry.
    let (kept, dropped) = ingest("a=b=c;;,ok=1");
    assert_eq!(kept, vec!["ok=1"]);
    assert_eq!(dropped, vec![1]);

    // Header over 1024 bytes: all of it goes, once.
    let long: Vec<String> = (0..20)
        .map(|i| format!("k{i}={}", "v".repeat(60)))
        .collect();
    let header = long.join(",");
    assert!(header.len() > 1024);
    let (kept, dropped) = ingest(&header);
    assert!(kept.is_empty(), "{kept:?}");
    assert_eq!(dropped, vec![20]);

    // Boundary values are kept and nothing is recorded.
    let (kept, dropped) = ingest(&format!("{}={}", "k".repeat(64), "v".repeat(128)));
    assert_eq!(kept.len(), 1);
    assert!(dropped.is_empty());
}

#[test]
fn caps_hold_on_the_way_out_too() {
    let captured = capture_exclusive();
    let span = info_span!("a2-add-caps");
    let sent = span.in_scope(|| {
        for i in 0..10 {
            concord_telemetry::context::add(Box::leak(format!("k{i}").into_boxed_str()), "v");
        }
        concord_telemetry::context::add("big", "v".repeat(129));
        carrier::capture()
    });
    assert_eq!(sent.baggage.unwrap().split(',').count(), 8);
    assert_eq!(
        captured
            .errors_where(|e| e.code == BAGGAGE_DROPPED_CODE)
            .len(),
        3
    );
    drop(span);
    assert!(spans().iter().any(|s| s.name == "a2-add-caps"));
}
