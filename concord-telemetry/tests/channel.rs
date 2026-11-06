#![cfg(feature = "channel")]

use concord_telemetry::carrier;
use concord_telemetry::channel::{spawn_traced, traced};
use concord_telemetry::testing::capture_exclusive;
use concord_telemetry::EVENT_TARGET;
use tracing::{event, info_span, Instrument, Level};

#[tokio::test]
async fn traced_channel_hands_receiver_the_senders_span() {
    let captured = capture_exclusive();
    let (sender, mut receiver) = traced(1);
    let sender_span = info_span!("sender");
    sender_span.in_scope(|| concord_telemetry::context::add("action_id", "A6"));
    async {
        event!(name: "channel-sender", target: EVENT_TARGET, Level::INFO, marker = "sent");
        sender.send(7u8).await.expect("send");
    }
    .instrument(sender_span)
    .await;

    let (value, span) = receiver.recv().await.expect("message");
    async move {
        assert_eq!(value, 7u8);
        event!(name: "channel-child", target: EVENT_TARGET, Level::INFO, marker = "received");
        assert_eq!(carrier::capture().baggage.as_deref(), Some("action_id=A6"));
    }
    .instrument(span)
    .await;
    let sender_trace = captured
        .trace_id_for("channel-sender")
        .expect("sender trace");
    assert_eq!(captured.trace_id_for("channel-child"), Some(sender_trace));
}

#[tokio::test]
async fn spawn_traced_keeps_baggage() {
    let _captured = capture_exclusive();
    let span = info_span!("parent");
    let carrier = span
        .in_scope(|| {
            concord_telemetry::context::add("request_id", "R6");
            spawn_traced(async { carrier::capture() })
        })
        .await
        .expect("task");
    assert_eq!(carrier.baggage.as_deref(), Some("request_id=R6"));
}
