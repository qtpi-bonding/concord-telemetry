// Its own binary: it sets TRACEPARENT, which other tests' threads would race.
use concord_telemetry::propagation::{set_parent_from_env, TRACEPARENT_ENV};
use concord_telemetry::testing::capture;
use concord_telemetry::EVENT_TARGET;

#[test]
fn a_span_joins_the_trace_in_the_environment() {
    std::env::set_var(
        TRACEPARENT_ENV,
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
    );
    let captured = capture();
    let span = tracing::info_span!("from-env");
    set_parent_from_env(&span);
    span.in_scope(|| {
        tracing::event!(name: "propagation-env", target: EVENT_TARGET, tracing::Level::INFO, marker = "env");
    });
    let actual = captured
        .trace_id_for("propagation-env")
        .expect("event has a trace context");
    let hex: String = actual.iter().map(|byte| format!("{byte:02x}")).collect();
    assert_eq!(hex, "4bf92f3577b34da6a3ce929d0e0e4736");
}

#[test]
fn env_round_trip_carries_baggage() {
    std::env::set_var(
        TRACEPARENT_ENV,
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
    );
    std::env::set_var(
        concord_telemetry::propagation::BAGGAGE_ENV,
        "action_id=A1,turn_id=t9",
    );
    let _captured = capture();
    let span = tracing::info_span!("from-env-baggage");
    set_parent_from_env(&span);
    let carried = span.in_scope(concord_telemetry::carrier::capture);
    assert_eq!(carried.baggage.as_deref(), Some("action_id=A1,turn_id=t9"));
    assert!(carried
        .traceparent
        .unwrap()
        .contains("4bf92f3577b34da6a3ce929d0e0e4736"));
}
