// Always built: no `#![cfg]`. Runs with and without `otel`.
use concord_telemetry::carrier::{self, context};
use concord_telemetry::labels;
use concord_telemetry::propagation;
use concord_telemetry::{init, Config, LocalOutput, TraceCarrier};
use std::sync::{Mutex, Once};
use tracing::info_span;

const VALID_TP: &str = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";

static ENV: Mutex<()> = Mutex::new(());
static SETUP: Once = Once::new();

fn setup() {
    SETUP.call_once(|| {
        let _lock = ENV.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");
        let mut config = Config::new("label-core", "0");
        config.local = LocalOutput::Off;
        let guard = init(config).expect("init");
        // Kept for the whole run. The lint is for the off build, where Guard has no Drop.
        #[allow(clippy::forget_non_drop)]
        std::mem::forget(guard);
    });
}

#[test]
fn labels_travel_without_export() {
    setup();
    let span = info_span!("hop");
    let _entered = span.enter();
    context::add("action_id", "A1");
    let c = carrier::capture();
    assert_eq!(c.baggage.as_deref(), Some("action_id=A1"));
    assert_eq!(c.traceparent, None);
}

#[test]
fn parent_on_takes_baggage_without_export() {
    setup();
    let span = info_span!("hop");
    carrier::parent_on(
        &span,
        &TraceCarrier {
            traceparent: Some(VALID_TP.to_owned()),
            baggage: Some("action_id=A1,turn_id=T1".to_owned()),
            ..TraceCarrier::default()
        },
    );
    assert_eq!(labels::of(&span, "action_id").as_deref(), Some("A1"));
}

#[test]
fn an_entered_span_refuses_new_baggage() {
    setup();
    let span = info_span!("hop");
    let _entered = span.enter();
    context::add("action_id", "A1");
    carrier::parent_on(
        &span,
        &TraceCarrier {
            traceparent: Some(VALID_TP.to_owned()),
            baggage: Some("action_id=B".to_owned()),
            ..TraceCarrier::default()
        },
    );
    assert_eq!(labels::of(&span, "action_id").as_deref(), Some("A1"));
}

#[test]
fn set_parent_from_env_takes_baggage_without_export() {
    setup();
    let _lock = ENV.lock().unwrap_or_else(|e| e.into_inner());
    std::env::remove_var(propagation::TRACEPARENT_ENV);
    let previous = std::env::var(propagation::BAGGAGE_ENV).ok();
    std::env::set_var(propagation::BAGGAGE_ENV, "turn_id=T9");
    let span = info_span!("hop");
    propagation::set_parent_from_env(&span);
    match previous {
        Some(value) => std::env::set_var(propagation::BAGGAGE_ENV, value),
        None => std::env::remove_var(propagation::BAGGAGE_ENV),
    }
    assert_eq!(labels::of(&span, "turn_id").as_deref(), Some("T9"));
}

#[test]
fn the_library_never_writes_trace_env() {
    setup();
    let _lock = ENV.lock().unwrap_or_else(|e| e.into_inner());
    std::env::remove_var(propagation::TRACEPARENT_ENV);
    std::env::remove_var(propagation::BAGGAGE_ENV);
    let span = info_span!("hop");
    let _entered = span.enter();
    context::add("action_id", "A1");
    let _ = carrier::capture();
    propagation::inject(|_, _| {});
    let carrier = carrier::capture();
    let _ = concord_telemetry::propagation::inject_env(&carrier).count();
    propagation::set_parent_from_env(&span);
    carrier::parent_on_or_env(&span, None);
    assert!(std::env::var(propagation::TRACEPARENT_ENV).is_err());
    assert!(std::env::var(propagation::BAGGAGE_ENV).is_err());
}

#[test]
fn no_source_file_writes_process_env() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for entry in std::fs::read_dir(&dir).expect("src") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs")
            || path.file_name().and_then(|n| n.to_str()) == Some("testing.rs")
        {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("read");
        // Production code ends at the first test module: `#[cfg(test)]` or
        // `#[cfg(all(test, ...))]`.
        let production = text
            .split("#[cfg(test)]")
            .next()
            .and_then(|head| head.split("#[cfg(all(test").next())
            .unwrap_or("");
        assert!(!production.contains("set_var("), "{}", path.display());
        assert!(!production.contains("remove_var("), "{}", path.display());
    }
}

#[cfg(feature = "grpc")]
mod grpc_tests {
    use super::*;
    use concord_telemetry::grpc::{self, ServerLayer};
    use concord_telemetry::propagation::traceparent;
    use std::convert::Infallible;
    use tower::{service_fn, Layer, ServiceExt};
    use tracing::{Instrument, Span};

    type Request = http::Request<()>;

    fn request() -> Request {
        http::Request::builder()
            .uri("/pkg.Service/Method")
            .body(())
            .expect("request")
    }

    fn echo_baggage() -> impl tower::Service<
        Request,
        Response = Option<String>,
        Error = Infallible,
        Future = impl Send,
    > + Clone {
        service_fn(|request: Request| async move {
            Ok::<_, Infallible>(
                request
                    .headers()
                    .get("baggage")
                    .map(|v| v.to_str().expect("ascii").to_owned()),
            )
        })
    }

    #[tokio::test]
    async fn no_request_scope_sends_no_cause_label() {
        setup();
        let echo = service_fn(|request: Request| async move {
            Ok::<_, Infallible>(request.headers().get("baggage").is_some())
        });
        let sent = grpc::client(echo).oneshot(request()).await.expect("ok");
        assert!(!sent);

        let span = info_span!("left");
        span.in_scope(|| context::add("action_id", "A1"));
        let sent = grpc::client(echo).oneshot(request()).await.expect("ok");
        assert!(!sent);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn two_concurrent_requests_do_not_cross() {
        setup();
        let mut tasks = Vec::new();
        for i in 0..64 {
            tasks.push(tokio::spawn(async move {
                let span = info_span!("req", i);
                span.in_scope(|| {
                    context::add("action_id", format!("A{i}"));
                });
                let span2 = span.clone();
                let echoed = async {
                    for _ in 0..3 {
                        tokio::task::yield_now().await;
                    }
                    grpc::client(echo_baggage())
                        .oneshot(request())
                        .await
                        .expect("ok")
                }
                .instrument(span2)
                .await;
                (i, echoed)
            }));
        }
        for task in tasks {
            let (i, echoed) = task.await.expect("join");
            assert_eq!(echoed, Some(format!("action_id=A{i}")));
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn server_side_labels_do_not_cross() {
        setup();
        let handler = service_fn(|_: Request| async move {
            Ok::<_, Infallible>(labels::of(&Span::current(), "action_id"))
        });
        let mut tasks = Vec::new();
        for i in 0..64 {
            let svc = ServerLayer.layer(handler);
            tasks.push(tokio::spawn(async move {
                let mut req = request();
                req.headers_mut().insert(
                    "baggage",
                    http::HeaderValue::from_str(&format!("action_id=A{i}")).expect("hv"),
                );
                let got = svc.oneshot(req).await.expect("ok");
                (i, got)
            }));
        }
        for task in tasks {
            let (i, got) = task.await.expect("join");
            assert_eq!(got, Some(format!("A{i}")));
        }
    }

    // The env lock is held across the await on purpose: it serializes the
    // tests that touch the process env, and the body is a single task.
    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn the_library_never_writes_trace_env_grpc() {
        setup();
        let _lock = ENV.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var(propagation::TRACEPARENT_ENV);
        std::env::remove_var(propagation::BAGGAGE_ENV);
        let span = info_span!("hop");
        let _entered = span.enter();
        context::add("action_id", "A1");
        let _ = traceparent();
        let _ = grpc::client(echo_baggage()).oneshot(request()).await;
        assert!(std::env::var(propagation::TRACEPARENT_ENV).is_err());
        assert!(std::env::var(propagation::BAGGAGE_ENV).is_err());
    }
}
