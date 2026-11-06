// Its own binary: it calls `init`, which sets the global subscriber. The
// parent re-runs this test as a child and reads the child's stderr.
use concord_telemetry::{init, Config, LocalOutput};
use std::io::{Read, Write};
use std::net::TcpListener;

const CHILD: &str = "CONCORD_TEXT_CHILD";

fn sink() -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    std::thread::spawn(move || {
        for incoming in listener.incoming() {
            let Ok(mut stream) = incoming else { break };
            let mut buffer = [0_u8; 65536];
            let _ = stream.read(&mut buffer);
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
        }
    });
    address
}

#[test]
fn text_line_ends_with_the_first_eight_hex_of_the_trace_id() {
    if std::env::var_os(CHILD).is_some() {
        let mut config = Config::new("text-child", "0");
        config.local = LocalOutput::Text;
        let guard = init(config).expect("child init");
        tracing::info!("outside");
        tracing::info_span!("work").in_scope(|| tracing::info!("inside"));
        drop(guard);
        return;
    }

    let address = sink();
    let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
        .arg("--exact")
        .arg("text_line_ends_with_the_first_eight_hex_of_the_trace_id")
        .arg("--nocapture")
        .env(CHILD, "1")
        .env("OTEL_EXPORTER_OTLP_ENDPOINT", format!("http://{address}"))
        .output()
        .expect("run child test");
    assert!(output.status.success(), "child failed: {:?}", output.status);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let inside = stderr
        .lines()
        .find(|line| line.contains("inside"))
        .unwrap_or_else(|| panic!("no inside line: {stderr}"));
    let (_, suffix) = inside
        .rsplit_once(" trace=")
        .unwrap_or_else(|| panic!("no trace suffix: {inside}"));
    assert!(
        suffix.len() == 8 && suffix.bytes().all(|b| b.is_ascii_hexdigit()),
        "{inside}"
    );
    let outside = stderr
        .lines()
        .find(|line| line.contains("outside"))
        .unwrap_or_else(|| panic!("no outside line: {stderr}"));
    assert!(!outside.contains("trace="), "{outside}");
}
