use concord_telemetry::{error, error_anyhow};

fn example(e: &str, method: &str) {
    concord_telemetry::error!(&e, widget = 1);
    concord_telemetry::error!(&e, rpc.method = %method);
    error!(&e, source = X::NAME);
    error_anyhow!(&e, source = X::NAME, rpc.method = %method);
    error!(&e, source = X::NAME);
}