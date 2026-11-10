//! The panic hook waits no longer than `Config::flush_deadline`.

use concord_telemetry::{init, Config, LocalOutput};
use std::net::TcpListener;
use std::panic;
use std::thread;
use std::time::{Duration, Instant};

#[test]
fn the_panic_hook_flush_is_bounded_by_the_configured_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    thread::spawn(move || {
        for stream in listener.incoming() {
            if stream.is_err() {
                break;
            }
            thread::sleep(Duration::from_secs(10));
        }
    });
    std::env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", format!("http://{address}"));
    panic::set_hook(Box::new(|_| {}));
    let mut config = Config::new("panic-deadline", "0");
    config.local = LocalOutput::Off;
    config.flush_deadline = Duration::from_millis(300);
    let _guard = init(config).expect("init");
    let started = Instant::now();
    assert!(panic::catch_unwind(|| panic!("boom")).is_err());
    let waited = started.elapsed();
    assert!(waited >= Duration::from_millis(250), "{waited:?}");
    assert!(waited < Duration::from_millis(1200), "{waited:?}");
}
