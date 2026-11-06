use concord_telemetry::{error_text, set_error_text, Config, ErrorText, LocalOutput};

// init can run only once per test binary, so this test has its own binary.
#[test]
fn unknown_concord_error_text_warns_once_and_means_full() {
    std::env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");
    std::env::set_var("CONCORD_ERROR_TEXT", "junk");
    set_error_text(ErrorText::Own);
    let path = std::env::temp_dir().join(format!("concord-error-text-{}.log", std::process::id()));
    let mut config = Config::new("error-text", "0");
    config.local = LocalOutput::JsonFile(path.clone());
    let guard = concord_telemetry::init(config).expect("init");
    assert_eq!(error_text(), ErrorText::Full);
    drop(guard);
    let content = std::fs::read_to_string(&path).expect("read json log");
    let _ = std::fs::remove_file(&path);
    let warnings = content
        .lines()
        .filter(|line| line.contains("\"WARN\"") && line.contains("CONCORD_ERROR_TEXT"))
        .collect::<Vec<_>>();
    assert_eq!(warnings.len(), 1, "log was: {content}");
    assert!(warnings[0].contains("junk"));
}
