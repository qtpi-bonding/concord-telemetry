use tracing::info;

fn example(error: &str, action_id: &str) {
    info!(%error, "x");
    info!(?action_id, "x");
}