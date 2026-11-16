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
    use concord_telemetry::testing::{capture_exclusive, span_attr, spans, stamp_count};
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

    static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Sets the trace environment for `f` and restores it. A local stand-in:
    /// the `testing::with_trace_env` helper replaces it.
    fn with_trace_env_local<R>(
        traceparent: Option<&str>,
        baggage: Option<&str>,
        f: impl FnOnce() -> R,
    ) -> R {
        use concord_telemetry::propagation::{BAGGAGE_ENV, TRACEPARENT_ENV};
        let _lock = ENV.lock().unwrap_or_else(|p| p.into_inner());
        let saved = [TRACEPARENT_ENV, BAGGAGE_ENV].map(|k| (k, std::env::var_os(k)));
        for (key, value) in [(TRACEPARENT_ENV, traceparent), (BAGGAGE_ENV, baggage)] {
            match value {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
        struct Restore([(&'static str, Option<std::ffi::OsString>); 2]);
        impl Drop for Restore {
            fn drop(&mut self) {
                for (key, value) in &self.0 {
                    match value {
                        Some(v) => std::env::set_var(key, v),
                        None => std::env::remove_var(key),
                    }
                }
            }
        }
        let _restore = Restore(saved);
        f()
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
        let again = with_trace_env_local(Some(env), None, || {
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
            let got = with_trace_env_local(Some(env), None, || {
                let span = tracing::Span::none().in_scope(|| info_span!("lh-fallback"));
                carrier::parent_on_or_env(&span, given.as_ref());
                span.in_scope(carrier::capture)
            });
            assert_eq!(trace_id_of(&got), "11111111111111111111111111111111");
        }
    }
}
