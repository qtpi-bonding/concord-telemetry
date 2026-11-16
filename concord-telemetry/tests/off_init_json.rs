//! Own binary, `otel` off: `init` with `LocalOutput::Json` writes a JSON
//! line, and with `LocalOutput::Off` writes none. Both go to stderr, so a
//! child process is re-run for each and its stderr read.
#![cfg(not(feature = "otel"))]
use concord_telemetry::{init, Config, LocalOutput};

const CHILD: &str = "CONCORD_OFF_JSON_CHILD";

fn child(mode: &str, test: &str) -> String {
    let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
        .args(["--exact", test, "--nocapture"])
        .env(CHILD, mode)
        .env_remove("OTEL_EXPORTER_OTLP_ENDPOINT")
        .output()
        .expect("run child test");
    assert!(output.status.success(), "child failed: {:?}", output.status);
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn run_child(local: LocalOutput) {
    let mut config = Config::new("off-json", "0");
    config.local = local;
    let guard = init(config).expect("child init");
    tracing::info!(target: "application", "off-json-marker");
    #[allow(clippy::drop_non_drop)] // Guard has no Drop without `otel`
    drop(guard);
}

#[test]
fn json_writes_a_local_line() {
    if std::env::var(CHILD).as_deref() == Ok("json") {
        return run_child(LocalOutput::Json);
    }
    let stderr = child("json", "json_writes_a_local_line");
    let lines: Vec<_> = stderr
        .lines()
        .filter(|l| l.contains("off-json-marker"))
        .collect();
    assert_eq!(lines.len(), 1, "{stderr}");
    let parsed: serde_json::Value = serde_json::from_str(lines[0]).expect("json line");
    assert_eq!(parsed["fields"]["message"], "off-json-marker");
}

#[test]
fn off_writes_no_local_line() {
    if std::env::var(CHILD).as_deref() == Ok("off") {
        return run_child(LocalOutput::Off);
    }
    let stderr = child("off", "off_writes_no_local_line");
    assert!(!stderr.contains("off-json-marker"), "{stderr}");
}
