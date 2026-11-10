use concord_telemetry::carrier;
use concord_telemetry::context;
use concord_telemetry::context::MAX_ENTRIES;
use concord_telemetry::testing::{capture_exclusive, spans};
use concord_telemetry::{AttrValue, TraceCarrier, BAGGAGE_DROPPED_CODE};
use tracing::info_span;

fn entry(i: usize) -> String {
    format!("k{i}=v{i}")
}

/// A 135-character agent id: `p256:` and an uncompressed P-256 key in hex.
fn agent_id() -> String {
    format!("p256:{}", "04".repeat(65))
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
    // One entry over the cap.
    let seventeen: Vec<String> = (0..=MAX_ENTRIES).map(entry).collect();
    let (kept, dropped) = ingest(&seventeen.join(","));
    assert_eq!(kept.len(), MAX_ENTRIES, "{kept:?}");
    assert_eq!(dropped, vec![1]);

    // A 257-byte value.
    let (kept, dropped) = ingest(&format!("big={},ok=1", "v".repeat(257)));
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

    // Header over 8192 bytes: all of it goes, once.
    let long: Vec<String> = (0..40)
        .map(|i| format!("k{i}={}", "v".repeat(250)))
        .collect();
    let header = long.join(",");
    assert!(header.len() > 8192);
    let (kept, dropped) = ingest(&header);
    assert!(kept.is_empty(), "{kept:?}");
    assert_eq!(dropped, vec![40]);

    // Boundary values are kept and nothing is recorded.
    let (kept, dropped) = ingest(&format!("{}={}", "k".repeat(64), "v".repeat(256)));
    assert_eq!(kept.len(), 1);
    assert!(dropped.is_empty());
}

#[test]
fn caps_hold_on_the_way_out_too() {
    let captured = capture_exclusive();
    let span = info_span!("a2-add-caps");
    let sent = span.in_scope(|| {
        for i in 0..MAX_ENTRIES + 2 {
            context::add(Box::leak(format!("k{i}").into_boxed_str()), "v");
        }
        context::add("big", "v".repeat(257));
        carrier::capture()
    });
    assert_eq!(sent.baggage.unwrap().split(',').count(), MAX_ENTRIES);
    assert_eq!(
        captured
            .errors_where(|e| e.code == BAGGAGE_DROPPED_CODE)
            .len(),
        3
    );
    drop(span);
    assert!(spans().iter().any(|s| s.name == "a2-add-caps"));
}

#[test]
fn a_p256_agent_id_survives_ingest_and_inject() {
    let captured = capture_exclusive();
    let id = agent_id();
    assert_eq!(id.len(), 135);

    let c = TraceCarrier {
        baggage: Some(format!("agent_id={id}")),
        ..TraceCarrier::default()
    };
    let span = info_span!("a2-agent-wire");
    carrier::parent_on(&span, &c);
    let wire = span
        .in_scope(carrier::capture)
        .baggage
        .expect("agent_id is carried");
    assert!(captured
        .errors_where(|e| e.code == BAGGAGE_DROPPED_CODE)
        .is_empty());
    // `ingest` takes the exclusive capture itself, so release ours first.
    drop(captured);

    // The ':' may be percent-encoded on the wire; decoding it gives the id back.
    let decoded = wire.replace("%3A", ":");
    assert_eq!(decoded, format!("agent_id={id}"));

    // And the wire form parses back to the same value.
    let (kept, dropped) = ingest(&wire);
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert_eq!(kept[0].replace("%3A", ":"), format!("agent_id={id}"));
    assert!(dropped.is_empty());
}

#[test]
fn overwriting_a_key_at_the_entry_cap_is_allowed() {
    let captured = capture_exclusive();
    let span = info_span!("a2-overwrite");
    let sent = span.in_scope(|| {
        for i in 0..MAX_ENTRIES {
            context::add(Box::leak(format!("k{i}").into_boxed_str()), "v");
        }
        context::add("k3", "new");
        carrier::capture()
    });
    let entries: Vec<String> = sent
        .baggage
        .expect("baggage")
        .split(',')
        .map(str::to_owned)
        .collect();
    assert_eq!(entries.len(), MAX_ENTRIES, "{entries:?}");
    assert!(entries.contains(&"k3=new".to_owned()), "{entries:?}");
    assert!(captured
        .errors_where(|e| e.code == BAGGAGE_DROPPED_CODE)
        .is_empty());
    // `ingest` takes the exclusive capture itself, so release ours first.
    drop(captured);

    // The same holds on ingest: a repeated key replaces, nothing is dropped.
    let header = format!(
        "{},k3=new",
        (0..MAX_ENTRIES).map(entry).collect::<Vec<_>>().join(",")
    );
    let (kept, dropped) = ingest(&header);
    assert_eq!(kept.len(), MAX_ENTRIES, "{kept:?}");
    assert!(kept.contains(&"k3=new".to_owned()), "{kept:?}");
    assert!(dropped.is_empty());
}

#[test]
fn format_records_entries_it_cannot_fit() {
    let captured = capture_exclusive();
    let span = info_span!("a2-format-fit");
    // "é" is 2 bytes, so 128 of them is a 256-byte value; percent-encoded it
    // is 768 bytes on the wire, so about ten entries fill the 8192-byte header.
    let value = "é".repeat(128);
    let sent = span.in_scope(|| {
        for i in 0..MAX_ENTRIES {
            context::add(Box::leak(format!("k{i}").into_boxed_str()), value.clone());
        }
        carrier::capture()
    });
    let header = sent.baggage.expect("baggage");
    assert!(header.len() <= 8192, "{}", header.len());
    let written = header.split(',').count();
    assert!((1..MAX_ENTRIES).contains(&written), "{written}");
    let dropped: Vec<i64> = captured
        .errors_where(|e| e.code == BAGGAGE_DROPPED_CODE)
        .into_iter()
        .map(|e| match e.fields.get("dropped") {
            Some(AttrValue::I64(n)) => *n,
            other => panic!("dropped count missing: {other:?}"),
        })
        .collect();
    assert_eq!(dropped, vec![(MAX_ENTRIES - written) as i64]);
}

#[test]
fn an_agent_id_label_reaches_a_child_span() {
    let _captured = capture_exclusive();
    let id = agent_id();
    let parent = info_span!("a2-agent-parent");
    parent.in_scope(|| {
        context::add("agent_id", id.clone());
        info_span!("a2-agent-child").in_scope(|| {});
    });
    drop(parent);

    let child = spans()
        .into_iter()
        .find(|s| s.name == "a2-agent-child")
        .expect("child span finished");
    let value = child
        .attributes
        .iter()
        .find(|kv| kv.key.as_str() == "agent_id")
        .map(|kv| kv.value.to_string());
    assert_eq!(value.as_deref(), Some(id.as_str()));
}
