// Always built: no `#![cfg]`. Runs with and without `otel`.
use concord_telemetry::carrier::{self, context};
use concord_telemetry::causes::{self, Causes};
use concord_telemetry::labels;
use concord_telemetry::propagation;
use concord_telemetry::{init, Cause, Config, LocalOutput, TraceCarrier};
use std::sync::{Mutex, Once};
use tracing::info_span;

static ENV: Mutex<()> = Mutex::new(());
static SETUP: Once = Once::new();

fn setup() {
    SETUP.call_once(|| {
        let _lock = ENV.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");
        let mut config = Config::new("causes", "0");
        config.local = LocalOutput::Off;
        let guard = init(config).expect("init");
        // Kept for the whole run. The lint is for the off build, where Guard has no Drop.
        #[allow(clippy::forget_non_drop)]
        std::mem::forget(guard);
    });
}

fn cause(kind: &str, id: &str) -> Cause {
    Cause::new(kind, id)
}

/// A span parented on a carrier whose baggage is `baggage`.
fn entry(baggage: &str) -> tracing::Span {
    let span = info_span!("entry");
    carrier::parent_on(
        &span,
        &TraceCarrier {
            baggage: Some(baggage.to_owned()),
            ..TraceCarrier::default()
        },
    );
    span
}

#[test]
fn capture_is_frozen_at_the_entry_point() {
    setup();
    let span = entry("action_id=A1");
    let _entered = span.enter();
    context::add("action_id", "OWN");
    assert_eq!(
        causes::capture_incoming().to_vec(),
        vec![cause("action", "A1")]
    );
    assert_eq!(labels::of(&span, "action_id").as_deref(), Some("OWN"));
}

#[test]
fn a_child_span_sees_its_entry_snapshot() {
    setup();
    let span = entry("action_id=A1");
    let child = info_span!(parent: &span, "child");
    let _entered = child.enter();
    assert_eq!(
        causes::capture_incoming().to_vec(),
        vec![cause("action", "A1")]
    );
}

#[test]
fn a_nested_entry_with_no_baggage_is_a_boundary() {
    setup();
    let outer = entry("action_id=A1");
    let _outer = outer.enter();
    let inner = info_span!("inner");
    carrier::parent_on(&inner, &TraceCarrier::default());
    let _inner = inner.enter();
    assert!(causes::capture_incoming().is_empty());
}

#[test]
fn no_entry_point_means_no_causes() {
    setup();
    let span = info_span!("plain");
    let _entered = span.enter();
    context::add("action_id", "A1");
    assert!(causes::capture_incoming().is_empty());
}

#[tokio::test]
async fn scope_holds_the_snapshot_across_awaits() {
    setup();
    let c = Causes::new([cause("turn", "T")]);
    let seen = causes::scope(c.clone(), async {
        tokio::task::yield_now().await;
        causes::incoming()
    })
    .await;
    assert_eq!(seen, c);
}

#[tokio::test]
async fn scope_does_not_cross_spawn() {
    setup();
    let c = Causes::new([cause("turn", "T")]);
    let seen = causes::scope(c, async {
        tokio::spawn(async { causes::incoming() })
            .await
            .expect("join")
    })
    .await;
    assert!(seen.is_empty());
}

#[tokio::test]
async fn scope_with_extends_only_the_inner_call() {
    setup();
    let outer = Causes::new([cause("turn", "T")]);
    let (inside, after) = causes::scope(outer.clone(), async {
        let inside =
            causes::scope_with([cause("message", "M")], async { causes::incoming() }).await;
        (inside, causes::incoming())
    })
    .await;
    assert_eq!(
        inside.to_vec(),
        vec![cause("turn", "T"), cause("message", "M")]
    );
    assert_eq!(after.to_vec(), vec![cause("turn", "T")]);
}

#[test]
fn concurrent_scopes_do_not_cross() {
    setup();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .build()
        .expect("runtime");
    runtime.block_on(async {
        let tasks: Vec<_> = (0..64)
            .map(|i| {
                let id = format!("A{i}");
                let own = Causes::new([cause("action", &id)]);
                tokio::spawn(causes::scope(own.clone(), async move {
                    for _ in 0..3 {
                        tokio::task::yield_now().await;
                    }
                    (own, causes::incoming())
                }))
            })
            .collect();
        for task in tasks {
            let (expected, seen) = task.await.expect("join");
            assert_eq!(seen, expected);
        }
    });
}

#[test]
fn set_parent_from_env_marks_the_cli_span() {
    setup();
    let _lock = ENV.lock().unwrap_or_else(|e| e.into_inner());
    std::env::remove_var(propagation::TRACEPARENT_ENV);
    let previous = std::env::var(propagation::BAGGAGE_ENV).ok();
    std::env::set_var(propagation::BAGGAGE_ENV, "turn_id=T1");
    let span = info_span!("cli");
    propagation::set_parent_from_env(&span);
    match previous {
        Some(value) => std::env::set_var(propagation::BAGGAGE_ENV, value),
        None => std::env::remove_var(propagation::BAGGAGE_ENV),
    }
    assert_eq!(
        span.in_scope(causes::capture_incoming).to_vec(),
        vec![cause("turn", "T1")]
    );
}

#[cfg(feature = "grpc")]
mod grpc {
    use super::*;
    use concord_telemetry::grpc::ServerLayer;
    use std::convert::Infallible;
    use tower::{service_fn, Layer, ServiceExt};
    use tracing::Instrument;

    type Request = http::Request<()>;

    fn request(baggage: Option<&str>) -> Request {
        let mut builder = http::Request::builder().uri("/pkg.Service/Method");
        if let Some(value) = baggage {
            builder = builder.header("baggage", value);
        }
        builder.body(()).expect("request")
    }

    /// Serves `request`; the handler writes its own `action_id`, yields, and returns its incoming causes.
    async fn served(request: Request) -> Causes {
        let handler = service_fn(|_: Request| async {
            context::add("action_id", "OWN");
            tokio::task::yield_now().await;
            Ok::<_, Infallible>(causes::incoming())
        });
        ServerLayer
            .layer(handler)
            .oneshot(request)
            .instrument(tracing::Span::none())
            .await
            .expect("infallible")
    }

    #[tokio::test]
    async fn grpc_server_scopes_the_handler() {
        setup();
        let seen = served(request(Some("action_id=A1,instance_id=I1"))).await;
        assert_eq!(
            seen.to_vec(),
            vec![cause("action", "A1"), cause("instance", "I1")]
        );
    }

    #[tokio::test]
    async fn grpc_server_without_baggage_has_no_causes() {
        setup();
        assert!(served(request(None)).await.is_empty());
    }
}

#[cfg(feature = "http-server")]
mod ingress {
    use super::*;
    use concord_telemetry::http_server::IngressLayer;
    use std::convert::Infallible;
    use tower::{service_fn, Layer, ServiceExt};

    #[tokio::test]
    async fn http_ingress_scopes_the_handler() {
        setup();
        let request = http::Request::builder()
            .uri("/items")
            .header("baggage", "action_id=A1,instance_id=I1")
            .body(())
            .expect("request");
        let handler = service_fn(|_: http::Request<()>| async {
            context::add("action_id", "OWN");
            tokio::task::yield_now().await;
            Ok::<_, Infallible>(http::Response::new(causes::incoming()))
        });
        let response = IngressLayer
            .layer(handler)
            .oneshot(request)
            .await
            .expect("infallible");
        assert_eq!(
            response.into_body().to_vec(),
            vec![cause("action", "A1"), cause("instance", "I1")]
        );
    }
}
