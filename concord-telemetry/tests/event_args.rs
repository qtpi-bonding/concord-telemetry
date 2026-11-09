//! `event!` evaluates its argument outside any closure: `.await` and `?` work in it.

#[allow(dead_code)]
#[path = "proto/fixtures.rs"]
mod fixtures;

use fixtures::{ActionStatusChanged, Status};

fn action(id: &str) -> ActionStatusChanged {
    ActionStatusChanged {
        source_record_id: id.to_owned(),
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

async fn make(id: &str) -> Result<ActionStatusChanged, std::convert::Infallible> {
    Ok(action(id))
}

async fn emit_with_await_and_question_mark() -> Result<(), std::convert::Infallible> {
    concord_telemetry::event!(make("awaited").await?);
    Ok(())
}

#[test]
fn await_and_question_mark_compile_and_emit() {
    let captured = concord_telemetry::testing::capture_exclusive();
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(emit_with_await_and_question_mark())
        .unwrap();
    let found = captured.events_where::<ActionStatusChanged>(|e| e.source_record_id == "awaited");
    assert_eq!(found.len(), 1);
}

struct Holder {
    ev: ActionStatusChanged,
}

#[test]
fn event_borrows_its_argument() {
    let captured = concord_telemetry::testing::capture_exclusive();
    let named = action("named");
    concord_telemetry::event!(named);
    concord_telemetry::event!(named); // still usable after the first emit
    assert_eq!(named.source_record_id, "named");
    let holder = Holder {
        ev: action("field"),
    };
    concord_telemetry::event!(holder.ev);
    let boxed = Box::new(action("boxed"));
    concord_telemetry::event!(*boxed);
    assert_eq!(holder.ev.source_record_id, "field");
    assert_eq!(boxed.source_record_id, "boxed");
    let count = |id: &str| {
        captured
            .events_where::<ActionStatusChanged>(|e| e.source_record_id == id)
            .len()
    };
    assert_eq!(count("named"), 2);
    assert_eq!(count("field"), 1);
    assert_eq!(count("boxed"), 1);
}
