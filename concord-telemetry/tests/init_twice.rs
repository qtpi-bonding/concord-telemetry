use concord_telemetry::{init, Config, InitError};
use std::net::TcpListener;

#[test]
fn second_initialization_is_refused() {
    std::env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");
    assert!(!tracing::dispatcher::has_been_set());
    let guard = init(Config::new("twice", "0")).expect("first init");
    let listener = TcpListener::bind("127.0.0.1:0").expect("reserve refused port");
    let port = listener.local_addr().expect("reserved address").port();
    drop(listener);
    std::env::set_var(
        "OTEL_EXPORTER_OTLP_ENDPOINT",
        format!("http://127.0.0.1:{port}"),
    );
    // No exporter is built: init.rs checks the already-initialized state at step 0.
    let result = init(Config::new("twice", "0"));
    match result {
        Err(InitError::AlreadyInitialized) => {}
        Err(other) => panic!("unexpected initialization error: {other}"),
        Ok(_) => panic!("second initialization unexpectedly succeeded"),
    }
    drop(guard);
}
