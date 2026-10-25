// Its own binary: no OpenTelemetry layer, as in a process with no exporter.
use concord_telemetry::grpc::{client, ServerLayer};
use std::convert::Infallible;
use tower::{service_fn, Layer, ServiceExt};

#[test]
fn without_opentelemetry_requests_pass_through_untouched() {
    let _default = tracing::subscriber::set_default(tracing_subscriber::registry());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime");
    runtime.block_on(async {
        let request = http::Request::builder()
            .uri("/pkg.Service/Method")
            .body(())
            .expect("request");
        let sent = client(service_fn(|request: http::Request<()>| async move {
            Ok::<_, Infallible>(request)
        }))
        .oneshot(request)
        .await
        .expect("infallible");
        assert!(sent.headers().get("traceparent").is_none());

        let response = ServerLayer
            .layer(service_fn(|_: http::Request<()>| async {
                Ok::<_, Infallible>("served")
            }))
            .oneshot(sent)
            .await
            .expect("infallible");
        assert_eq!(response, "served");
    });
}
