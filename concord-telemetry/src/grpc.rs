//! Trace context for gRPC, as tower layers over `http::Request`, so tonic
//! stays out of this crate.
//!
//! Server: `Server::builder().layer(concord_telemetry::grpc::ServerLayer)`.
//! Client: `FooClient::new(concord_telemetry::grpc::client(channel))`.
//!
//! The server span covers the handler's future up to its response. Items a
//! streaming response produces later are polled outside it, so their records
//! carry no trace id, and `causes::incoming()` is empty there too.
//!
//! With the `grpc-status` feature, [`reject`], [`reject_anyhow`] and
//! [`reject_anyhow_redacted`] turn an error into the `tonic::Status` a handler
//! returns and record it, at the handler's own file and line.

use crate::{carrier, causes, init::GRPC_TARGET, TraceCarrier};
use std::task::{Context, Poll};
use tower_layer::Layer;
use tower_service::Service;
use tracing::instrument::{Instrument, Instrumented};

/// Joins each request's trace and baggage, if it carries them, in a
/// `grpc.server` span. A request without a trace is not an error: the server
/// is an ingress.
#[derive(Clone, Copy, Debug, Default)]
pub struct ServerLayer;

/// Sends the trace and baggage of a `grpc.client` span with each request.
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
    type Future = Instrumented<causes::Scoped<S::Future>>;

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
        let incoming = TraceCarrier::from_pairs(|key| {
            headers
                .get(key)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        });
        carrier::extract_ingress("grpc", &span, &incoming);
        let causes = span.in_scope(causes::capture_incoming);
        let future = causes::sync_scope(causes.clone(), || {
            span.in_scope(|| self.inner.call(request))
        });
        causes::scope(causes, future).instrument(span)
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
            crate::propagation::inject(|key, value| {
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

/// The fingerprint source and module of the records `reject*` make: the
/// caller's module is not known to a `#[track_caller]` function, so they all
/// share this one, and the code and class tell them apart.
#[cfg(feature = "grpc-status")]
const REJECT_SOURCE: &str = module_path!();

/// Maps `err` to a `tonic::Status` by its class and records it with
/// `rpc.method = method`. The record's `code.file.path` and `code.line.number`
/// are the call site of `reject`, not this file.
#[cfg(feature = "grpc-status")]
#[track_caller]
pub fn reject<E: coded_error::CodedError>(method: &'static str, err: E) -> tonic::Status {
    let location = std::panic::Location::caller();
    crate::emit_error_at(
        location.file(),
        location.line(),
        REJECT_SOURCE,
        &err,
        &[("rpc.method", crate::AttrValue::Str(method.to_owned()))],
    );
    coded_error::grpc::to_status(&err, REJECT_SOURCE)
}

/// `reject` for an `anyhow::Error`: the code and class come from its coded
/// cause, `unknown` when it has none. The status message is the full chain.
#[cfg(feature = "grpc-status")]
#[track_caller]
pub fn reject_anyhow(method: &'static str, err: &anyhow::Error) -> tonic::Status {
    record_anyhow(method, err, std::panic::Location::caller());
    coded_error::grpc::to_status_anyhow(err, REJECT_SOURCE)
}

/// `reject_anyhow` for a caller the message must not reach: the status message
/// is `fixed` unless the error is a logical typed one, whose own text is safe
/// to show. The record keeps the whole chain.
#[cfg(feature = "grpc-status")]
#[track_caller]
pub fn reject_anyhow_redacted(
    method: &'static str,
    err: &anyhow::Error,
    fixed: &str,
) -> tonic::Status {
    record_anyhow(method, err, std::panic::Location::caller());
    coded_error::grpc::to_status_anyhow_redacted(err, REJECT_SOURCE, fixed)
}

#[cfg(feature = "grpc-status")]
fn record_anyhow(
    method: &'static str,
    err: &anyhow::Error,
    location: &'static std::panic::Location<'static>,
) {
    crate::error_at_anyhow!(
        location.file(),
        location.line(),
        REJECT_SOURCE,
        err,
        rpc.method = method
    );
}
