//! The off API called in a build where `otel` is on (feature unification):
//! with no OTLP endpoint it behaves as the off build does.
#![cfg(feature = "otel")]

use concord_telemetry::{carrier, init, Config, LocalOutput};
use std::time::{Duration, Instant};

fn thread_count() -> usize {
    std::fs::read_dir("/proc/self/task")
        .map(|dir| dir.count())
        .unwrap_or(0)
}

#[test]
fn unified_off_api_is_inert_without_endpoint() {
    std::env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");
    let threads_before = thread_count();
    let path = std::env::temp_dir().join(format!("concord-unified-{}.log", std::process::id()));
    let mut config = Config::new("unified", "0");
    config.local = LocalOutput::JsonFile(path.clone());
    let guard = init(config).expect("init");
    assert!(
        thread_count() <= threads_before,
        "no exporter thread without an endpoint"
    );
    tracing::info!(target: "application", "unified-line");
    assert_eq!(
        carrier::capture(),
        concord_telemetry::TraceCarrier::default()
    );
    let started = Instant::now();
    drop(guard);
    assert!(started.elapsed() < Duration::from_millis(200));
    let content = std::fs::read_to_string(&path).expect("read log");
    assert!(content.contains("unified-line"));
    assert!(!content.contains("trace_id"));
    let _ = std::fs::remove_file(path);
}
