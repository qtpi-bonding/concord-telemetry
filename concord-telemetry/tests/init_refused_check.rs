use concord_telemetry::{init, Config, LocalOutput};

/// sqlx asks `enabled!` for every query's log level and usually gets "no".
/// Under per-layer filtering that refusal dropped the next record on the
/// thread, so a service lost most of its own logs.
#[test]
fn a_refused_enabled_check_does_not_drop_the_next_record() {
    std::env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");
    std::env::set_var("RUST_LOG", "info");
    let file =
        std::env::temp_dir().join(format!("concord-refused-check-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&file);
    let mut config = Config::new("refused-check", "0");
    config.local = LocalOutput::JsonFile(file.clone());
    let guard = init(config).expect("init");
    assert!(!tracing::enabled!(target: "sqlx::query", tracing::Level::DEBUG));
    tracing::info!(target: "application", "after a refused check");
    drop(guard);
    let output = std::fs::read_to_string(&file).expect("json output");
    assert!(
        output.contains("after a refused check"),
        "record dropped: {output:?}"
    );
    let _ = std::fs::remove_file(file);
}
