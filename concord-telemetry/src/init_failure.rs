//! Single-line JSON report for failures during telemetry initialisation.

use std::error::Error;

/// Error code carried by every init-failure line.
const CODE: &str = "telemetry_init_failed";

/// Builds the one-line JSON record describing an init failure.
///
/// Keys appear in a fixed order: `level`, `code`, `service`, `message`.
/// `message` is the error's `Display` followed by each `source()` joined
/// with `": "`. The returned string has no trailing newline.
pub fn init_failure_line(service: &str, err: &dyn Error) -> String {
    let mut message = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    let service_json = serde_json::to_string(service).unwrap_or_else(|_| "\"\"".to_owned());
    let message_json = serde_json::to_string(&message).unwrap_or_else(|_| "\"\"".to_owned());
    format!(
        "{{\"level\":\"error\",\"code\":\"{CODE}\",\"service\":{service_json},\"message\":{message_json}}}"
    )
}

/// Writes the init-failure line for `service` and `err` to stderr.
pub fn report_init_failure(service: &str, err: &dyn Error) {
    let line = init_failure_line(service, err);
    eprintln!("{line}");
}
