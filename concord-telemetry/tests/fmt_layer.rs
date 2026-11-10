//! tracing-subscriber's fmt layer formats a span's fields, running user `Debug`,
//! while it holds the span's extensions write lock. Labels must not take any
//! lock that such a `Debug` could wait on.

use concord_telemetry::{context, init, Config, LocalOutput};
use std::sync::mpsc;
use std::time::Duration;
use tracing::info_span;

#[tracing::instrument]
fn fresh() {
    context::add("turn_id", "inner");
}

struct Enters;
impl std::fmt::Debug for Enters {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fresh();
        f.write_str("enters")
    }
}

#[test]
fn a_debug_that_enters_a_fresh_instrumented_span_under_the_fmt_layer_does_not_deadlock() {
    std::env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", "http://127.0.0.1:9");
    let mut config = Config::new("fmt-layer", "0");
    config.local = LocalOutput::Text;
    config.flush_deadline = Duration::from_millis(200);
    let _guard = init(config).expect("init");
    let (done, finished) = mpsc::channel();
    std::thread::spawn(move || {
        let parent = info_span!("fmt-parent");
        parent.in_scope(|| {
            context::add("action_id", "A");
            let child = info_span!("fmt-child", detail = ?Enters);
            child.in_scope(|| context::add("turn_id", "outer"));
            child.record("detail", tracing::field::debug(Enters));
        });
        let _ = done.send(());
    });
    finished
        .recv_timeout(Duration::from_secs(20))
        .expect("a Debug that enters a span under the fmt layer must not deadlock");
}
