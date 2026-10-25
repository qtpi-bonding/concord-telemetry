use concord_telemetry::grpc::{client, ServerLayer};
use concord_telemetry::propagation::traceparent;
use concord_telemetry::testing::capture_exclusive;
use concord_telemetry::EVENT_TARGET;
use std::convert::Infallible;
use tower::{service_fn, Layer, ServiceExt};
use tracing::{event, info_span, Instrument, Level};

const TRACE_ID: &str = "0af7651916cd43dd8448eb211c80319c";
const PARENT: &str = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";

type Request = http::Request<()>;

fn hex(bytes: [u8; 16]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn request(traceparent: Option<&str>) -> Request {
    let mut builder = http::Request::builder().uri("/pkg.Service/Method");
    if let Some(value) = traceparent {
        builder = builder.header("traceparent", value);
    }
    builder.body(()).expect("request")
}

/// The request as the client layer would send it.
async fn sent(request: Request) -> Request {
    client(service_fn(|request: Request| async move {
        Ok::<_, Infallible>(request)
    }))
    .oneshot(request)
    .await
    .expect("infallible")
}

/// Serves `request` through the server layer; the handler emits `name`.
async fn served(name: &'static str, request: Request) {
    let handler = service_fn(move |_: Request| async move {
        match name {
            "grpc-joined" => {
                event!(name: "grpc-joined", target: EVENT_TARGET, Level::INFO, marker = "joined")
            }
            "grpc-end-to-end" => {
                event!(name: "grpc-end-to-end", target: EVENT_TARGET, Level::INFO, marker = "e2e")
            }
            _ => {
                event!(name: "grpc-stripped", target: EVENT_TARGET, Level::INFO, marker = "stripped")
            }
        }
        Ok::<_, Infallible>(())
    });
    ServerLayer
        .layer(handler)
        .oneshot(request)
        .instrument(tracing::Span::none())
        .await
        .expect("infallible");
}

fn header(request: &Request) -> Option<String> {
    request
        .headers()
        .get("traceparent")
        .map(|value| value.to_str().expect("ascii").to_owned())
}

#[tokio::test]
async fn the_server_joins_an_incoming_trace() {
    let captured = capture_exclusive();
    served("grpc-joined", request(Some(PARENT))).await;
    let actual = captured.trace_id_for("grpc-joined").expect("trace context");
    assert_eq!(hex(actual), TRACE_ID);
}

#[tokio::test]
async fn the_client_sends_a_trace_without_a_caller_span() {
    let _captured = capture_exclusive();
    let request = sent(request(None)).instrument(tracing::Span::none()).await;
    let value = header(&request).expect("traceparent sent");
    assert_eq!(value.len(), 55, "{value}");
    assert!(value.starts_with("00-"), "{value}");
    assert!(request.headers().get("tracestate").is_none());
}

#[tokio::test]
async fn the_client_sends_the_callers_trace() {
    let _captured = capture_exclusive();
    let caller = info_span!("caller");
    let expected = caller.in_scope(|| traceparent().expect("caller traceparent"));
    let request = sent(request(None)).instrument(caller).await;
    let value = header(&request).expect("traceparent sent");
    assert_eq!(value[3..35], expected[3..35]);
    assert_eq!(request.headers().get_all("traceparent").iter().count(), 1);
}

#[tokio::test]
async fn one_trace_spans_client_and_server() {
    let captured = capture_exclusive();
    let caller = info_span!("caller");
    let expected = caller.in_scope(|| traceparent().expect("caller traceparent"));
    let request = sent(request(None)).instrument(caller).await;
    // Served outside the caller span: only the header can carry the trace.
    served("grpc-end-to-end", request).await;
    let actual = captured
        .trace_id_for("grpc-end-to-end")
        .expect("trace context");
    assert_eq!(hex(actual), expected[3..35]);
}

#[tokio::test]
async fn without_the_header_the_server_starts_its_own_trace() {
    let captured = capture_exclusive();
    let caller = info_span!("caller");
    let expected = caller.in_scope(|| traceparent().expect("caller traceparent"));
    let mut request = sent(request(None)).instrument(caller).await;
    request.headers_mut().remove("traceparent");
    served("grpc-stripped", request).await;
    let actual = captured
        .trace_id_for("grpc-stripped")
        .expect("trace context");
    assert_ne!(hex(actual), expected[3..35]);
}
