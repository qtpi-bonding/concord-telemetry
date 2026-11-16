//! `lifecycle` feature: the heartbeat ticks on its period and stops on drop.
#![cfg(feature = "lifecycle")]
mod lifecycle_support;
use concord_telemetry::lifecycle;
use lifecycle_support::*;
use std::time::Duration;

async fn settle() {
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
}

#[tokio::test(start_paused = true)]
async fn alive_ticks_then_stops_on_drop() {
    let (_guard, path) = init_json("alive", "0");
    let alive = lifecycle::spawn_alive(Duration::from_secs(10)).expect("alive");
    settle().await;
    assert_eq!(
        events(&path, ALIVE).len(),
        0,
        "no tick before the first period"
    );
    tokio::time::advance(Duration::from_secs(35)).await;
    settle().await;
    let ticks = events(&path, ALIVE);
    assert_eq!(ticks.len(), 3);
    assert_eq!(ticks[0]["service"], "alive");
    assert!(ticks[0]["uptime_ms"].as_i64().expect("uptime") >= 0);
    drop(alive);
    settle().await;
    tokio::time::advance(Duration::from_secs(60)).await;
    settle().await;
    assert_eq!(events(&path, ALIVE).len(), 3);
}
