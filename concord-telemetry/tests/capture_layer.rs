//! Own binary: shows `layer()` + `reader()` for a repo that installs its own
//! subscriber (so `capture()` is not an option).

#[allow(dead_code)]
#[path = "proto/fixtures.rs"]
mod fixtures;

use fixtures::{ActionStatusChanged, Status};
use tracing_subscriber::layer::SubscriberExt;

#[test]
fn layer_and_reader_work_under_a_caller_installed_subscriber() {
    let subscriber = tracing_subscriber::registry().with(concord_telemetry::testing::layer());
    tracing::subscriber::set_global_default(subscriber).expect("test installs its own subscriber");

    let event = ActionStatusChanged {
        source_record_id: "layer-1".to_owned(),
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
    concord_telemetry::event!(event.clone());

    let found = concord_telemetry::testing::reader()
        .single_event_where::<ActionStatusChanged>(|e| e.source_record_id == "layer-1");
    assert_eq!(found, event);
}
