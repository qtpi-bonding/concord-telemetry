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
