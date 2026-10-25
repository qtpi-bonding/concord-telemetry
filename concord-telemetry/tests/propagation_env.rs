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
