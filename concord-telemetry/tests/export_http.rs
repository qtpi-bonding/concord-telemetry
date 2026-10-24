use concord_telemetry::{init, Config, LocalOutput, Principal, PrincipalKind, EVENT_TARGET};
use opentelemetry_proto::tonic::collector::{
    logs::v1::ExportLogsServiceRequest, trace::v1::ExportTraceServiceRequest,
};
use prost14::Message;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone)]
struct Request {
    path: String,
    headers: String,
    body: Vec<u8>,
}

fn read_request(mut stream: std::net::TcpStream) -> Option<Request> {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    let header_end;
    loop {
        let read = stream.read(&mut buffer).ok()?;
        if read == 0 {
            return None;
        }
        bytes.extend_from_slice(&buffer[..read]);
        if let Some(position) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            header_end = position;
            break;
        }
    }
    let header_text = String::from_utf8_lossy(&bytes[..header_end]).to_string();
    let content_length = header_text
        .lines()
        .find_map(|line| {
            line.split_once(':')
                .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
        })
        .and_then(|(_, value)| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let body_start = header_end + 4;
    while bytes.len() - body_start < content_length {
        let read = stream.read(&mut buffer).ok()?;
        if read == 0 {
            return None;
        }
        bytes.extend_from_slice(&buffer[..read]);
    }
    let path = header_text.split_whitespace().nth(1)?.to_owned();
    Some(Request {
        path,
        headers: header_text,
        body: bytes[body_start..body_start + content_length].to_vec(),
    })
}

#[test]
fn exports_logs_and_traces_over_http() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let requests: Arc<Mutex<Vec<Request>>> = Arc::new(Mutex::new(Vec::new()));
    let received = Arc::clone(&requests);
    thread::spawn(move || {
        for incoming in listener.incoming() {
            let mut stream = match incoming {
                Ok(stream) => stream,
                Err(_) => break,
            };
            if let Some(request) = read_request(stream.try_clone().expect("clone stream")) {
                received.lock().expect("request lock").push(request);
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
            }
        }
    });

    std::env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", format!("http://{address}"));
    let mut config = Config::new("http-test", "1");
    config.local = LocalOutput::Off;
    config.principal = Some(Principal::new(PrincipalKind::Agent, "agent-1"));
    let guard = init(config).expect("init");
    tracing::info!(target: "http-test", "log record");
    tracing::event!(name: "test.events.v1.X", target: EVENT_TARGET, tracing::Level::INFO, marker = "event-value", asserted_field = "asserted-value");
    let span = tracing::info_span!("exported-span");
    let entered = span.enter();
    tracing::info!("inside exported span");
    drop(entered);
    drop(span);
    drop(guard);

    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let has_logs = requests
            .lock()
            .expect("request lock")
            .iter()
            .any(|request| request.path == "/v1/logs");
        let has_traces = requests
            .lock()
            .expect("request lock")
            .iter()
            .any(|request| request.path == "/v1/traces");
        if has_logs && has_traces {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "did not receive both OTLP requests"
        );
        thread::sleep(Duration::from_millis(20));
    }

    let requests = requests.lock().expect("request lock");
    let logs_request = requests
        .iter()
        .find(|request| request.path == "/v1/logs")
        .expect("logs request");
    let traces_request = requests
        .iter()
        .find(|request| request.path == "/v1/traces")
        .expect("traces request");
    assert!(logs_request
        .headers
        .to_ascii_lowercase()
        .contains("content-type: application/x-protobuf"));
    assert!(traces_request
        .headers
        .to_ascii_lowercase()
        .contains("content-type: application/x-protobuf"));

    let logs = ExportLogsServiceRequest::decode(logs_request.body.as_slice()).expect("decode logs");
    let log = logs
        .resource_logs
        .iter()
        .flat_map(|resource| resource.scope_logs.iter())
        .flat_map(|scope| scope.log_records.iter())
        .find(|record| record.event_name == "test.events.v1.X")
        .expect("event log record");
    let plain = logs
        .resource_logs
        .iter()
        .flat_map(|resource| resource.scope_logs.iter())
        .flat_map(|scope| scope.log_records.iter())
        .find(|record| {
            matches!(
                record.body.as_ref().and_then(|body| body.value.as_ref()),
                Some(opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(value)) if value == "log record"
            )
        })
        .expect("plain info log record");
    assert!(matches!(
        plain.body.as_ref().and_then(|body| body.value.as_ref()),
        Some(opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(value)) if value == "log record"
    ));
    let marker = log
        .attributes
        .iter()
        .find(|attribute| attribute.key == "marker")
        .expect("event marker attribute");
    assert!(
        matches!(marker.value.as_ref().and_then(|value| value.value.as_ref()), Some(opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(value)) if value == "event-value")
    );
    let asserted_field = log
        .attributes
        .iter()
        .find(|attribute| attribute.key == "asserted_field")
        .expect("event asserted field attribute");
    assert!(matches!(
        asserted_field.value.as_ref().and_then(|value| value.value.as_ref()),
        Some(opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(value)) if value == "asserted-value"
    ));
    let resource = logs
        .resource_logs
        .iter()
        .find_map(|resource| resource.resource.as_ref())
        .expect("log resource");
    let resource_value = |key: &str| {
        resource
            .attributes
            .iter()
            .find(|attribute| attribute.key == key)
            .and_then(|attribute| attribute.value.as_ref())
            .and_then(|value| value.value.as_ref())
    };
    assert!(
        matches!(resource_value("service.name"), Some(opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(value)) if value == "http-test")
    );
    assert!(
        matches!(resource_value("service.version"), Some(opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(value)) if value == "1")
    );
    assert!(
        matches!(resource_value("service.instance.id"), Some(opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(value)) if !value.is_empty())
    );
    assert!(
        matches!(resource_value("principal.kind"), Some(opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(value)) if value == "PRINCIPAL_KIND_AGENT")
    );
    assert!(
        matches!(resource_value("principal.id"), Some(opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(value)) if value == "agent-1")
    );
    assert!(
        matches!(resource_value("principal.verified_by"), Some(opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(value)) if value == "[]")
    );

    let traces =
        ExportTraceServiceRequest::decode(traces_request.body.as_slice()).expect("decode traces");
    assert!(traces
        .resource_spans
        .iter()
        .flat_map(|resource| resource.scope_spans.iter())
        .flat_map(|scope| scope.spans.iter())
        .any(|span| span.name == "exported-span"));
}
