//! Own binary: exported log records carry the span keys for code location.

#[allow(dead_code)]
#[path = "proto/fixtures.rs"]
mod fixtures;

use concord_telemetry::{AttrValue, Attrs, Event};
use fixtures::{ActionStatusChanged, Status};

#[derive(Debug, thiserror::Error, coded_error::CodedError)]
#[coded(namespace = "test.location")]
enum LocationError {
    #[error("location failure")]
    #[coded(code = "failed", class = Logical)]
    Failed,
}

fn assert_location(attrs: &Attrs, line: u32) {
    assert_eq!(
        attrs.get("code.file.path"),
        Some(&AttrValue::Str(file!().to_owned()))
    );
    assert_eq!(
        attrs.get("code.line.number"),
        Some(&AttrValue::I64(i64::from(line)))
    );
    assert_eq!(
        attrs.get("code.module.name"),
        Some(&AttrValue::Str(module_path!().to_owned()))
    );
    for old in [
        "code.filepath",
        "code.filename",
        "code.lineno",
        "code.namespace",
    ] {
        assert!(!attrs.contains_key(old), "{old} must not be emitted");
    }
}

#[test]
fn event_and_error_records_carry_the_callers_location() {
    let captured = concord_telemetry::testing::capture_exclusive();
    let event = ActionStatusChanged {
        source_record_id: "loc".to_owned(),
        action_id: "action".to_owned(),
        old_status: Status::Ready as i32,
        new_status: Status::Running as i32,
        principal: None,
        at: None,
        took: None,
        attempt: 1,
        retried: false,
        labels: Vec::new(),
        note: None,
    };
    let event_line = line!() + 1;
    concord_telemetry::event!(event);
    let error_line = line!() + 1;
    concord_telemetry::error!(&LocationError::Failed);

    let events = captured.attrs_for(ActionStatusChanged::TYPE_NAME);
    assert_eq!(events.len(), 1);
    assert_location(&events[0], event_line);

    let errors = captured.attrs_for("error");
    assert_eq!(errors.len(), 1);
    assert_location(&errors[0], error_line);
}
