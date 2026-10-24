use concord_telemetry::{init, Config, InitError};

#[test]
fn second_initialization_is_refused() {
    std::env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");
    assert!(!tracing::dispatcher::has_been_set());
    let guard = init(Config::new("twice", "0")).expect("first init");
    let result = init(Config::new("twice", "0"));
    match result {
        Err(InitError::AlreadyInitialized) => {}
        Err(other) => panic!("unexpected initialization error: {other}"),
        Ok(_) => panic!("second initialization unexpectedly succeeded"),
    }
    drop(guard);
}
