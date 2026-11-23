//! Own binary: the assertions read exclusive captures and span exports.
#![cfg(feature = "testing")]

use coded_error::CodedError as _;
use concord_telemetry::testing::{capture_exclusive, span_attr, spans};
use concord_telemetry::{error, warn, warn_anyhow, AttrValue};
use opentelemetry::trace::Status;

#[derive(Debug, thiserror::Error, coded_error::CodedError)]
#[coded(namespace = "test.warn")]
enum WarnError {
    #[error("degraded but handled")]
    #[coded(code = "degraded", class = Logical)]
    Degraded,
}

#[test]
fn warn_records_a_coded_warn() {
    let capture = capture_exclusive();
    let err = WarnError::Degraded;
    warn!(&err, source = "s", action_id = "A1");

    let records = capture.errors_where(|_| true);
    assert_eq!(records.len(), 1, "{records:?}");
    let record = &records[0];
    assert_eq!(record.code, err.code());
    assert_eq!(record.level, "WARN");
    assert_eq!(record.source, "s");
    assert_eq!(
        record.fields.get("action_id"),
        Some(&AttrValue::Str("A1".into()))
    );
}

#[test]
fn warn_anyhow_records_unknown_without_a_coded_cause() {
    let capture = capture_exclusive();
    let plain = anyhow::anyhow!("boom");
    warn_anyhow!(&plain, source = "s");

    let records = capture.errors_where(|_| true);
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(records[0].code, "unknown");
    assert_eq!(records[0].level, "WARN");
    assert_eq!(records[0].source, "s");
}

#[test]
fn warn_does_not_mark_the_span_failed() {
    let capture = capture_exclusive();
    {
        let span = tracing::info_span!("warn-span");
        let _entered = span.enter();
        warn!(&WarnError::Degraded, source = "s");
    }
    {
        let span = tracing::info_span!("error-span");
        let _entered = span.enter();
        error!(&WarnError::Degraded, source = "s");
    }
    assert_eq!(capture.errors_where(|_| true).len(), 2);

    let exported = spans();
    let warned = exported
        .iter()
        .find(|span| span.name == "warn-span")
        .expect("warn-span exported");
    assert_eq!(span_attr(warned, "error.code"), None);
    assert_ne!(
        warned.status,
        Status::Error {
            description: "error".into()
        }
    );

    let errored = exported
        .iter()
        .find(|span| span.name == "error-span")
        .expect("error-span exported");
    assert!(span_attr(errored, "error.code").is_some());
    assert_eq!(
        errored.status,
        Status::Error {
            description: "error".into()
        }
    );
}
