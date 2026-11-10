//! HTTP server tracing and ingress propagation.
//!
//! The span is named `{method}` when the request arrives: the layer sits
//! outside routing, so the path is unbounded (ids) and the route unknown. A
//! service that has matched a route records it through the [`ServerSpan`] in
//! the request's extensions, which renames the span `{method} {route}` and sets
//! `http.route`. `http.response.status_code` is recorded when the response
//! future is ready. `rpc.grpc.status_code` is not recorded: it lives in
//! trailers, which needs a body wrapper (see concordium's
//! `docs/concord-telemetry-deferred.md`).

use crate::{carrier, init::HTTP_TARGET, TraceCarrier};
use pin_project_lite::pin_project;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tower_layer::Layer;
use tower_service::Service;
use tracing::Span;

/// Joins an incoming W3C trace, if present, in an `http.server` span.
#[derive(Clone, Copy, Debug, Default)]
pub struct IngressLayer;

#[derive(Clone, Debug)]
pub struct IngressService<S> {
    inner: S,
}

/// The request's `http.server` span, in its extensions.
#[derive(Clone, Debug)]
pub struct ServerSpan {
    span: Span,
    method: http::Method,
}

impl ServerSpan {
    /// Records the matched route (a template such as `/items/{id}`, never the
    /// concrete path): `http.route`, and the span's name `{method} {route}`.
    pub fn record_route(&self, route: &str) {
        use opentelemetry::trace::TraceContextExt as _;
        use tracing_opentelemetry::OpenTelemetrySpanExt as _;
        self.span.record("http.route", route);
        // The span has started by now (the handler runs inside it), and
        // tracing-opentelemetry ignores an `otel.name` recorded after that.
        self.span
            .context()
            .span()
            .update_name(format!("{} {route}", self.method));
    }

    /// The span itself.
    pub fn span(&self) -> &Span {
        &self.span
    }
}

impl<S> Layer<S> for IngressLayer {
    type Service = IngressService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        IngressService { inner }
    }
}

pin_project! {
    /// The inner service's future, run inside the `http.server` span, that
    /// records the response's status when it is ready. Dropping it drops the
    /// inner future inside the span, as `tracing`'s `Instrumented` does.
    #[project = ResponseFutureProj]
    pub struct ResponseFuture<F> {
        #[pin]
        inner: Option<F>,
        span: Span,
    }

    impl<F> PinnedDrop for ResponseFuture<F> {
        fn drop(this: Pin<&mut Self>) {
            let mut this = this.project();
            let _entered = this.span.enter();
            this.inner.set(None);
        }
    }
}

impl<F, B, E> Future for ResponseFuture<F>
where
    F: Future<Output = Result<http::Response<B>, E>>,
{
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let ResponseFutureProj { inner, span } = self.project();
        let _entered = span.enter();
        let Some(future) = inner.as_pin_mut() else {
            panic!("ResponseFuture polled after it was dropped");
        };
        let poll = future.poll(cx);
        if let Poll::Ready(Ok(response)) = &poll {
            span.record(
                "http.response.status_code",
                i64::from(response.status().as_u16()),
            );
        }
        poll
    }
}

impl<S, B, ResBody> Service<http::Request<B>> for IngressService<S>
where
    S: Service<http::Request<B>, Response = http::Response<ResBody>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = ResponseFuture<S::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), S::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut request: http::Request<B>) -> Self::Future {
        // A fixed target that init always enables, as for gRPC: a span
        // disabled by RUST_LOG would silently drop the caller's trace.
        let span = tracing::info_span!(
            target: HTTP_TARGET,
            "http.server",
            otel.name = %request.method(),
            otel.kind = "server",
            http.request.method = %request.method(),
            http.route = tracing::field::Empty,
            http.response.status_code = tracing::field::Empty,
        );
        let headers = request.headers();
        let incoming = TraceCarrier::from_pairs(|key| {
            headers
                .get(key)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        });
        carrier::extract_ingress("http", &span, &incoming);
        let method = request.method().clone();
        request.extensions_mut().insert(ServerSpan {
            span: span.clone(),
            method,
        });
        let future = span.in_scope(|| self.inner.call(request));
        ResponseFuture {
            inner: Some(future),
            span,
        }
    }
}
