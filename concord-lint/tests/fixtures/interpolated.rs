use tracing::{info, warn};

fn example(id: &str, y: usize) {
    info!("sent {id}");
    warn!("x {}", y);
    tracing::error!(%error, "failed: {id}");
}