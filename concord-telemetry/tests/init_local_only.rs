use concord_telemetry::{init, Config, EVENT_TARGET};
use std::time::Instant;

#[test]
fn initializes_without_exporter_and_drops_quickly() {
    std::env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");
    let guard = init(Config::new("t", "0")).expect("local initialization");
    tracing::info!(target: "application", "local record");
    tracing::event!(name: "test.event", target: EVENT_TARGET, tracing::Level::INFO, answer = 42);
    let started = Instant::now();
    drop(guard);
    assert!(
        started.elapsed().as_secs() < 1,
        "local-only drop took too long"
    );
}
