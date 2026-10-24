use tracing::debug;

fn example(s: &str) {
    debug!(stdout = %s, "ran");
}