use concord_telemetry::{init, Config, LocalOutput, EVENT_TARGET};

#[test]
fn events_remain_enabled_when_rust_log_is_error() {
    std::env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");
    std::env::set_var("RUST_LOG", "error");
    let file = std::env::temp_dir().join(format!("concord-filter-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&file);
    let mut config = Config::new("filter", "0");
    config.local = LocalOutput::JsonFile(file.clone());
    let guard = init(config).expect("init");
    tracing::info!(target: "application", "hidden");
    tracing::event!(name: "visible", target: EVENT_TARGET, tracing::Level::INFO, marker = "yes");
    drop(guard);
    let output = std::fs::read_to_string(&file).expect("json output");
    assert!(output.contains("\"marker\":\"yes\""));
    assert!(!output.contains("hidden"));
    let _ = std::fs::remove_file(file);
}
