// Its own binary: it calls `init`, which sets the global subscriber.
use concord_telemetry::{context, init, Config, LocalOutput};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

fn sink() -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    thread::spawn(move || {
        for incoming in listener.incoming() {
            let Ok(mut stream) = incoming else { break };
            let mut buffer = [0_u8; 65536];
            let _ = stream.read(&mut buffer);
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
        }
    });
    address
}

fn is_hex(value: &str, len: usize) -> bool {
    value.len() == len && value.bytes().all(|b| b.is_ascii_hexdigit())
}

#[test]
fn json_line_has_trace_id_span_id_and_baggage() {
    let address = sink();
    std::env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", format!("http://{address}"));
    let path = std::env::temp_dir().join(format!("concord-trace-json-{}.log", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let mut config = Config::new("json-trace", "0");
    config.local = LocalOutput::JsonFile(path.clone());
    let guard = init(config).expect("init");

    tracing::info!("outside");
    let span = tracing::info_span!("work");
    span.in_scope(|| {
        context::add("action_id", "A1");
        tracing::info!(marker = "m", "inside");
    });
    drop(span);
    drop(guard);

    let content = std::fs::read_to_string(&path).expect("read json");
    let line = content
        .lines()
        .find(|line| line.contains("\"inside\""))
        .expect("the inside line");
    let value: serde_json::Value = serde_json::from_str(line).expect("valid json");
    let trace_id = value["trace_id"].as_str().expect("trace_id");
    let span_id = value["span_id"].as_str().expect("span_id");
    assert!(is_hex(trace_id, 32), "{line}");
    assert!(is_hex(span_id, 16), "{line}");
    assert_eq!(value["baggage"]["action_id"], "A1", "{line}");
    assert_eq!(value["fields"]["marker"], "m", "{line}");
    assert!(
        line.contains("\"fields\":{\"message\":\"inside\",\"marker\":\"m\"}"),
        "field order is kept: {line}"
    );

    let outside = content
        .lines()
        .find(|line| line.contains("\"outside\""))
        .expect("the outside line");
    let outside: serde_json::Value = serde_json::from_str(outside).expect("valid json");
    assert!(outside.get("trace_id").is_none(), "{outside}");
    assert!(outside.get("baggage").is_none(), "{outside}");
    let _ = std::fs::remove_file(path);
}
