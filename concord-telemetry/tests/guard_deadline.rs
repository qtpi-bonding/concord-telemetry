use concord_telemetry::{init, Config, LocalOutput};
use std::io::Read;
use std::net::TcpListener;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::{Duration, Instant};

#[test]
fn guard_flush_is_bounded_for_a_hung_sink() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let accepted = Arc::new(AtomicBool::new(false));
    let accepted_flag = Arc::clone(&accepted);
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = match stream {
                Ok(stream) => stream,
                Err(_) => break,
            };
            accepted_flag.store(true, Ordering::SeqCst);
            let mut byte = [0_u8; 1];
            let _ = stream.read(&mut byte);
            thread::sleep(Duration::from_secs(10));
        }
    });
    std::env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", format!("http://{address}"));
    let mut config = Config::new("deadline", "0");
    config.local = LocalOutput::Off;
    config.flush_deadline = Duration::from_secs(2);
    let guard = init(config).expect("init");
    let emitting = Instant::now();
    for index in 0..100 {
        tracing::info!(index, "record");
    }
    assert!(emitting.elapsed() < Duration::from_secs(2));
    let dropping = Instant::now();
    drop(guard);
    assert!(accepted.load(Ordering::SeqCst));
    assert!(dropping.elapsed() >= Duration::from_secs_f32(1.5));
    assert!(dropping.elapsed() < Duration::from_secs(3));
}
