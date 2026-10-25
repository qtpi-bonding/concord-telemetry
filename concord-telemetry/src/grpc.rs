//! Trace context for gRPC, as tower layers over `http::Request`, so tonic
//! stays out of this crate.
//!
//! Server: `Server::builder().layer(concord_telemetry::grpc::ServerLayer)`.
//! Client: `FooClient::new(concord_telemetry::grpc::client(channel))`.
//!
//! The server span covers the handler's future up to its response. Items a
//! streaming response produces later are polled outside it, so their records
//! carry no trace id.

use crate::{init::GRPC_TARGET, propagation};
use std::task::{Context, Poll};
use tower_layer::Layer;
use tower_service::Service;
use tracing::instrument::{Instrument, Instrumented};

/// Joins each request's trace, if it carries one, in a `grpc.server` span.
#[derive(Clone, Copy, Debug, Default)]
pub struct ServerLayer;

/// Sends the trace of a `grpc.client` span with each request.
#[derive(Clone, Copy, Debug, Default)]
pub struct ClientLayer;

#[derive(Clone, Debug)]
pub struct ServerService<S> {
    inner: S,
}

#[derive(Clone, Debug)]
pub struct ClientService<S> {
    inner: S,
}

/// Wraps a client channel so every call sends its trace.
pub fn client<S>(channel: S) -> ClientService<S> {
    ClientLayer.layer(channel)
}

impl<S> Layer<S> for ServerLayer {
    type Service = ServerService<S>;

    fn layer(&self, inner: S) -> ServerService<S> {
        ServerService { inner }
    }
}

impl<S> Layer<S> for ClientLayer {
    type Service = ClientService<S>;

    fn layer(&self, inner: S) -> ClientService<S> {
        ClientService { inner }
    }
}

impl<S, B> Service<http::Request<B>> for ServerService<S>
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
            target: GRPC_TARGET,
            "grpc.server",
            otel.name = %request.uri().path(),
            otel.kind = "server",
            rpc.system = "grpc",
        );
        let headers = request.headers();
        propagation::set_parent(&span, |key| {
            headers
                .get(key)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        });
        let future = span.in_scope(|| self.inner.call(request));
        future.instrument(span)
    }
}

impl<S, B> Service<http::Request<B>> for ClientService<S>
where
    S: Service<http::Request<B>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Instrumented<S::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), S::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut request: http::Request<B>) -> Self::Future {
        let span = tracing::info_span!(
            target: GRPC_TARGET,
            "grpc.client",
            otel.name = %request.uri().path(),
            otel.kind = "client",
            rpc.system = "grpc",
        );
        let future = span.in_scope(|| {
            let headers = request.headers_mut();
            propagation::inject(|key, value| {
                if let (Ok(name), Ok(value)) = (
                    http::HeaderName::from_bytes(key.as_bytes()),
                    http::HeaderValue::from_str(&value),
                ) {
                    headers.insert(name, value);
                }
            });
            self.inner.call(request)
        });
        future.instrument(span)
    }
}
