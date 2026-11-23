//! The API with the `otel` feature off: same signatures, local output only.
#![cfg(not(feature = "otel"))]

#[allow(dead_code)]
#[path = "proto/fixtures.rs"]
mod fixtures;

use concord_telemetry::{
    carrier, error, error_anyhow, init, warn, Config, Event, Guard, LocalOutput,
};
use fixtures::{ActionStatusChanged, Status};
use std::path::PathBuf;
use std::sync::{Mutex, Once};
use std::time::{Duration, Instant};

static INIT: Once = Once::new();
static GUARD: Mutex<Option<Guard>> = Mutex::new(None);

/// Initialises once for the whole binary, writing JSON lines to a file, and
/// returns the file's path.
fn log_path() -> PathBuf {
    let path = std::env::temp_dir().join(format!("concord-off-build-{}.log", std::process::id()));
    INIT.call_once(|| {
        std::env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");
        let mut config = Config::new("off-build", "0");
        config.local = LocalOutput::JsonFile(path.clone());
        let guard = init(config).expect("off-build init");
        *GUARD.lock().unwrap() = Some(guard);
    });
    path
}

fn lines_with(needle: &str) -> Vec<String> {
    let path = log_path();
    let content = std::fs::read_to_string(path).expect("read log file");
    content
        .lines()
        .filter(|line| line.contains(needle))
        .map(str::to_owned)
        .collect()
}

#[derive(Debug, thiserror::Error, coded_error::CodedError)]
#[coded(namespace = "test.off")]
enum OffError {
    #[error("off failure")]
    #[coded(code = "failed", class = Logical)]
    Failed,
}

#[test]
fn init_text_writes_a_local_line() {
    log_path();
    tracing::info!(target: "application", "off-build-marker-line");
    let lines = lines_with("off-build-marker-line");
    assert_eq!(lines.len(), 1, "{lines:?}");
    let parsed: serde_json::Value = serde_json::from_str(&lines[0]).expect("json line");
    assert_eq!(parsed["fields"]["message"], "off-build-marker-line");
}

#[test]
fn guard_drop_is_noop() {
    log_path();
    let guard = GUARD.lock().unwrap().take();
    let started = Instant::now();
    #[allow(clippy::drop_non_drop)] // Guard has no Drop without `otel`
    drop(guard);
    assert!(started.elapsed() < Duration::from_millis(200));
}

#[test]
fn capture_is_empty() {
    log_path();
    assert_eq!(
        carrier::capture(),
        concord_telemetry::TraceCarrier::default()
    );
    let span = tracing::info_span!("off-span");
    carrier::parent_on(
        &span,
        &concord_telemetry::TraceCarrier {
            traceparent: Some("00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01".into()),
            ..Default::default()
        },
    );
    carrier::context::add("k", "v");
    assert_eq!(
        carrier::capture(),
        concord_telemetry::TraceCarrier::default()
    );
    assert!(concord_telemetry::propagation::traceparent().is_none());
}

#[test]
fn error_macro_compiles_and_writes_code() {
    log_path();
    let error = OffError::Failed;
    error!(&error, source = "off.source");
    let plain = anyhow::anyhow!("plain");
    error_anyhow!(&plain, source = "off.source.anyhow");
    assert_eq!(lines_with("off.source.anyhow").len(), 1);
    let lines = lines_with("test.off.failed");
    assert_eq!(lines.len(), 1, "{lines:?}");
    let parsed: serde_json::Value = serde_json::from_str(&lines[0]).expect("json line");
    assert_eq!(parsed["fields"]["error.code"], "test.off.failed");
    assert_eq!(parsed["fields"]["error.class"], "logical");
}

#[derive(Debug, thiserror::Error, coded_error::CodedError)]
#[coded(namespace = "test.off")]
enum OffWarning {
    #[error("off degraded")]
    #[coded(code = "degraded", class = Logical)]
    Degraded,
}

#[test]
fn warn_macro_compiles_and_writes_code() {
    log_path();
    let warning = OffWarning::Degraded;
    warn!(&warning, source = "off.source.warn");
    let lines = lines_with("off.source.warn");
    assert_eq!(lines.len(), 1, "{lines:?}");
    let parsed: serde_json::Value = serde_json::from_str(&lines[0]).expect("json line");
    assert_eq!(parsed["level"], "WARN");
    assert_eq!(parsed["fields"]["error.code"], "test.off.degraded");
}

#[test]
fn event_derive_works_off() {
    log_path();
    let event = ActionStatusChanged {
        source_record_id: "off-record-1".into(),
        action_id: "off-action".into(),
        old_status: Status::Ready as i32,
        new_status: Status::Running as i32,
        principal: None,
        at: None,
        took: None,
        attempt: 1,
        retried: false,
        labels: vec![],
        note: None,
    };
    event.emit();
    let lines = lines_with("off-record-1");
    assert_eq!(lines.len(), 1, "{lines:?}");
    let parsed: serde_json::Value = serde_json::from_str(&lines[0]).expect("json line");
    assert_eq!(
        parsed["fields"]["event.type"],
        "test.events.v1.ActionStatusChanged"
    );
}

#[cfg(feature = "grpc")]
#[test]
fn grpc_layer_off_runs() {
    use concord_telemetry::grpc::{client, ServerLayer};
    use std::convert::Infallible;
    use tower::{service_fn, Layer, ServiceExt};

    log_path();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime");
    runtime.block_on(async {
        let request = http::Request::builder()
            .uri("/pkg.Service/Method")
            .body(())
            .expect("request");
        let sent = client(service_fn(|request: http::Request<()>| async move {
            assert_eq!(
                tracing::Span::current().metadata().map(|m| m.name()),
                Some("grpc.client")
            );
            Ok::<_, Infallible>(request)
        }))
        .oneshot(request)
        .await
        .expect("infallible");
        assert!(sent.headers().get("traceparent").is_none());
        let served = ServerLayer
            .layer(service_fn(|_: http::Request<()>| async {
                Ok::<_, Infallible>(
                    tracing::Span::current()
                        .metadata()
                        .map(|m| m.name())
                        .unwrap_or("none"),
                )
            }))
            .oneshot(sent)
            .await
            .expect("infallible");
        assert_eq!(served, "grpc.server");
    });
}
