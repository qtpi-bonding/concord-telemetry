#![cfg(feature = "http-server")]

use concord_telemetry::carrier;
use concord_telemetry::http_server::IngressLayer;
use concord_telemetry::testing::capture_exclusive;
use concord_telemetry::EVENT_TARGET;
use std::convert::Infallible;
use tower::{service_fn, Layer, ServiceExt};
use tracing::{event, Instrument, Level};

const PARENT: &str = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";
type Request = http::Request<()>;

async fn served(request: Request) -> Option<String> {
    IngressLayer
        .layer(service_fn(|_: Request| async move {
            event!(name: "http-ingress", target: EVENT_TARGET, Level::INFO, marker = "seen");
            Ok::<_, Infallible>(carrier::capture().traceparent)
        }))
        .oneshot(request)
        .instrument(tracing::Span::none())
        .await
        .expect("infallible")
}

#[tokio::test]
async fn http_ingress_joins_a_traceparent_and_is_silent_without_one() {
    let captured = capture_exclusive();
    let request = http::Request::builder()
        .uri("/health")
        .header("traceparent", PARENT)
        .body(())
        .expect("request");
    let joined = served(request).await.expect("carrier");
    // Same trace; the span id is the http.server span's own, not the caller's.
    let trace_id = |tp: &str| tp.split('-').nth(1).map(str::to_owned);
    assert_eq!(trace_id(&joined), trace_id(PARENT));
    assert_ne!(joined, PARENT);
    assert!(captured.trace_id_for("http-ingress").is_some());

    let request = http::Request::builder()
        .uri("/health")
        .body(())
        .expect("request");
    served(request).await;
    assert!(captured.errors_where(|_| true).is_empty());
}
