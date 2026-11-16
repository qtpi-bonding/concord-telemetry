//! The public label and carrier helpers: `labels::*`, `carrier::with_label`,
//! `carrier::without_label`, `carrier::parent_on_or_env`.

use concord_telemetry::{carrier, labels, TraceCarrier};

fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
    items
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

fn with_baggage(header: &str) -> TraceCarrier {
    TraceCarrier {
        traceparent: Some("00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01".to_owned()),
        baggage: Some(header.to_owned()),
        ..TraceCarrier::default()
    }
}

#[test]
fn parse_drops_properties_and_decodes() {
    assert_eq!(
        labels::parse("a=1;p=x,b=%20y"),
        pairs(&[("a", "1"), ("b", " y")])
    );
}

#[test]
fn parse_respects_caps() {
    let header: Vec<String> = (0..25).map(|i| format!("k{i}=v")).collect();
    assert_eq!(labels::parse(&header.join(",")).len(), 24);
    let long = format!("big={},ok=1", "x".repeat(257));
    assert_eq!(labels::parse(&long), pairs(&[("ok", "1")]));
    let at_cap = format!("big={}", "x".repeat(256));
    assert_eq!(labels::parse(&at_cap).len(), 1);
}

#[test]
fn format_round_trips() {
    let entries = pairs(&[("a", "1"), ("b", " y,z=%;\u{e9}"), ("c", "")]);
    let header = labels::format(&entries).expect("header");
    assert_eq!(labels::parse(&header), entries);
    assert_eq!(labels::format(&[]), None);
}

#[test]
fn encode_is_w3c_strict_and_round_trips() {
    let key = "k()<>@:/[]{}?\\\"";
    let value = "x\\y, z=;\"q\"%\u{e9}";
    let header = labels::format(&pairs(&[(key, value)])).expect("header");
    let (k, v) = header.split_once('=').expect("pair");
    let token = |b: u8| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b);
    assert!(k.bytes().all(token), "key not a token: {k}");
    let octet = |b: u8| matches!(b, 0x21 | 0x23..=0x2B | 0x2D..=0x3A | 0x3C..=0x5B | 0x5D..=0x7E);
    assert!(v.bytes().all(octet), "value not baggage-octet: {v}");
    assert!(v.starts_with("x%5Cy%2C%20z%3D%3B%22q%22%25"), "{v}");
    assert_eq!(labels::parse(&header), pairs(&[(key, value)]));
}

#[test]
fn with_label_replaces_existing() {
    let c = with_baggage("a=1,b=2");
    let out = carrier::with_label(&c, "a", "9");
    assert_eq!(out.baggage.as_deref(), Some("a=9,b=2"));
}

#[test]
fn with_label_appends_and_keeps_traceparent() {
    let c = with_baggage("a=1");
    let out = carrier::with_label(&c, "b", "x y");
    assert_eq!(out.baggage.as_deref(), Some("a=1,b=x%20y"));
    assert_eq!(out.traceparent, c.traceparent);
    let none = carrier::with_label(&TraceCarrier::default(), "k", "v");
    assert_eq!(none.baggage.as_deref(), Some("k=v"));
}

#[test]
fn without_label_removes_only_that_key() {
    let c = with_baggage("a=1,b=2,c=3");
    let out = carrier::without_label(&c, "b");
    assert_eq!(out.baggage.as_deref(), Some("a=1,c=3"));
    assert_eq!(out.traceparent, c.traceparent);
    let last = carrier::without_label(&with_baggage("a=1"), "a");
    assert_eq!(last.baggage, None);
}

#[test]
fn without_label_on_none_baggage_is_unchanged() {
    let c = TraceCarrier::default();
    assert_eq!(carrier::without_label(&c, "a"), c);
}

#[cfg(feature = "testing")]
mod traced {
    use super::*;
    use concord_telemetry::testing::{
        capture_exclusive, span_attr, spans, stamp_count, with_trace_env,
    };
    use concord_telemetry::{context, AttrValue};
    use tracing::info_span;

    #[test]
    fn with_label_invalid_value_leaves_carrier_and_reports() {
        let captured = capture_exclusive();
        let c = with_baggage("a=1");
        let out = carrier::with_label(&c, "b", &"x".repeat(257));
        assert_eq!(out, c);
        let dropped = captured.errors_where(|e| e.code == concord_telemetry::BAGGAGE_DROPPED_CODE);
        assert_eq!(dropped.len(), 1);
        assert_eq!(dropped[0].fields.get("dropped"), Some(&AttrValue::I64(1)));
    }

    fn dropped_count(captured: &concord_telemetry::testing::ExclusiveCapture) -> usize {
        captured
            .errors_where(|e| e.code == concord_telemetry::BAGGAGE_DROPPED_CODE)
            .len()
    }

    fn oversize() -> String {
        (0..40)
            .map(|i| format!("k{i}={}", "x".repeat(250)))
            .collect::<Vec<_>>()
            .join(",")
    }

    fn dirty_headers(extra: &str) -> Vec<String> {
        vec![
            oversize(),
            format!("a=1,{extra},bad"),
            format!("a=1,{extra},concord.x=2"),
        ]
    }

    #[test]
    fn with_label_on_dirty_or_oversize_baggage_forwards_only_the_new_label() {
        for header in dirty_headers("b=2") {
            let captured = capture_exclusive();
            let c = with_baggage(&header);
            let out = carrier::with_label(&c, "b", "9");
            assert_eq!(out.baggage.as_deref(), Some("b=9"), "{header}");
            assert_eq!(out.traceparent, c.traceparent);
            assert_eq!(dropped_count(&captured), 1);
        }
    }

    #[test]
    fn with_label_invalid_value_on_dirty_baggage_forwards_no_baggage() {
        let captured = capture_exclusive();
        let c = with_baggage("a=1,bad");
        let out = carrier::with_label(&c, "b", &"x".repeat(257));
        assert_eq!(out.baggage, None);
        assert_eq!(out.traceparent, c.traceparent);
        assert_eq!(dropped_count(&captured), 1);
    }

    #[test]
    fn without_label_on_dirty_or_oversize_baggage_drops_the_baggage() {
        for header in dirty_headers("b=2") {
            let captured = capture_exclusive();
            let c = with_baggage(&header);
            let out = carrier::without_label(&c, "b");
            assert_eq!(out.baggage, None, "{header}");
            assert_eq!(out.traceparent, c.traceparent);
            assert_eq!(dropped_count(&captured), 1);
        }
    }

    #[test]
    fn with_label_on_clean_header_does_not_report() {
        let captured = capture_exclusive();
        let out = carrier::with_label(&with_baggage("a=1"), "b", "2");
        assert_eq!(out.baggage.as_deref(), Some("a=1,b=2"));
        assert_eq!(dropped_count(&captured), 0);
    }

    #[test]
    fn without_label_on_clean_header_does_not_report() {
        let captured = capture_exclusive();
        let out = carrier::without_label(&with_baggage("a=1,b=2"), "b");
        assert_eq!(out.baggage.as_deref(), Some("a=1"));
        assert_eq!(dropped_count(&captured), 0);
    }

    fn exported(name: &str) -> opentelemetry_sdk::trace::SpanData {
        spans()
            .into_iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("no span {name}"))
    }

    #[test]
    fn add_once_does_not_double_stamp() {
        let _c = capture_exclusive();
        let before = stamp_count();
        let span = info_span!("lh-add-once");
        span.in_scope(|| {
            labels::add_once(&span, "agent_id", "A1");
            labels::add_once(&span, "agent_id", "A2");
            labels::add_once(&span, "empty", "");
            assert_eq!(labels::of(&span, "agent_id").as_deref(), Some("A1"));
            assert_eq!(labels::of(&span, "empty"), None);
        });
        drop(span);
        let data = exported("lh-add-once");
        // The span's end collapses `concord.label.agent_id` to `agent_id`;
        // the stamp counter shows the label was written once.
        assert_eq!(span_attr(&data, "agent_id").as_deref(), Some("A1"));
        assert_eq!(stamp_count() - before, 1);
    }

    #[test]
    fn record_once_skips_when_label_present() {
        let _c = capture_exclusive();
        let span = info_span!("lh-record-skip", agent_id = tracing::field::Empty);
        span.in_scope(|| context::add("agent_id", "A"));
        labels::record_once(&span, "agent_id", "B");
        drop(span);
        assert_eq!(
            span_attr(&exported("lh-record-skip"), "agent_id").as_deref(),
            Some("A")
        );

        let span = info_span!("lh-record-do", agent_id = tracing::field::Empty);
        labels::record_once(&span, "agent_id", "B");
        drop(span);
        assert_eq!(
            span_attr(&exported("lh-record-do"), "agent_id").as_deref(),
            Some("B")
        );
    }

    fn trace_id_of(c: &TraceCarrier) -> String {
        c.traceparent.as_ref().expect("traceparent")[3..35].to_owned()
    }

    fn origin_carrier(name: &'static str) -> TraceCarrier {
        let origin = info_span!("lh-origin", origin = name);
        origin.in_scope(carrier::capture)
    }

    #[test]
    fn parent_on_or_env_prefers_carrier() {
        let _c = capture_exclusive();
        let sent = origin_carrier("carrier");
        let env = "00-11111111111111111111111111111111-2222222222222222-01";
        let again = with_trace_env(Some(env), None, || {
            let span = tracing::Span::none().in_scope(|| info_span!("lh-prefers"));
            carrier::parent_on_or_env(&span, Some(&sent));
            span.in_scope(carrier::capture)
        });
        assert_eq!(trace_id_of(&again), trace_id_of(&sent));
    }

    #[test]
    fn falls_back_to_env() {
        let _c = capture_exclusive();
        let env = "00-11111111111111111111111111111111-2222222222222222-01";
        for given in [None, Some(TraceCarrier::default())] {
            let got = with_trace_env(Some(env), None, || {
                let span = tracing::Span::none().in_scope(|| info_span!("lh-fallback"));
                carrier::parent_on_or_env(&span, given.as_ref());
                span.in_scope(carrier::capture)
            });
            assert_eq!(trace_id_of(&got), "11111111111111111111111111111111");
        }
    }

    fn adopted_labels(
        carrier_in: Option<&TraceCarrier>,
        env_baggage: Option<&str>,
    ) -> Option<String> {
        let env = "00-11111111111111111111111111111111-2222222222222222-01";
        with_trace_env(Some(env), env_baggage, || {
            let span = tracing::Span::none().in_scope(|| info_span!("lh-sanitise"));
            carrier::parent_on_or_env(&span, carrier_in);
            span.in_scope(carrier::capture).baggage
        })
    }

    #[test]
    fn parent_on_or_env_does_not_adopt_dirty_env_baggage() {
        for header in dirty_headers("b=2") {
            let captured = capture_exclusive();
            let got = adopted_labels(None, Some(&header));
            assert_eq!(got, None, "{header}");
            assert_eq!(dropped_count(&captured), 1, "{header}");
        }
    }

    #[test]
    fn parent_on_or_env_does_not_adopt_dirty_carrier_baggage() {
        for header in dirty_headers("b=2") {
            let captured = capture_exclusive();
            let got = adopted_labels(Some(&with_baggage(&header)), None);
            assert_eq!(got, None, "{header}");
            assert_eq!(dropped_count(&captured), 1, "{header}");
        }
    }

    #[test]
    fn parent_on_or_env_adopts_clean_baggage() {
        let captured = capture_exclusive();
        let from_env = adopted_labels(None, Some("a=1,b=2"));
        assert_eq!(from_env.as_deref(), Some("a=1,b=2"));
        let from_carrier = adopted_labels(Some(&with_baggage("c=3")), Some("a=1"));
        assert_eq!(from_carrier.as_deref(), Some("c=3"));
        assert_eq!(dropped_count(&captured), 0);
    }
}
