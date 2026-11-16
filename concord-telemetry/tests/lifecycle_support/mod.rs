//! Shared helpers for the lifecycle test binaries. Each binary initialises
//! telemetry once, writing JSON lines to a file, and reads the events back.
#![allow(dead_code)]

use concord_telemetry::{init, Config, Guard, LocalOutput};
use serde_json::Value;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub type Requests = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

pub const STARTED: &str = "concord_telemetry.v1.ServiceStarted";
pub const STOPPING: &str = "concord_telemetry.v1.ServiceStopping";
pub const ALIVE: &str = "concord_telemetry.v1.ServiceAlive";

/// Initialises with `JsonFile` output and no exporter; returns the guard and
/// the log path.
pub fn init_json(service: &'static str, version: &'static str) -> (Guard, PathBuf) {
    std::env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");
    let path = std::env::temp_dir().join(format!(
        "concord-lifecycle-{service}-{}.log",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let mut config = Config::new(service, version);
    config.local = LocalOutput::JsonFile(path.clone());
    (init(config).expect("init"), path)
}

/// The `fields` object of every logged event with `event.type == event_type`.
pub fn events(path: &PathBuf, event_type: &str) -> Vec<Value> {
    std::fs::read_to_string(path)
        .expect("read log file")
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .map(|line| line["fields"].clone())
        .filter(|fields| fields["event.type"] == event_type)
        .collect()
}

/// A listener that records every HTTP request body by path.
pub fn mock_otlp() -> (String, Requests) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let requests = Requests::default();
    let received = Arc::clone(&requests);
    std::thread::spawn(move || {
        for incoming in listener.incoming() {
            let Ok(mut stream) = incoming else { break };
            let mut bytes = Vec::new();
            let mut buffer = [0_u8; 4096];
            let (header_end, length) = loop {
                let Ok(read) = stream.read(&mut buffer) else {
                    break (0, 0);
                };
                if read == 0 {
                    break (0, 0);
                }
                bytes.extend_from_slice(&buffer[..read]);
                if let Some(at) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&bytes[..at]).to_ascii_lowercase();
                    let length = head
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .and_then(|v| v.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    break (at + 4, length);
                }
            };
            if header_end == 0 {
                continue;
            }
            while bytes.len() - header_end < length {
                match stream.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => bytes.extend_from_slice(&buffer[..n]),
                }
            }
            let head = String::from_utf8_lossy(&bytes[..header_end]).into_owned();
            let path = head.split_whitespace().nth(1).unwrap_or("").to_owned();
            received
                .lock()
                .unwrap()
                .push((path, bytes[header_end..].to_vec()));
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
        }
    });
    (format!("http://{address}"), requests)
}
