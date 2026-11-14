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

// ---- try_send / reserve / closed / poll_recv ----

use concord_telemetry::channel::TracedSender;
use std::future::Future;
use std::task::Poll;
use tokio::sync::mpsc::error::TrySendError;

/// The baggage `receiver`'s span carries, which names the span current when the sender sent.
async fn baggage_of(span: tracing::Span) -> Option<String> {
    async { carrier::capture().baggage }.instrument(span).await
}

fn tagged_span(name: &'static str, tag: &str) -> tracing::Span {
    let span = info_span!("sender", which = name);
    span.in_scope(|| concord_telemetry::context::add("tag", tag));
    span
}

#[tokio::test]
async fn try_send_delivers_with_the_span_current_at_the_call() {
    let _captured = capture_exclusive();
    let (sender, mut receiver) = traced(1);
    tagged_span("a", "T1").in_scope(|| sender.try_send(1u8).expect("room"));
    let (value, span) = receiver.recv().await.expect("message");
    assert_eq!(value, 1u8);
    assert_eq!(baggage_of(span).await.as_deref(), Some("tag=T1"));
}

#[tokio::test]
async fn try_send_into_a_full_buffer_gives_back_the_callers_value() {
    let (sender, _receiver) = traced(1);
    sender.try_send(String::from("first")).expect("room");
    match sender.try_send(String::from("second")) {
        Err(TrySendError::Full(value)) => assert_eq!(value, String::from("second")),
        other => panic!("expected Full, got {other:?}"),
    }
}

#[tokio::test]
async fn try_send_to_a_dropped_receiver_gives_back_the_callers_value() {
    let (sender, receiver) = traced(1);
    drop(receiver);
    match sender.try_send(String::from("lost")) {
        Err(TrySendError::Closed(value)) => assert_eq!(value, String::from("lost")),
        other => panic!("expected Closed, got {other:?}"),
    }
}

#[tokio::test]
async fn a_permit_sends_with_the_span_current_when_send_is_called_not_when_reserved() {
    let _captured = capture_exclusive();
    let (sender, mut receiver) = traced(1);
    let permit = tagged_span("reserve", "AT_RESERVE")
        .in_scope(|| sender.reserve())
        .await
        .expect("permit");
    tagged_span("send", "AT_SEND").in_scope(|| permit.send(9u8));
    let (value, span) = receiver.recv().await.expect("message");
    assert_eq!(value, 9u8);
    assert_eq!(baggage_of(span).await.as_deref(), Some("tag=AT_SEND"));
}

#[tokio::test]
async fn reserve_waits_while_full_and_completes_when_the_receiver_takes_one() {
    let (sender, mut receiver) = traced(1);
    sender.send(1u8).await.expect("send");
    let mut reserving = Box::pin(sender.reserve());
    assert!(
        futures_poll_once(reserving.as_mut()).is_pending(),
        "full buffer: reserve waits"
    );
    assert_eq!(receiver.recv().await.expect("one").0, 1u8);
    let permit = reserving.await.expect("room now");
    permit.send(2u8);
    assert_eq!(receiver.recv().await.expect("two").0, 2u8);
}

#[tokio::test]
async fn a_reserve_dropped_while_pending_loses_nothing_and_a_new_reserve_works() {
    let (sender, mut receiver) = traced(1);
    sender.send(1u8).await.expect("send");
    {
        let mut reserving = Box::pin(sender.reserve());
        assert!(futures_poll_once(reserving.as_mut()).is_pending());
    } // dropped while pending, as a select! branch is
    assert_eq!(receiver.recv().await.expect("kept").0, 1u8);
    let permit = sender.reserve().await.expect("fresh reserve");
    permit.send(2u8);
    assert_eq!(receiver.recv().await.expect("two").0, 2u8);
}

#[tokio::test]
async fn a_permit_dropped_without_send_frees_its_slot() {
    let (sender, mut receiver) = traced(1);
    let permit = sender.reserve().await.expect("permit");
    assert!(
        matches!(sender.try_send(1u8), Err(TrySendError::Full(1))),
        "the permit holds the only slot"
    );
    drop(permit);
    sender.try_send(2u8).expect("slot freed");
    assert_eq!(receiver.recv().await.expect("two").0, 2u8);
}

#[tokio::test]
async fn closed_resolves_when_the_receiver_is_dropped() {
    let (sender, receiver) = traced::<u8>(1);
    assert!(!sender.is_closed());
    let waiting = tokio::spawn({
        let sender: TracedSender<u8> = sender.clone();
        async move { sender.closed().await }
    });
    tokio::task::yield_now().await;
    assert!(!waiting.is_finished());
    drop(receiver);
    tokio::time::timeout(std::time::Duration::from_secs(5), waiting)
        .await
        .expect("closed() resolved")
        .expect("task");
    assert!(sender.is_closed());
}

#[tokio::test]
async fn poll_recv_is_pending_when_empty_and_ready_none_after_all_senders_drop() {
    let (sender, mut receiver) = traced::<u8>(2);
    let poll = |receiver: &mut concord_telemetry::channel::TracedReceiver<u8>| {
        let waker = std::task::Waker::noop();
        let mut cx = std::task::Context::from_waker(waker);
        receiver.poll_recv(&mut cx)
    };
    assert!(matches!(poll(&mut receiver), Poll::Pending));
    sender.try_send(5).expect("room");
    match poll(&mut receiver) {
        Poll::Ready(Some((value, _span))) => assert_eq!(value, 5),
        other => panic!("expected the value, got {other:?}"),
    }
    drop(sender);
    assert!(matches!(poll(&mut receiver), Poll::Ready(None)));
}

#[tokio::test]
async fn close_stops_new_sends_but_keeps_queued_values() {
    let (sender, mut receiver) = traced(2);
    sender.try_send(1u8).expect("room");
    receiver.close();
    assert!(matches!(sender.try_send(2u8), Err(TrySendError::Closed(2))));
    assert_eq!(receiver.recv().await.expect("queued").0, 1u8);
    assert!(receiver.recv().await.is_none());
}

fn futures_poll_once<F: Future>(mut f: std::pin::Pin<&mut F>) -> Poll<F::Output> {
    let waker = std::task::Waker::noop();
    let mut cx = std::task::Context::from_waker(waker);
    f.as_mut().poll(&mut cx)
}
