//! Own binary: capture_exclusive only guarantees "exactly the records emitted
//! while held" when no other test in the process emits events.

#[allow(dead_code)]
#[path = "proto/fixtures.rs"]
mod fixtures;

use fixtures::{ActionStatusChanged, Status};

fn action(source_record_id: &str) -> ActionStatusChanged {
    ActionStatusChanged {
        source_record_id: source_record_id.to_owned(),
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
    }
}

#[test]
fn exclusive_capture_contains_exactly_the_records_emitted_while_held() {
    {
        let earlier = concord_telemetry::testing::capture_exclusive();
        concord_telemetry::event!(action("before"));
        assert_eq!(earlier.events::<ActionStatusChanged>().len(), 1);
    }

    let captured = concord_telemetry::testing::capture_exclusive();
    assert_eq!(captured.events::<ActionStatusChanged>().len(), 0);
    concord_telemetry::event!(action("during"));
    let events = captured.events::<ActionStatusChanged>();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].source_record_id, "during");
}
