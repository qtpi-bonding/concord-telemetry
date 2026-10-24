use concord_telemetry::{init, Config, LocalOutput};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

fn drain_request(mut stream: std::net::TcpStream) -> std::io::Result<()> {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    let header_end = loop {
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            return Ok(());
        }
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(position) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            break position;
        }
    };
    let headers = String::from_utf8_lossy(&bytes[..header_end]);
    let content_length = headers
        .lines()
        .find_map(|line| {
            line.split_once(':')
                .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
        })
        .and_then(|(_, value)| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let body_start = header_end + 4;
    while bytes.len() - body_start < content_length {
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    Ok(())
}

#[test]
fn exporter_does_not_export_its_own_feedback() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = count.clone();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut response_stream = stream.try_clone().expect("clone request stream");
            let _ = drain_request(stream);
            observed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let _ = response_stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
        }
    });
    std::env::set_var("RUST_LOG", "trace");
    std::env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", format!("http://{address}"));
    let mut config = Config::new("feedback", "0");
    config.local = LocalOutput::Off;
    let guard = init(config).expect("init");
    let span = tracing::info_span!("feedback-span");
    let _entered = span.enter();
    tracing::info!("one record");
    let first_deadline = Instant::now() + Duration::from_secs(3);
    while count.load(std::sync::atomic::Ordering::Relaxed) == 0 {
        assert!(
            Instant::now() < first_deadline,
            "exporter did not send the initial request"
        );
        thread::sleep(Duration::from_millis(20));
    }
    thread::sleep(Duration::from_secs(3));
    let first = count.load(std::sync::atomic::Ordering::Relaxed);
    thread::sleep(Duration::from_secs(3));
    let second = count.load(std::sync::atomic::Ordering::Relaxed);
    assert_eq!(first, second);
    drop(guard);
}
