// Its own binary: it calls `init`, which sets the global subscriber.
use concord_telemetry::{context, init, Config, LocalOutput};

#[test]
fn json_line_without_exporter_has_no_trace_fields() {
    std::env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");
    let path = std::env::temp_dir().join(format!("concord-no-trace-{}.log", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let mut config = Config::new("json-no-trace", "0");
    config.local = LocalOutput::JsonFile(path.clone());
    let guard = init(config).expect("init");
    let span = tracing::info_span!("work");
    span.in_scope(|| {
        context::add("action_id", "A1");
        tracing::info!(marker = "m", "inside");
    });
    drop(span);
    drop(guard);

    let content = std::fs::read_to_string(&path).expect("read json");
    let line = content
        .lines()
        .find(|line| line.contains("\"inside\""))
        .expect("the inside line");
    let value: serde_json::Value = serde_json::from_str(line).expect("valid json");
    for field in ["trace_id", "span_id", "baggage"] {
        assert!(value.get(field).is_none(), "{field} present: {line}");
    }
    assert_eq!(value["fields"]["marker"], "m");
    let _ = std::fs::remove_file(path);
}
