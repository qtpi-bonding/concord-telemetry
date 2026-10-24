use tracing::{debug, info};

#[cfg(test)]
mod t {
    fn f() {
        info!(widget = 1, "ok");
        info!("bad {x}");
        debug!(stdout = %s, "ran");
    }
}