use concord_telemetry::{error_text, ErrorText};

// init can run only once per test binary, so this test has its own binary.
#[test]
fn concord_error_text_env_sets_initial_mode() {
    std::env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");
    std::env::set_var("CONCORD_ERROR_TEXT", "own");
    assert_eq!(error_text(), ErrorText::Full);
    let _guard = concord_telemetry::init(concord_telemetry::Config::new("test", "0"))
        .expect("init should install the subscriber");
    assert_eq!(error_text(), ErrorText::Own);
}
