//! HTTP server tracing and ingress propagation.

use crate::{carrier, TraceCarrier};
use std::task::{Context, Poll};
use tower_layer::Layer;
use tower_service::Service;
use tracing::instrument::{Instrument, Instrumented};

/// Joins an incoming W3C trace, if present, in an `http.server` span.
#[derive(Clone, Copy, Debug, Default)]
pub struct IngressLayer;

#[derive(Clone, Debug)]
pub struct IngressService<S> {
    inner: S,
}

impl<S> Layer<S> for IngressLayer {
    type Service = IngressService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        IngressService { inner }
    }
}

impl<S, B> Service<http::Request<B>> for IngressService<S>
where
    S: Service<http::Request<B>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Instrumented<S::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), S::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: http::Request<B>) -> Self::Future {
        let span = tracing::info_span!(
            "http.server",
            otel.name = %request.uri().path(),
            otel.kind = "server",
            http.request.method = %request.method(),
        );
        let headers = request.headers();
        let incoming = TraceCarrier::from_pairs(|key| {
            headers
                .get(key)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        });
        carrier::extract_ingress("http", &span, &incoming);
        let future = span.in_scope(|| self.inner.call(request));
        future.instrument(span)
    }
}
