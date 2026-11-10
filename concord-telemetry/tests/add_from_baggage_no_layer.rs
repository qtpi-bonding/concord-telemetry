//! No subscriber, so no baggage layer: `add_from_baggage` does nothing, and a
//! header's drops are not recorded.

use concord_telemetry::{carrier, context};

#[test]
fn add_from_baggage_without_the_layer_is_a_no_op() {
    let span = tracing::info_span!("afb-no-layer");
    span.in_scope(|| {
        context::add_from_baggage("agent_id=A1,concord.x=y,bad");
        assert!(carrier::capture().is_empty());
    });
}
