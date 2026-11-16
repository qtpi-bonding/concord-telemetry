//! `testing::{check_attributes, spans_checked, trace_env, with_trace_env,
//! label_names}` and `clip`.
#![cfg(feature = "testing")]

use concord_telemetry::propagation::{BAGGAGE_ENV, TRACEPARENT_ENV};
use concord_telemetry::testing::{self, check_attributes, Registry};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
    items
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

#[test]
#[should_panic(expected = "duplicate")]
fn duplicate_key_panics() {
    check_attributes(
        &Registry::shared(),
        "my-span",
        &pairs(&[("outcome", "a"), ("outcome", "b")]),
    );
}

#[test]
#[should_panic(expected = "not_a_field")]
fn unregistered_key_panics() {
    check_attributes(
        &Registry::shared(),
        "my-span",
        &pairs(&[("not_a_field", "x")]),
    );
}

#[test]
fn layer_keys_and_error_keys_pass() {
    check_attributes(
        &Registry::shared(),
        "my-span",
        &pairs(&[
            ("target", "t"),
            ("level", "INFO"),
            ("busy_ns", "1"),
            ("idle_ns", "2"),
            ("otel.kind", "client"),
            ("code.file.path", "a.rs"),
            ("thread.id", "1"),
            ("error.code", "x"),
            ("error.class", "Logical"),
            ("error.fingerprint", "f"),
            ("error.source", "s"),
            ("ingress", "http"),
        ]),
    );
}

#[test]
#[should_panic(expected = "free_text key \"command\"")]
fn free_text_over_cap_panics() {
    let long = "x".repeat(concord_fields_cap() + 1);
    check_attributes(
        &Registry::shared(),
        "my-span",
        &pairs(&[("command", &long)]),
    );
}

#[test]
fn free_text_at_cap_passes() {
    let at = "x".repeat(concord_fields_cap());
    check_attributes(&Registry::shared(), "my-span", &pairs(&[("command", &at)]));
}

fn concord_fields_cap() -> usize {
    1024
}

#[test]
fn spans_checked_filters_by_prefix() {
    let _c = testing::capture_exclusive();
    drop(tracing::info_span!("th-keep-1", outcome = "ok"));
    drop(tracing::info_span!("th-keep-2"));
    drop(tracing::info_span!("th-other"));
    let spans = testing::spans_checked(&Registry::shared(), "th-keep");
    let names: Vec<_> = spans.iter().map(|s| s.name.to_string()).collect();
    assert_eq!(names, ["th-keep-1", "th-keep-2"]);
}

#[test]
fn label_names_match_registry() {
    let names = testing::label_names();
    assert!(!names.is_empty());
    assert_eq!(names, Registry::shared().label_names());
}

/// Sets a pre-existing traceparent once, before any test takes the lock, so
/// restoration of a present value (traceparent) and of an absent one
/// (baggage, removed here) are both observable.
/// The three env tests read the variables outside `trace_env`'s own lock, so
/// they run one at a time; the guard this returns is held for the whole test.
fn init_env() -> std::sync::MutexGuard<'static, ()> {
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    init_env_once();
    serial
}

fn init_env_once() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        std::env::set_var(TRACEPARENT_ENV, "tp-prev");
        std::env::remove_var(BAGGAGE_ENV);
    });
}

#[test]
fn trace_env_sets_and_restores() {
    let _serial = init_env();
    {
        let _g = testing::trace_env(Some("tp-1"), Some("k=v"));
        assert_eq!(std::env::var(TRACEPARENT_ENV).as_deref(), Ok("tp-1"));
        assert_eq!(std::env::var(BAGGAGE_ENV).as_deref(), Ok("k=v"));
    }
    assert_eq!(std::env::var(TRACEPARENT_ENV).as_deref(), Ok("tp-prev"));
    assert!(std::env::var_os(BAGGAGE_ENV).is_none());

    // None removes the variable for the scope, and it comes back after.
    let seen = testing::with_trace_env(None, Some("k=v"), || {
        (
            std::env::var_os(TRACEPARENT_ENV),
            std::env::var(BAGGAGE_ENV).ok(),
        )
    });
    assert_eq!(seen, (None, Some("k=v".to_owned())));
    assert_eq!(std::env::var(TRACEPARENT_ENV).as_deref(), Ok("tp-prev"));
    assert!(std::env::var_os(BAGGAGE_ENV).is_none());
}

#[test]
fn trace_env_restores_on_panic() {
    let _serial = init_env();
    let result = std::panic::catch_unwind(|| {
        testing::with_trace_env(Some("tp-panic"), Some("x=y"), || {
            assert_eq!(std::env::var(TRACEPARENT_ENV).as_deref(), Ok("tp-panic"));
            panic!("boom");
        })
    });
    assert!(result.is_err());
    // The lock is usable again (not stuck, poison ignored) and env is restored.
    let _g = testing::trace_env(Some("tp-after"), None);
    drop(_g);
    assert_eq!(std::env::var(TRACEPARENT_ENV).as_deref(), Ok("tp-prev"));
    assert!(std::env::var_os(BAGGAGE_ENV).is_none());
}

#[test]
fn trace_env_serialises() {
    let _serial = init_env();
    let guard = testing::trace_env(Some("tp-holder"), None);
    let got_it = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    let flag = got_it.clone();
    let other = std::thread::spawn(move || {
        tx.send(()).unwrap();
        let g = testing::trace_env(Some("tp-second"), None);
        flag.store(true, Ordering::SeqCst);
        drop(g);
    });
    rx.recv().unwrap();
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        !got_it.load(Ordering::SeqCst),
        "a second thread held the trace env lock together with the first"
    );
    assert_eq!(std::env::var(TRACEPARENT_ENV).as_deref(), Ok("tp-holder"));
    drop(guard);
    other.join().unwrap();
    assert!(got_it.load(Ordering::SeqCst));
}

#[test]
fn clip_is_char_boundary_safe() {
    assert_eq!(concord_telemetry::clip("short"), "short");
    let exact = "a".repeat(1024);
    assert_eq!(concord_telemetry::clip(&exact), exact);
    // 1023 ASCII bytes then a 2-byte char straddling the cap.
    let s = format!("{}é{}", "a".repeat(1023), "tail");
    let clipped = concord_telemetry::clip(&s);
    assert_eq!(clipped, "a".repeat(1023));
    assert!(clipped.len() <= 1024);
}
