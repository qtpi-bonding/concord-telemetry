#![cfg(feature = "testing")]
//! Spans exported by `testing::capture` carry their tracing level as the `level` attribute.
use concord_telemetry::testing::{capture_exclusive, span_attr, spans};
use tracing::{debug_span, info_span};

#[test]
fn captured_spans_carry_their_tracing_level() {
    let _capture = capture_exclusive();
    // The capture subscriber has no level filter, so DEBUG spans are exported too.
    info_span!("level_probe_info").in_scope(|| {});
    debug_span!("level_probe_debug").in_scope(|| {});
    let all = spans();
    let find = |name: &str| {
        all.iter()
            .find(|span| span.name == name)
            .unwrap_or_else(|| panic!("span {name} not captured"))
    };
    assert_eq!(
        span_attr(find("level_probe_info"), "level").as_deref(),
        Some("INFO")
    );
    assert_eq!(
        span_attr(find("level_probe_debug"), "level").as_deref(),
        Some("DEBUG")
    );
}
