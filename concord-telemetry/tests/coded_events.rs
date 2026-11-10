//! The library's own coded records share one shape: code, class,
//! fingerprint of (code, class, source), source, and their one extra field.

use coded_error::{fingerprint, ErrorClass};
use concord_telemetry::carrier;
use concord_telemetry::testing::{capture_exclusive, set_otel_active};
use concord_telemetry::{
    AttrValue, TraceCarrier, BAGGAGE_DROPPED_CODE, CARRIER_DROPPED_CODE, TRACE_MISSING_CODE,
};
use tracing::info_span;

#[test]
fn the_library_s_coded_records_carry_a_matching_fingerprint_and_their_field() {
    let captured = capture_exclusive();
    set_otel_active(true);
    let missing = info_span!("coded-missing");
    carrier::extract_required("test.hop", &missing, &TraceCarrier::default());
    let bad = info_span!("coded-bad");
    carrier::parent_on(
        &bad,
        &TraceCarrier {
            traceparent: Some("nope".into()),
            baggage: Some("a=b=c".into()),
            ..TraceCarrier::default()
        },
    );
    set_otel_active(false);

    let cases = [
        (
            TRACE_MISSING_CODE,
            ErrorClass::Infrastructure,
            "hop",
            AttrValue::Str("test.hop".into()),
        ),
        (
            CARRIER_DROPPED_CODE,
            ErrorClass::Logical,
            "header",
            AttrValue::Str("traceparent".into()),
        ),
        (
            BAGGAGE_DROPPED_CODE,
            ErrorClass::Logical,
            "dropped",
            AttrValue::I64(1),
        ),
    ];
    for (code, class, field, value) in cases {
        let found = captured.errors_where(|e| e.code == code);
        assert_eq!(found.len(), 1, "{code}");
        let record = &found[0];
        assert_eq!(record.class, class.as_str());
        assert_eq!(record.source, "concord_telemetry::carrier");
        assert_eq!(
            record.fingerprint,
            fingerprint(code, class, &record.source).as_str()
        );
        assert_eq!(record.fields.get(field), Some(&value), "{code}");
    }
}
