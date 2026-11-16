//! SIGTERM to the process emits `ServiceStopping` once and returns.
#![cfg(all(feature = "lifecycle", unix))]
mod lifecycle_support;
use concord_telemetry::lifecycle;
use lifecycle_support::*;

#[tokio::test]
async fn shutdown_signal_emits_stopping_once() {
    let (_guard, path) = init_json("signal", "0");
    let waiter = tokio::spawn(lifecycle::shutdown_signal());
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    // SAFETY: raising a signal the runtime has a handler for.
    assert_eq!(unsafe { libc::raise(libc::SIGTERM) }, 0);
    tokio::time::timeout(std::time::Duration::from_secs(5), waiter)
        .await
        .expect("returns on SIGTERM")
        .expect("join");
    let stopping = events(&path, STOPPING);
    assert_eq!(stopping.len(), 1);
    assert_eq!(stopping[0]["reason"], "sigterm");
    assert_eq!(stopping[0]["service"], "signal");
}
