use concord_telemetry::{init, Config, LocalOutput};
use std::net::TcpListener;
use std::panic;
use std::time::{Duration, Instant};

#[test]
fn refused_endpoint_does_not_block_shutdown_or_panic_hook() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("reserve port");
    let port = listener.local_addr().expect("address").port();
    drop(listener);
    std::env::set_var(
        "OTEL_EXPORTER_OTLP_ENDPOINT",
        format!("http://127.0.0.1:{port}"),
    );
    let mut config = Config::new("refused", "0");
    config.local = LocalOutput::Off;
    let guard = init(config).expect("init");
    tracing::info!("record");
    let started = Instant::now();
    let result = panic::catch_unwind(|| panic!("expected panic"));
    assert!(result.is_err());
    assert!(started.elapsed() < Duration::from_secs(3));
    let started = Instant::now();
    drop(guard);
    assert!(started.elapsed() < Duration::from_secs(3));
}
