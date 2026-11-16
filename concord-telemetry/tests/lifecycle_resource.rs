//! `otel` on, exporting: `source_record_id` is the Resource's `service.instance.id`.
#![cfg(feature = "otel")]
mod lifecycle_support;
use concord_telemetry::{init, lifecycle, Config, LocalOutput};
use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
use opentelemetry_proto::tonic::common::v1::any_value::Value;
use prost14::Message;
use std::time::{Duration, Instant};

fn string(attrs: &[opentelemetry_proto::tonic::common::v1::KeyValue], key: &str) -> Option<String> {
    attrs.iter().find(|kv| kv.key == key).and_then(|kv| {
        match kv.value.as_ref()?.value.as_ref()? {
            Value::StringValue(s) => Some(s.clone()),
            _ => None,
        }
    })
}

#[test]
fn started_carries_service_version_and_instance() {
    let (endpoint, requests) = lifecycle_support::mock_otlp();
    std::env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", endpoint);
    let mut config = Config::new("svc", "1.2.3");
    config.local = LocalOutput::Off;
    let guard = init(config).expect("init");
    lifecycle::started().expect("started");
    drop(guard);

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if requests
            .lock()
            .unwrap()
            .iter()
            .any(|(p, _)| p == "/v1/logs")
        {
            break;
        }
        assert!(Instant::now() < deadline, "no logs exported");
        std::thread::sleep(Duration::from_millis(20));
    }
    let body = requests
        .lock()
        .unwrap()
        .iter()
        .find(|(p, _)| p == "/v1/logs")
        .unwrap()
        .1
        .clone();
    let logs = ExportLogsServiceRequest::decode(body.as_slice()).expect("decode");
    let resource_logs = &logs.resource_logs[0];
    let resource = &resource_logs
        .resource
        .as_ref()
        .expect("resource")
        .attributes;
    let instance = string(resource, "service.instance.id").expect("instance id");
    let record = resource_logs
        .scope_logs
        .iter()
        .flat_map(|s| s.log_records.iter())
        .find(|r| r.event_name == "concord_telemetry.v1.ServiceStarted")
        .expect("started record");
    assert_eq!(
        string(&record.attributes, "source_record_id"),
        Some(instance)
    );
    assert_eq!(
        string(&record.attributes, "service").as_deref(),
        Some("svc")
    );
    assert_eq!(
        string(&record.attributes, "version").as_deref(),
        Some("1.2.3")
    );
}
