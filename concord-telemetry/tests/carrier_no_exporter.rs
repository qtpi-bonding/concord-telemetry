// Its own binary: it calls `init`, which sets the global subscriber.
use concord_telemetry::{carrier, init, Config, LocalOutput, TraceCarrier, TRACE_MISSING_CODE};

#[test]
fn no_exporter_means_no_trace_missing() {
    std::env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");
    let path = std::env::temp_dir().join(format!("concord-no-exporter-{}.log", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let mut config = Config::new("no-exporter", "0");
    config.local = LocalOutput::JsonFile(path.clone());
    let guard = init(config).expect("init");

    let span = tracing::info_span!("hop");
    carrier::extract_required("test.hop", &span, &TraceCarrier::default());
    carrier::extract_ingress("test.ingress", &span, &TraceCarrier::default());
    span.in_scope(|| {
        concord_telemetry::context::add("action_id", "A1");
        assert!(carrier::capture().is_empty());
    });
    tracing::info!(action_id = "a1", "local still works");
    drop(guard);

    let content = std::fs::read_to_string(&path).expect("read json");
    assert!(content.contains("local still works"), "{content}");
    assert!(!content.contains(TRACE_MISSING_CODE), "{content}");
    let _ = std::fs::remove_file(path);
}
