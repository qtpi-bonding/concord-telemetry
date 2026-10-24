use opentelemetry::trace::TraceContextExt as _;
#[allow(dead_code)]
#[path = "proto/fixtures.rs"]
mod fixtures;

use concord_telemetry::testing::capture;
use concord_telemetry::{error, AttrValue, Attrs, Event, RebuildError};
use fixtures::{ActionStatusChanged, Scalars, Status};
use std::sync::atomic::{AtomicU64, Ordering};
use tracing::{event, info_span};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn id() -> String {
    format!("capture-{}", NEXT_ID.fetch_add(1, Ordering::Relaxed))
}

fn action(source_record_id: String) -> ActionStatusChanged {
    ActionStatusChanged {
        source_record_id,
        action_id: "action".to_owned(),
        old_status: Status::Ready as i32,
        new_status: Status::Running as i32,
        principal: None,
        at: None,
        took: None,
        attempt: 1,
        retried: false,
        labels: vec!["test".to_owned()],
        note: None,
    }
}

#[test]
fn captures_events_from_async_and_thread_workers() {
    let _capture = capture();
    let async_id = id();
    let thread_id = id();
    let blocking_id = id();
    let expected_ids = [async_id.clone(), thread_id.clone(), blocking_id.clone()];

    std::thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().expect("create test runtime");
        runtime.block_on(async move {
            tokio::spawn(async move { concord_telemetry::event!(action(async_id)) })
                .await
                .expect("join worker");
            tokio::task::spawn_blocking(move || concord_telemetry::event!(action(blocking_id)))
                .await
                .expect("join blocking worker");
        });
    })
    .join()
    .expect("join runtime thread");

    let thread = std::thread::spawn(move || concord_telemetry::event!(action(thread_id)));
    thread.join().expect("join ordinary thread");

    let captured = capture();
    for source_record_id in expected_ids {
        let event = captured.single_event_where::<ActionStatusChanged>(|event| {
            event.source_record_id == source_record_id
        });
        assert_eq!(event.action_id, "action");
    }
}

#[test]
fn concurrent_tests_can_filter_by_unique_domain_ids() {
    let left = id();
    let right = id();
    let _capture = capture();
    let left_for_thread = left.clone();
    let right_for_thread = right.clone();

    let first = std::thread::spawn(move || {
        let captured = capture();
        concord_telemetry::event!(action(left_for_thread));
        captured.single_event_where::<ActionStatusChanged>(|event| event.source_record_id == left);
    });
    let second = std::thread::spawn(move || {
        let captured = capture();
        concord_telemetry::event!(action(right_for_thread));
        captured.single_event_where::<ActionStatusChanged>(|event| event.source_record_id == right);
    });

    first.join().expect("left capture");
    second.join().expect("right capture");
}

#[derive(Debug, thiserror::Error, coded_error::CodedError)]
#[coded(namespace = "capture.test")]
enum CaptureError {
    #[error("not found")]
    #[coded(code = "not_found", class = Logical)]
    NotFound,
}

#[test]
fn captures_coded_error_fields() {
    let captured = capture();
    let error = CaptureError::NotFound;
    error!(&error, source = "capture.source");
    let expected_fingerprint = coded_error::fingerprint(
        "capture.test.not_found",
        coded_error::ErrorClass::Logical,
        "capture.source",
    );

    let records = captured.errors_where(|record| {
        record.code == "capture.test.not_found" && record.fingerprint == expected_fingerprint
    });
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].source, "capture.source");
}

#[test]
fn appender_preserves_event_name() {
    let source_record_id = id();
    let captured = capture();
    concord_telemetry::event!(action(source_record_id.clone()));
    let event = captured.single_event_where::<ActionStatusChanged>(|event| {
        event.source_record_id == source_record_id
    });
    assert_eq!(
        ActionStatusChanged::TYPE_NAME,
        "test.events.v1.ActionStatusChanged"
    );
    assert_eq!(event.source_record_id, source_record_id);
}

#[test]
fn scalar_attributes_round_trip_through_otel() {
    let _capture = capture();
    let invalid_id = id();
    let invalid = Scalars {
        source_record_id: invalid_id.clone(),
        big: u64::MAX,
        count: u32::MAX,
        signed: i64::MIN,
        ratio: 1.25,
        precise: -9.5,
        enabled: true,
        status: Some(99),
        maybe_big: None,
        maybe_count: None,
        maybe_text: None,
        statuses: Vec::new(),
        big_values: Vec::new(),
    };
    let minimum_id = id();
    let minimum = Scalars {
        source_record_id: minimum_id.clone(),
        big: u64::MAX,
        count: u32::MAX,
        signed: i64::MIN,
        ratio: 1.25,
        precise: -9.5,
        enabled: true,
        status: Some(Status::Ready as i32),
        maybe_big: None,
        maybe_count: None,
        maybe_text: None,
        statuses: Vec::new(),
        big_values: Vec::new(),
    };
    let maximum_id = id();
    let maximum = Scalars {
        source_record_id: maximum_id.clone(),
        big: u64::MAX,
        count: u32::MAX,
        signed: i64::MAX,
        ratio: 1.25,
        precise: -9.5,
        enabled: true,
        status: Some(Status::Ready as i32),
        maybe_big: None,
        maybe_count: None,
        maybe_text: None,
        statuses: Vec::new(),
        big_values: Vec::new(),
    };
    minimum.emit();
    maximum.emit();
    invalid.emit();

    let captured = capture();
    let minimum_event =
        captured.single_event_where::<Scalars>(|event| event.source_record_id == minimum_id);
    assert_eq!(minimum_event.big, u64::MAX);
    assert_eq!(minimum_event.count, u32::MAX);
    assert_eq!(minimum_event.signed, i64::MIN);
    assert!(minimum_event.enabled);

    let maximum_event =
        captured.single_event_where::<Scalars>(|event| event.source_record_id == maximum_id);
    assert_eq!(maximum_event.signed, i64::MAX);
    assert_eq!(maximum_event.precise, -9.5);
    assert!(
        captured
            .events_where::<Scalars>(|event| { event.source_record_id == invalid_id })
            .is_empty(),
        "an unknown enum value must not rebuild"
    );

    let mut invalid_attrs = Attrs::new();
    invalid_attrs.insert("source_record_id".to_owned(), AttrValue::Str(invalid_id));
    invalid_attrs.insert("big".to_owned(), AttrValue::Str(u64::MAX.to_string()));
    invalid_attrs.insert("count".to_owned(), AttrValue::I64(i64::from(u32::MAX)));
    invalid_attrs.insert("signed".to_owned(), AttrValue::I64(i64::MIN));
    invalid_attrs.insert("ratio".to_owned(), AttrValue::F64(1.25));
    invalid_attrs.insert("precise".to_owned(), AttrValue::F64(-9.5));
    invalid_attrs.insert("enabled".to_owned(), AttrValue::Bool(true));
    invalid_attrs.insert(
        "status".to_owned(),
        AttrValue::Str("NOT_A_STATUS".to_owned()),
    );
    invalid_attrs.insert("statuses".to_owned(), AttrValue::Str("[]".to_owned()));
    invalid_attrs.insert("big_values".to_owned(), AttrValue::Str("[]".to_owned()));
    assert!(matches!(
        Scalars::from_attributes(&invalid_attrs),
        Err(RebuildError::Parse(_))
    ));
}

#[test]
fn log_records_have_the_current_span_trace_id() {
    let captured = capture();
    let span = info_span!("op");
    let _entered = span.enter();
    let expected_trace_id = {
        use tracing_opentelemetry::OpenTelemetrySpanExt;
        span.context().span().span_context().trace_id()
    };
    event!(
        name: "trace-capture",
        target: concord_telemetry::EVENT_TARGET,
        tracing::Level::INFO,
        source_record_id = id()
    );

    let actual_trace_id = captured
        .trace_id_for("trace-capture")
        .expect("captured log should have trace context");
    assert_eq!(actual_trace_id, expected_trace_id.to_bytes());
}
