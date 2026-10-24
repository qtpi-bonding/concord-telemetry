use concord_telemetry::{init, Config, LocalOutput};
use std::net::TcpListener;
use std::panic;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

#[test]
fn panic_is_recorded_without_display_text_and_chains_hook() {
    if std::env::var_os("CONCORD_PANIC_CHILD").is_some() {
        panic::set_hook(Box::new(|_| eprintln!("concord prior panic hook marker")));
        assert!(panic::catch_unwind(|| panic!("child panic")).is_err());
        return;
    }
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let accepted = Arc::new(AtomicBool::new(false));
    let accepted_flag = Arc::clone(&accepted);
    thread::spawn(move || {
        for stream in listener.incoming() {
            if stream.is_err() {
                break;
            }
            accepted_flag.store(true, Ordering::SeqCst);
            thread::sleep(Duration::from_secs(4));
        }
    });
    let path = std::env::temp_dir().join(format!("concord-panic-{}.json", std::process::id()));
    std::env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", format!("http://{address}"));
    let prior_called = Arc::new(AtomicBool::new(false));
    let prior_flag = Arc::clone(&prior_called);
    panic::set_hook(Box::new(move |_| {
        prior_flag.store(true, Ordering::SeqCst);
        eprintln!("concord prior panic hook marker");
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
    assert!(accepted.load(Ordering::SeqCst));
    let content = std::fs::read_to_string(&path).expect("panic JSON");
    let records: Vec<_> = content
        .lines()
        .filter(|line| line.contains("concord.process.panic"))
        .collect();
    assert_eq!(records.len(), 2);
    for record in records {
        assert!(record.contains("\"error.code\":\"concord.process.panic\""));
        assert!(record.contains("\"error.class\":\"unknown\""));
        let source = record
            .split_once("\"error.source\":\"")
            .and_then(|(_, value)| value.split_once('"').map(|(value, _)| value))
            .expect("panic source");
        assert!(source.ends_with("panic_hook.rs"));
        assert!(record.contains("\"error.fingerprint\":"));
    }
    assert!(!content.contains("boom"));

    let child = std::process::Command::new(std::env::current_exe().expect("test executable"))
        .arg("--exact")
        .arg("panic_is_recorded_without_display_text_and_chains_hook")
        .arg("--nocapture")
        .env("CONCORD_PANIC_CHILD", "1")
        .output()
        .expect("run panic child");
    assert!(child.status.success());
    assert!(String::from_utf8_lossy(&child.stderr).contains("concord prior panic hook marker"));
    let _ = std::fs::remove_file(path);
}
