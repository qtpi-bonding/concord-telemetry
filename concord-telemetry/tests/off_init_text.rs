//! Own binary, `otel` off: `init` with `LocalOutput::Text` writes a local
//! line. Text goes to stderr, so a child process is re-run and its stderr read.
#![cfg(not(feature = "otel"))]
use concord_telemetry::{init, Config, LocalOutput};

const CHILD: &str = "CONCORD_OFF_TEXT_CHILD";

#[test]
fn text_writes_a_local_line() {
    if std::env::var_os(CHILD).is_some() {
        let mut config = Config::new("off-text", "0");
        config.local = LocalOutput::Text;
        let guard = init(config).expect("child init");
        tracing::info!(target: "application", "off-text-marker");
        drop(guard);
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
        .args(["--exact", "text_writes_a_local_line", "--nocapture"])
        .env(CHILD, "1")
        .env_remove("OTEL_EXPORTER_OTLP_ENDPOINT")
        .output()
        .expect("run child test");
    assert!(output.status.success(), "child failed: {:?}", output.status);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let lines: Vec<_> = stderr
        .lines()
        .filter(|l| l.contains("off-text-marker"))
        .collect();
    assert_eq!(lines.len(), 1, "{stderr}");
}
