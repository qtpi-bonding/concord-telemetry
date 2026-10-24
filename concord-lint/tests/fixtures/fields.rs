use tracing::info;

fn example(action_id: &str, method: &str) {
    info!(action_id, "x");
    info!("rpc.method" = %method, "x");
    info!(format_args!("x"));
}