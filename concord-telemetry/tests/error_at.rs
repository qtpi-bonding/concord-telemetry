//! Own binary: `error_at!` records an error with an explicit code location.

use concord_telemetry::{AttrValue, Attrs};

#[derive(Debug, thiserror::Error, coded_error::CodedError)]
#[coded(namespace = "test.error_at")]
enum AtError {
    #[error("at failure")]
    #[coded(code = "failed", class = Logical)]
    Failed,
}

fn str_attr<'a>(attrs: &'a Attrs, key: &str) -> Option<&'a str> {
    match attrs.get(key) {
        Some(AttrValue::Str(text)) => Some(text),
        _ => None,
    }
}

#[test]
fn error_at_records_the_given_location_not_the_call_site() {
    let captured = concord_telemetry::testing::capture_exclusive();
    concord_telemetry::error_at!(
        "elsewhere/src/lib.rs",
        42,
        "elsewhere::inner",
        &AtError::Failed
    );
    let errors = captured.attrs_for("error");
    assert_eq!(errors.len(), 1);
    let record = &errors[0];
    assert_eq!(str_attr(record, "error.code"), Some("test.error_at.failed"));
    assert_eq!(
        str_attr(record, "code.file.path"),
        Some("elsewhere/src/lib.rs")
    );
    assert_eq!(record.get("code.line.number"), Some(&AttrValue::I64(42)));
    assert_eq!(
        str_attr(record, "code.module.name"),
        Some("elsewhere::inner")
    );
    // With no `source =`, the fingerprint source is the given module.
    assert_eq!(str_attr(record, "error.source"), Some("elsewhere::inner"));
}

#[test]
fn error_at_takes_source_and_fields_like_error() {
    let captured = concord_telemetry::testing::capture_exclusive();
    concord_telemetry::error_at!(
        "a.rs",
        1,
        "m",
        &AtError::Failed,
        source = "custom.source",
        runtime_session_id = "rs-1"
    );
    let errors = captured.attrs_for("error");
    assert_eq!(errors.len(), 1);
    assert_eq!(str_attr(&errors[0], "error.source"), Some("custom.source"));
    assert_eq!(str_attr(&errors[0], "runtime_session_id"), Some("rs-1"));
    assert_eq!(str_attr(&errors[0], "code.module.name"), Some("m"));
}

#[test]
fn error_at_anyhow_variant_exists() {
    let captured = concord_telemetry::testing::capture_exclusive();
    let err = anyhow::Error::new(AtError::Failed);
    concord_telemetry::error_at_anyhow!("a.rs", 7, "m", &err);
    assert_eq!(captured.attrs_for("error").len(), 1);
}
