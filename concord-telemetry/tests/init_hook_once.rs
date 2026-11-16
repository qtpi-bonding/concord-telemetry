//! Own binary: failed `init` calls leave the panic hook alone, and the one
//! successful `init` chains the previous hook exactly once. Runs in the `otel`
//! build and the off build.
use concord_telemetry::{init, Config, InitError, LocalOutput};
use std::panic;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[test]
fn retried_init_installs_one_hook() {
    std::env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");
    let probe = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&probe);
    panic::set_hook(Box::new(move |_| {
        seen.fetch_add(1, Ordering::SeqCst);
    }));

    for _ in 0..3 {
        let mut config = Config::new("hook-once", "0");
        config.local = LocalOutput::JsonFile("/nonexistent-concord-dir/hook.json".into());
        match init(config) {
            Err(InitError::JsonFile(_)) => {}
            Err(other) => panic!("unexpected initialization error: {other}"),
            Ok(_) => panic!("init with an unwritable log path unexpectedly succeeded"),
        }
    }
    let mut config = Config::new("hook-once", "0");
    config.local = LocalOutput::Off;
    let guard = init(config).expect("good init after failed ones");

    for round in 1..=2 {
        let result = std::thread::spawn(|| panic::panic_any("probe panic")).join();
        assert!(result.is_err());
        assert_eq!(
            probe.load(Ordering::SeqCst),
            round,
            "the previous hook runs once per panic"
        );
    }
    drop(guard);
}
