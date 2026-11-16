//! One `init` (JSON file, both builds): the lifecycle events' fields.
mod lifecycle_support;
use concord_telemetry::lifecycle;
use lifecycle_support::*;
use std::path::PathBuf;
use std::sync::OnceLock;

static LOG: OnceLock<PathBuf> = OnceLock::new();

fn log() -> &'static PathBuf {
    LOG.get_or_init(|| {
        let (guard, path) = init_json("svc", "1.2.3");
        std::mem::forget(guard);
        path
    })
}

#[test]
fn started_carries_service_version_and_instance() {
    let path = log();
    lifecycle::started().expect("started");
    let started = events(path, STARTED);
    // Other tests in this binary also call `started`.
    assert!(!started.is_empty());
    assert_eq!(started[0]["service"], "svc");
    assert_eq!(started[0]["version"], "1.2.3");
    let id = started[0]["source_record_id"].as_str().expect("id");
    assert_eq!(id.len(), 16);
    assert!(id.bytes().all(|b| b.is_ascii_hexdigit()), "{id}");
}

#[test]
fn stopping_carries_reason_and_uptime() {
    let path = log();
    lifecycle::stopping(Some("because")).expect("stopping");
    lifecycle::stopping(None).expect("stopping");
    // The only caller that passes `None` is this test.
    let stopping = events(path, STOPPING);
    let with: Vec<_> = stopping
        .iter()
        .filter(|e| e["reason"] == "because")
        .collect();
    let without: Vec<_> = stopping
        .iter()
        .filter(|e| e.get("reason").is_none())
        .collect();
    assert_eq!(with.len(), 1);
    assert_eq!(without.len(), 1);
    assert_eq!(with[0]["service"], "svc");
    assert!(with[0]["uptime_ms"].as_i64().expect("uptime") >= 0);
    assert_eq!(with[0]["source_record_id"], without[0]["source_record_id"]);
}

#[test]
fn every_lifecycle_event_shares_one_instance_id() {
    let path = log();
    lifecycle::started().expect("started");
    lifecycle::stopping(Some("shared")).expect("stopping");
    let mut ids: Vec<_> = events(path, STARTED)
        .into_iter()
        .chain(events(path, STOPPING))
        .map(|e| e["source_record_id"].as_str().expect("id").to_owned())
        .collect();
    ids.dedup();
    assert_eq!(ids.len(), 1, "{ids:?}");
}

/// Off build: the instance id and events exist without any OpenTelemetry.
#[cfg(not(feature = "otel"))]
#[test]
fn works_with_otel_off() {
    let path = log();
    lifecycle::started().expect("started");
    assert!(!events(path, STARTED).is_empty());
}
