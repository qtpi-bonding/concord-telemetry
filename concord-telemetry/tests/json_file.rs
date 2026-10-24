use concord_telemetry::{init, Config, LocalOutput};

const CHILD: &str = "CONCORD_JSON_CHILD";

#[test]
fn writes_json_records_to_file_and_not_stdout() {
    if std::env::var_os(CHILD).is_some() {
        let mut config = Config::new("json-child", "0");
        config.local = LocalOutput::JsonFile(std::env::temp_dir().join("concord-json-child.log"));
        let guard = init(config).expect("child init");
        tracing::info!(action_id = "a1", "sent");
        drop(guard);
        return;
    }

    std::env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");
    let path = std::env::temp_dir().join(format!("concord-json-{}.log", std::process::id()));
    let mut config = Config::new("json", "0");
    config.local = LocalOutput::JsonFile(path.clone());
    let guard = init(config).expect("init");
    tracing::info!(action_id = "a1", "sent");
    drop(guard);
    let content = std::fs::read_to_string(&path).expect("read json");
    assert!(content
        .lines()
        .any(|line| { line.contains("\"fields\":{\"message\":\"sent\",\"action_id\":\"a1\"}") }));

    let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
        .arg("--exact")
        .arg("writes_json_records_to_file_and_not_stdout")
        .arg("--nocapture")
        .env(CHILD, "1")
        .output()
        .expect("run child test");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout
        .lines()
        .any(|line| { line.contains("\"message\":\"sent\"") || line.contains("action_id") }));
    let _ = std::fs::remove_file(path);
}
