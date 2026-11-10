#![cfg(feature = "http-server")]

use concord_telemetry::carrier;
use concord_telemetry::http_server::{IngressLayer, ServerSpan};
use concord_telemetry::testing::{capture_exclusive, span_attr, spans};
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
            Ok::<_, Infallible>(http::Response::new(carrier::capture().traceparent))
        }))
        .oneshot(request)
        .instrument(tracing::Span::none())
        .await
        .expect("infallible")
        .into_body()
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

fn response(status: u16) -> http::Response<()> {
    http::Response::builder()
        .status(status)
        .body(())
        .expect("response")
}

fn server_spans() -> Vec<opentelemetry_sdk::trace::SpanData> {
    spans()
        .into_iter()
        .filter(|s| {
            s.attributes
                .iter()
                .any(|kv| kv.key.as_str() == "http.request.method")
        })
        .collect()
}

async fn serve_with(
    request: Request,
    handler: impl Fn(Request) -> http::Response<()> + Clone + Send + 'static,
) {
    IngressLayer
        .layer(service_fn(move |request: Request| {
            let handler = handler.clone();
            async move { Ok::<_, Infallible>(handler(request)) }
        }))
        .oneshot(request)
        .instrument(tracing::Span::none())
        .await
        .expect("infallible");
}

#[tokio::test]
async fn an_http_server_span_is_named_by_its_method_and_records_the_status() {
    let _captured = capture_exclusive();
    let request = http::Request::builder()
        .uri("/items/42")
        .body(())
        .expect("request");
    serve_with(request, |_| response(404)).await;
    let found = server_spans();
    assert_eq!(found.len(), 1);
    // The path is not in the name: it is unbounded until a router has matched.
    assert_eq!(found[0].name, "GET");
    assert_eq!(
        span_attr(&found[0], "http.response.status_code").as_deref(),
        Some("404")
    );
    assert_eq!(span_attr(&found[0], "http.route"), None);
}

#[tokio::test]
async fn a_service_that_matched_a_route_names_the_span_after_it() {
    let _captured = capture_exclusive();
    let request = http::Request::builder()
        .method("POST")
        .uri("/items/42")
        .body(())
        .expect("request");
    serve_with(request, |request| {
        request
            .extensions()
            .get::<ServerSpan>()
            .expect("the ingress layer provides the span")
            .record_route("/items/{id}");
        response(201)
    })
    .await;
    let found = server_spans();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "POST /items/{id}");
    assert_eq!(
        span_attr(&found[0], "http.route").as_deref(),
        Some("/items/{id}")
    );
    assert_eq!(
        span_attr(&found[0], "http.response.status_code").as_deref(),
        Some("201")
    );
}
