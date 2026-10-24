use concord_telemetry::{init, Config, LocalOutput};
use std::net::TcpListener;
use std::panic;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

#[test]
fn panic_is_recorded_without_display_text_and_chains_hook() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    thread::spawn(move || {
        for stream in listener.incoming() {
            if stream.is_err() {
                break;
            }
            thread::sleep(Duration::from_secs(4));
        }
    });
    let path = std::env::temp_dir().join(format!("concord-panic-{}.json", std::process::id()));
    std::env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", format!("http://{address}"));
    let prior_called = Arc::new(AtomicBool::new(false));
    let prior_flag = Arc::clone(&prior_called);
    panic::set_hook(Box::new(move |_| {
        prior_flag.store(true, Ordering::SeqCst);
    }));
    let mut config = Config::new("panic", "0");
    config.local = LocalOutput::JsonFile(path.clone());
    let guard = init(config).expect("init");
    let started = Instant::now();
    let result = panic::catch_unwind(|| panic!("boom"));
    assert!(result.is_err());
    assert!(started.elapsed() < Duration::from_secs_f32(2.5));
    assert!(prior_called.load(Ordering::SeqCst));

    let thread_result = thread::spawn(|| panic!("worker boom")).join();
    assert!(thread_result.is_err());
    drop(guard);
    let content = std::fs::read_to_string(&path).expect("panic JSON");
    let record = content
        .lines()
        .find(|line| line.contains("concord.process.panic"))
        .expect("panic record");
    assert!(record.contains("\"error.code\":\"concord.process.panic\""));
    assert!(record.contains("\"error.class\":\"unknown\""));
    assert!(record.contains("\"error.source\":"));
    assert!(record.contains("\"error.fingerprint\":"));
    assert!(record.contains("panic_hook.rs"));
    assert!(!record.contains("boom"));
    let _ = std::fs::remove_file(path);
}
