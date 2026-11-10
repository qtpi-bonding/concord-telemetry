//! Under a subscriber without `BaggageLayer` (`testing::layer()` leaves it
//! out), `add_from_baggage` does nothing, and a header's drops are not
//! recorded.

use concord_telemetry::testing::{self, capture_exclusive};
use concord_telemetry::{carrier, context, BAGGAGE_DROPPED_CODE};
use tracing_subscriber::layer::SubscriberExt;

#[test]
fn add_from_baggage_without_the_layer_is_a_no_op() {
    let captured = capture_exclusive();
    let subscriber = tracing_subscriber::registry().with(testing::layer());
    tracing::subscriber::with_default(subscriber, || {
        let span = tracing::info_span!("afb-no-layer");
        span.in_scope(|| {
            context::add_from_baggage("concord.x=y,bad");
            assert!(carrier::capture().is_empty());
        });
    });
    assert!(captured
        .errors_where(|e| e.code == BAGGAGE_DROPPED_CODE)
        .is_empty());
}
