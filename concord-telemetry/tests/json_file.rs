use concord_telemetry::{init, Config, LocalOutput};

const CHILD: &str = "CONCORD_JSON_CHILD";

#[test]
fn writes_json_records_to_file_and_not_stdout() {
    let child_path =
        std::env::temp_dir().join(format!("concord-json-child-{}.log", std::process::id()));
    if std::env::var_os(CHILD).is_some() {
        let mut config = Config::new("json-child", "0");
        config.local = LocalOutput::JsonFile(
            std::env::var_os("CONCORD_JSON_CHILD_PATH")
                .map(std::path::PathBuf::from)
                .expect("child log path"),
        );
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
        .env("CONCORD_JSON_CHILD_PATH", &child_path)
        .output()
        .expect("run child test");
    assert!(output.status.success(), "child failed: {:?}", output.status);
    let child_content = std::fs::read_to_string(&child_path).expect("read child json");
    assert!(child_content.contains("\"action_id\":\"a1\""));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("sent"));
    assert!(!stdout.contains("action_id"));
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(child_path);
}
